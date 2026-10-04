//! Almacén de blobs en el filesystem local, direccionado por SHA-256 (ADR-005).
//!
//! Disposición: `blobs/sha256/ab/cd/<hash>` y `staging/` para subidas en curso.
//! Los bytes se guardan exactamente como se recibieron.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

pub struct BlobStore {
    root: PathBuf,
    staging: PathBuf,
    seq: AtomicU64,
}

/// Subida en curso: escribe en staging y calcula el hash a la vez.
pub struct StagingWriter {
    file: fs::File,
    blob: StagedBlob,
    hasher: Sha256,
    max_size: u64,
}

/// Archivo completo y sincronizado en staging. Si no se persiste, se borra al soltarlo.
pub struct StagedBlob {
    path: PathBuf,
    sha256: String,
    size: u64,
    keep: bool,
}

#[derive(Debug)]
pub enum StagingError {
    TooLarge { max_size: u64 },
    Io(io::Error),
}

impl From<io::Error> for StagingError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl StagedBlob {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

impl Drop for StagedBlob {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl StagingWriter {
    pub async fn write(&mut self, chunk: &[u8]) -> Result<(), StagingError> {
        self.blob.size += chunk.len() as u64;
        if self.blob.size > self.max_size {
            return Err(StagingError::TooLarge {
                max_size: self.max_size,
            });
        }
        self.hasher.update(chunk);
        self.file.write_all(chunk).await?;
        Ok(())
    }

    /// Sincroniza el archivo a disco y devuelve el blob listo para inspeccionar y persistir.
    pub async fn finish(mut self) -> Result<StagedBlob, StagingError> {
        self.file.flush().await?;
        self.file.sync_all().await?;
        let digest = self.hasher.finalize();
        self.blob.sha256 = digest.iter().map(|b| format!("{b:02x}")).collect();
        Ok(self.blob)
    }
}

impl BlobStore {
    pub async fn open(data_dir: &Path) -> io::Result<Self> {
        let root = data_dir.join("blobs").join("sha256");
        let staging = data_dir.join("staging");
        fs::create_dir_all(&root).await?;
        fs::create_dir_all(&staging).await?;
        Ok(Self {
            root,
            staging,
            seq: AtomicU64::new(0),
        })
    }

    pub fn staging_dir(&self) -> &Path {
        &self.staging
    }

    /// Ruta de un blob. `sha256` debe venir de la base o de un hash calculado, nunca de una URL.
    pub fn path(&self, sha256: &str) -> PathBuf {
        debug_assert!(is_sha256_hex(sha256));
        self.root
            .join(&sha256[0..2])
            .join(&sha256[2..4])
            .join(sha256)
    }

    pub async fn begin_staging(&self, max_size: u64) -> io::Result<StagingWriter> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let path = self
            .staging
            .join(format!("{}-{seq}.upload", std::process::id()));
        let file = fs::File::create_new(&path).await?;
        Ok(StagingWriter {
            file,
            blob: StagedBlob {
                path,
                sha256: String::new(),
                size: 0,
                keep: false,
            },
            hasher: Sha256::new(),
            max_size,
        })
    }

    /// Mueve el blob de staging a su ubicación definitiva de forma durable (ADR-006).
    /// Si ya existe un blob con el mismo hash se reutiliza (deduplicación).
    pub async fn persist(&self, mut staged: StagedBlob) -> io::Result<()> {
        let target = self.path(&staged.sha256);
        let shard2 = target.parent().expect("blob con directorio");
        let shard1 = shard2.parent().expect("blob con directorio");

        if fs::try_exists(&target).await? {
            // Renueva la fecha para que la limpieza de huérfanos no lo borre mientras
            // se confirman los metadatos que lo van a referenciar.
            let file = fs::File::open(&target).await?.into_std().await;
            tokio::task::spawn_blocking(move || file.set_modified(SystemTime::now()))
                .await
                .map_err(io::Error::other)??;
            return Ok(());
        }

        fs::create_dir_all(shard2).await?;
        fs::rename(&staged.path, &target).await?;
        staged.keep = true;
        for dir in [shard2, shard1, self.root.as_path()] {
            sync_dir(dir).await?;
        }
        Ok(())
    }

