//! Almacenamiento provisional del spike (Fase 1): un directorio por versión.
//!
//! Se reemplaza en la Fase 2 por SQLite y blobs direccionados por hash (ADR-004, ADR-005,
//! ADR-006). Ya respeta lo esencial: el `.nupkg` se guarda byte a byte, se escribe en staging
//! con `fsync` antes de hacerse visible y una identidad existente no se sobrescribe (ADR-007).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use onepack_nuget::{NuGetVersion, PackageManifest};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub struct FeedStore {
    root: PathBuf,
    staging: PathBuf,
    publish_lock: Mutex<()>,
    staging_seq: AtomicU64,
}

#[derive(Debug)]
pub enum PublishError {
    /// La identidad `id + versión normalizada` ya existe en el feed.
    Conflict,
    Io(io::Error),
}

impl From<io::Error> for PublishError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl FeedStore {
    pub async fn open(data_dir: &Path, feed: &str) -> io::Result<Self> {
        let root = data_dir.join("feeds").join(feed);
        let staging = data_dir.join("staging");
        fs::create_dir_all(&root).await?;
        fs::create_dir_all(&staging).await?;
        Ok(Self {
            root,
            staging,
            publish_lock: Mutex::new(()),
            staging_seq: AtomicU64::new(0),
        })
    }

    /// `id_key` y `version_key` son claves de identidad ya validadas (minúsculas, sin separadores
    /// de ruta), nunca texto recibido sin parsear.
    fn nupkg_path(&self, id_key: &str, version_key: &str) -> PathBuf {
        self.root
            .join(id_key)
            .join(version_key)
            .join(format!("{id_key}.{version_key}.nupkg"))
    }

    pub async fn publish(
        &self,
        manifest: &PackageManifest,
        nupkg: &[u8],
    ) -> Result<(), PublishError> {
        let id_key = manifest.id.identity();
        let version_key = manifest.version.identity();
        let target = self.nupkg_path(&id_key, &version_key);
        let version_dir = target.parent().expect("ruta con directorio de versión");

        // Spike: publicaciones serializadas. En la Fase 2 la unicidad la garantiza la base.
        let _guard = self.publish_lock.lock().await;
        if fs::try_exists(version_dir).await? {
            return Err(PublishError::Conflict);
        }

        let seq = self.staging_seq.fetch_add(1, Ordering::Relaxed);
        let staged = self
            .staging
            .join(format!("{}-{seq}.nupkg", std::process::id()));
        let result = async {
            let mut file = fs::File::create(&staged).await?;
            file.write_all(nupkg).await?;
            file.sync_all().await?;
            fs::create_dir_all(version_dir).await?;
            fs::rename(&staged, &target).await?;
            fs::File::open(version_dir).await?.sync_all().await
        }
        .await;
        if result.is_err() {
            let _ = fs::remove_file(&staged).await;
        }
        Ok(result?)
    }

    pub async fn read_nupkg(&self, id_key: &str, version_key: &str) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.nupkg_path(id_key, version_key)).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Versiones de un paquete, como claves de identidad, en orden de precedencia NuGet.
    pub async fn versions(&self, id_key: &str) -> io::Result<Vec<String>> {
        let mut entries = match fs::read_dir(self.root.join(id_key)).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut versions = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            if let Some(v) = entry
                .file_name()
                .to_str()
                .and_then(|n| NuGetVersion::parse(n).ok())
            {
                versions.push(v);
            }
        }
        versions.sort_by(|a, b| a.precedence_cmp(b));
        Ok(versions.iter().map(NuGetVersion::identity).collect())
    }
}