    pub async fn open_blob(&self, sha256: &str) -> io::Result<fs::File> {
        fs::File::open(self.path(sha256)).await
    }

    /// Hashes de todos los blobs en disco con antigüedad mayor que `min_age`.
    pub async fn list_older_than(&self, min_age: Duration) -> io::Result<Vec<String>> {
        let mut found = Vec::new();
        for shard1 in read_dirs(&self.root).await? {
            for shard2 in read_dirs(&shard1).await? {
                let mut entries = fs::read_dir(&shard2).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let name = entry.file_name();
                    let Some(name) = name.to_str() else { continue };
                    if is_sha256_hex(name) && is_older_than(&entry.metadata().await?, min_age) {
                        found.push(name.to_owned());
                    }
                }
            }
        }
        Ok(found)
    }

    pub async fn remove(&self, sha256: &str) -> io::Result<()> {
        fs::remove_file(self.path(sha256)).await
    }

    /// Elimina subidas abandonadas en staging (p. ej. tras una caída del proceso).
    pub async fn clean_staging(&self, min_age: Duration) -> io::Result<usize> {
        let mut removed = 0;
        let mut entries = fs::read_dir(&self.staging).await?;
        while let Some(entry) = entries.next_entry().await? {
            if is_older_than(&entry.metadata().await?, min_age) {
                fs::remove_file(entry.path()).await?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_older_than(meta: &std::fs::Metadata, min_age: Duration) -> bool {
    meta.modified()
        .ok()
        .and_then(|m| m.elapsed().ok())
        .is_some_and(|age| age >= min_age)
}

async fn read_dirs(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    let mut entries = fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            dirs.push(entry.path());
        }
    }
    Ok(dirs)
}

/// Hace durable la entrada de directorio de un rename. En Windows no se puede abrir un
/// directorio como archivo; NTFS registra los cambios de metadatos en su journal.
#[cfg(unix)]
async fn sync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir).await?.sync_all().await
}

#[cfg(not(unix))]
async fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn stage(store: &BlobStore, content: &[u8]) -> StagedBlob {
        let mut w = store.begin_staging(1024).await.unwrap();
        for chunk in content.chunks(3) {
            w.write(chunk).await.unwrap();
        }
        w.finish().await.unwrap()
    }

    #[tokio::test]
    async fn stage_hashes_and_persist_keeps_exact_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path()).await.unwrap();
        let staged = stage(&store, b"hello").await;
        let sha = staged.sha256().to_owned();
        assert_eq!(
            sha,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(staged.size(), 5);

        store.persist(staged).await.unwrap();
        assert_eq!(std::fs::read(store.path(&sha)).unwrap(), b"hello");
        assert_eq!(std::fs::read_dir(store.staging_dir()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn duplicate_content_is_deduplicated_and_staging_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path()).await.unwrap();
        store.persist(stage(&store, b"same").await).await.unwrap();
        store.persist(stage(&store, b"same").await).await.unwrap();
        assert_eq!(
            store.list_older_than(Duration::ZERO).await.unwrap().len(),
            1
        );
        assert_eq!(std::fs::read_dir(store.staging_dir()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn oversized_upload_is_rejected_and_dropped_staging_removed() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path()).await.unwrap();
        let mut w = store.begin_staging(4).await.unwrap();
        assert!(matches!(
            w.write(b"12345").await,
            Err(StagingError::TooLarge { max_size: 4 })
        ));
        drop(w);
        assert_eq!(std::fs::read_dir(store.staging_dir()).unwrap().count(), 0);
    }
}
