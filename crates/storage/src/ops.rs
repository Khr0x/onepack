//! Operación (Fase 7, ADR-017): modo mantenimiento, backup, restore y comprobación de
//! integridad entre la base y los blobs.
//!
//! El backup copia la base con `VACUUM INTO` (copia consistente dentro de una transacción
//! de lectura) y después los blobs que esa copia referencia, verificando su hash. Como las
//! versiones nunca se borran, un blob referenciado en la copia sigue existiendo en disco.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use sqlx::sqlite::SqlitePoolOptions;

use crate::identity::audit;
use crate::{DB_FILE, MIGRATOR, Store, StoreError, migrate, read_only_options};

/// Versión del formato del backup. Cambia solo con cambios incompatibles.
pub const BACKUP_FORMAT: u32 = 1;
pub const MANIFEST_FILE: &str = "manifest.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Maintenance {
    pub reason: String,
    pub actor: Option<String>,
    pub started_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: u32,
    pub created_at: String,
    pub onepackd_version: String,
    /// Última migración aplicada en la base copiada.
    pub schema_version: i64,
    pub database: FileEntry,
    pub feeds: u64,
    pub versions: u64,
    pub blobs: Vec<BlobEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobEntry {
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RestoreReport {
    pub schema_version: i64,
    pub migrations_applied: usize,
    pub versions: u64,
    pub blobs: usize,
}

/// Resultado de `onepackd check`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CheckReport {
    pub versions: u64,
    pub blobs_checked: usize,
    pub hashes_verified: bool,
    /// Blobs referenciados sin archivo: esas versiones no se pueden descargar.
    pub missing: Vec<BlobProblem>,
    /// Archivos con tamaño o hash distinto del esperado.
    pub corrupt: Vec<BlobProblem>,
    /// Archivos que ninguna versión referencia; la limpieza los elimina tras la gracia.
    pub orphan_files: Vec<String>,
    pub staging_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlobProblem {
    pub sha256: String,
    pub detail: String,
    /// Versiones afectadas (`feed/Id@versión`).
    pub versions: Vec<String>,
}

impl CheckReport {
    pub fn ok(&self) -> bool {
        self.missing.is_empty() && self.corrupt.is_empty()
    }
}

#[derive(Debug)]
pub enum OpsError {
    Store(StoreError),
    /// El destino del backup o del restore no está vacío.
    NotEmpty(PathBuf),
    /// Ya hay un mantenimiento en curso (p. ej. otro backup).
    MaintenanceActive(Maintenance),
    /// El backup no supera la verificación.
    InvalidBackup(String),
    /// Un blob de origen no coincide con su hash: el backup no se completa.
    CorruptSource {
        sha256: String,
        detail: String,
    },
}

impl std::fmt::Display for OpsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(e) => e.fmt(f),
            Self::NotEmpty(p) => write!(f, "{} existe y no está vacío", p.display()),
            Self::MaintenanceActive(m) => write!(
                f,
                "ya hay un mantenimiento en curso ({}, desde {}, caduca {}); si quedó de un backup interrumpido, termínalo con `onepackd maintenance off`",
                m.reason, m.started_at, m.expires_at
            ),
            Self::InvalidBackup(m) => write!(f, "backup inválido: {m}"),
            Self::CorruptSource { sha256, detail } => write!(
                f,
                "el blob {sha256} está dañado ({detail}); ejecuta `onepackd check` antes de hacer backup"
            ),
        }
    }
}

impl std::error::Error for OpsError {}

impl<E: Into<StoreError>> From<E> for OpsError {
    fn from(e: E) -> Self {
        Self::Store(e.into())
    }
}

// ---------------------------------------------------------------------------------------------
// Mantenimiento
// ---------------------------------------------------------------------------------------------

impl Store {
    /// Mantenimiento vigente, si lo hay (uno caducado se ignora).
    pub async fn maintenance(&self) -> Result<Option<Maintenance>, StoreError> {
        let row = sqlx::query(
            "SELECT reason, actor, started_at, expires_at FROM maintenance
             WHERE expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .fetch_optional(&self.reader)
        .await?;
        Ok(row.map(|r| Maintenance {
            reason: r.get("reason"),
            actor: r.get("actor"),
            started_at: r.get("started_at"),
            expires_at: r.get("expires_at"),
        }))
    }

    /// Activa el mantenimiento durante `ttl_secs` como máximo. Falla si ya hay uno vigente.
    pub async fn begin_maintenance(
        &self,
        reason: &str,
        ttl_secs: u64,
        actor: &str,
    ) -> Result<Result<(), Maintenance>, StoreError> {
        let mut tx = self.writer.begin().await?;
        sqlx::query(
            "DELETE FROM maintenance WHERE expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .execute(&mut *tx)
        .await?;
        if let Some(r) =
            sqlx::query("SELECT reason, actor, started_at, expires_at FROM maintenance")
                .fetch_optional(&mut *tx)
                .await?
        {
            return Ok(Err(Maintenance {
                reason: r.get("reason"),
                actor: r.get("actor"),
                started_at: r.get("started_at"),
                expires_at: r.get("expires_at"),
            }));
        }
        sqlx::query(
            "INSERT INTO maintenance (id, reason, actor, expires_at)
             VALUES (1, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?))",
        )
        .bind(reason)
        .bind(actor)
        .bind(format!("+{ttl_secs} seconds"))
        .execute(&mut *tx)
        .await?;
        audit(
            &mut tx,
            "maintenance.begin",
            Some(actor),
            None,
            Some(reason),
            "success",
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(Ok(()))
    }

    /// Termina el mantenimiento. `false` si no había ninguno.
    pub async fn end_maintenance(&self, actor: &str) -> Result<bool, StoreError> {
        let mut tx = self.writer.begin().await?;
        let ended = sqlx::query("DELETE FROM maintenance")
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        if ended {
            audit(
                &mut tx,
                "maintenance.end",
                Some(actor),
                None,
                None,
                "success",
                None,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(ended)
    }

    /// La base responde (para `/readyz`).
    pub async fn ping(&self) -> Result<(), StoreError> {
        sqlx::query("SELECT 1").execute(&self.reader).await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Utilidades de archivos (bloqueantes: se ejecutan en `spawn_blocking`)
// ---------------------------------------------------------------------------------------------

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Copia `src` en `dst` calculando el SHA-256 de lo copiado, y deja `dst` en disco.
fn copy_hashed(src: &Path, dst: &Path) -> io::Result<(String, u64)> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut input = std::fs::File::open(src)?;
    let mut output = std::fs::File::create(dst)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut size = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        output.write_all(&buf[..n])?;
        size += n as u64;
    }
    output.sync_all()?;
    Ok((hex(&hasher.finalize()), size))
}

fn hash_file(path: &Path) -> io::Result<(String, u64)> {
    let mut input = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut size = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hex(&hasher.finalize()), size))
}

/// Ruta relativa de un blob: `blobs/sha256/ab/cd/<hash>` (ADR-005).
fn blob_rel(sha256: &str) -> PathBuf {
    PathBuf::from("blobs")
        .join("sha256")
        .join(&sha256[0..2])
        .join(&sha256[2..4])
        .join(sha256)
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Un destino vale si no existe o es un directorio vacío.
fn ensure_empty(dir: &Path) -> Result<(), OpsError> {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                Err(OpsError::NotEmpty(dir.to_owned()))
            } else {
                Ok(())
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            std::fs::create_dir_all(dir)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(io::Error::other)?
}

// ---------------------------------------------------------------------------------------------
// Backup
// ---------------------------------------------------------------------------------------------

/// Hace un backup completo de `data_dir` en `out` (que debe estar vacío o no existir).
/// Activa el mantenimiento durante la copia y lo termina siempre, también si falla.
pub async fn backup(
    data_dir: &Path,
    out: &Path,
    actor: &str,
    maintenance_ttl_secs: u64,
) -> Result<BackupManifest, OpsError> {
    ensure_empty(out)?;
    let store = Store::open(data_dir).await?;
    if let Err(active) = store
        .begin_maintenance("backup", maintenance_ttl_secs, actor)
        .await?
    {
        store.close().await;
        return Err(OpsError::MaintenanceActive(active));
    }
    let result = copy_backup(&store, data_dir, out).await;
    let ended = store.end_maintenance(actor).await;
    store.close().await;
    let manifest = result?;
    ended?;
    Ok(manifest)
}

async fn copy_backup(
    store: &Store,
    data_dir: &Path,
    out: &Path,
) -> Result<BackupManifest, OpsError> {
    let db_copy = out.join(DB_FILE);
    sqlx::query("VACUUM INTO ?")
        .bind(db_copy.to_string_lossy().into_owned())
        .execute(&store.writer)
        .await?;

    // Lo que se copia es lo que referencia la copia de la base, no la base viva.
    let copy = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(read_only_options(&db_copy))
        .await?;
    let schema_version: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&copy)
            .await?;
    let feeds: i64 = sqlx::query_scalar("SELECT count(*) FROM feed")
        .fetch_one(&copy)
        .await?;
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM package_version")
        .fetch_one(&copy)
        .await?;
    let referenced: Vec<(String, i64)> = sqlx::query_as(
        "SELECT DISTINCT v.blob_sha256, b.size FROM package_version v
         JOIN blob b ON b.sha256 = v.blob_sha256 ORDER BY v.blob_sha256",
    )
    .fetch_all(&copy)
    .await?;
    copy.close().await;

    let mut blobs = Vec::with_capacity(referenced.len());
    for (sha256, size) in referenced {
        let src = data_dir.join(blob_rel(&sha256));
        let dst = out.join(blob_rel(&sha256));
        let (actual, actual_size) =
            blocking(move || copy_hashed(&src, &dst))
                .await
                .map_err(|e| OpsError::CorruptSource {
                    sha256: sha256.clone(),
                    detail: format!("no se pudo leer: {e}"),
                })?;
        if actual != sha256 || actual_size != size as u64 {
            return Err(OpsError::CorruptSource {
                sha256,
                detail: format!("hash {actual}, {actual_size} bytes; esperado {size} bytes"),
            });
        }
        blobs.push(BlobEntry {
            sha256,
            size: actual_size,
        });
    }

    let db_path = db_copy.clone();
    let (db_sha, db_size) = blocking(move || hash_file(&db_path)).await?;
    let manifest = BackupManifest {
        format: BACKUP_FORMAT,
        created_at: now_rfc3339(store).await?,
        onepackd_version: env!("CARGO_PKG_VERSION").to_owned(),
        schema_version,
        database: FileEntry {
            path: DB_FILE.to_owned(),
            sha256: db_sha,
            size: db_size,
        },
        feeds: feeds as u64,
        versions: versions as u64,
        blobs,
    };
    let json = serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?;
    let manifest_path = out.join(MANIFEST_FILE);
    blocking(move || {
        let mut f = std::fs::File::create(&manifest_path)?;
        f.write_all(&json)?;
        f.sync_all()
    })
    .await?;
    Ok(manifest)
}

async fn now_rfc3339(store: &Store) -> Result<String, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
            .fetch_one(&store.reader)
            .await?,
    )
}

// ---------------------------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------------------------

/// Lee y valida el manifiesto de un backup, y verifica la base y cada blob contra sus
/// hashes. No escribe nada.
pub async fn verify_backup(backup: &Path) -> Result<BackupManifest, OpsError> {
    let invalid = |m: String| OpsError::InvalidBackup(m);
    let manifest_path = backup.join(MANIFEST_FILE);
    let bytes = tokio::fs::read(&manifest_path)
        .await
        .map_err(|e| invalid(format!("{}: {e}", manifest_path.display())))?;
    let manifest: BackupManifest =
        serde_json::from_slice(&bytes).map_err(|e| invalid(format!("manifiesto ilegible: {e}")))?;
    if manifest.format != BACKUP_FORMAT {
        return Err(invalid(format!(
            "formato {} no soportado (esta versión lee el {BACKUP_FORMAT})",
            manifest.format
        )));
    }
    if !MIGRATOR.version_exists(manifest.schema_version) {
        return Err(invalid(format!(
            "el esquema {} es más nuevo que esta versión de onepackd ({})",
            manifest.schema_version, manifest.onepackd_version
        )));
    }
    if manifest.database.path != DB_FILE {
        return Err(invalid("ruta de base inesperada en el manifiesto".into()));
    }
    let db = backup.join(DB_FILE);
    let (sha, size) = blocking(move || hash_file(&db))
        .await
        .map_err(|e| invalid(format!("{DB_FILE}: {e}")))?;
    if sha != manifest.database.sha256 || size != manifest.database.size {
        return Err(invalid(format!("{DB_FILE} no coincide con el manifiesto")));
    }
    for blob in &manifest.blobs {
        if !is_sha256_hex(&blob.sha256) {
            return Err(invalid(format!(
                "hash inválido en el manifiesto: {}",
                blob.sha256
            )));
        }
        let path = backup.join(blob_rel(&blob.sha256));
        let (sha, size) = blocking(move || hash_file(&path))
            .await
            .map_err(|e| invalid(format!("falta el blob {}: {e}", blob.sha256)))?;
        if sha != blob.sha256 || size != blob.size {
            return Err(invalid(format!("el blob {} está dañado", blob.sha256)));
        }
    }
    Ok(manifest)
}

/// Restaura un backup en un directorio de datos vacío (o inexistente). Verifica todo antes de
/// copiar y migra la base si el backup es de una versión anterior.
pub async fn restore(backup: &Path, data_dir: &Path) -> Result<RestoreReport, OpsError> {
    ensure_empty(data_dir)?;
    let manifest = verify_backup(backup).await?;
    let result = async {
        let (src, dst) = (backup.join(DB_FILE), data_dir.join(DB_FILE));
        blocking(move || copy_hashed(&src, &dst)).await?;
        for blob in &manifest.blobs {
            let src = backup.join(blob_rel(&blob.sha256));
            let dst = data_dir.join(blob_rel(&blob.sha256));
            let (sha, _) = blocking(move || copy_hashed(&src, &dst)).await?;
            if sha != blob.sha256 {
                return Err(OpsError::InvalidBackup(format!(
                    "el blob {} cambió durante la copia",
                    blob.sha256
                )));
            }
        }
        let migrated = migrate(data_dir).await?;
        // Comprueba que el resultado abre y es coherente.
        let report = check(data_dir, false).await?;
        if !report.ok() {
            return Err(OpsError::InvalidBackup(format!(
                "tras restaurar faltan {} blob(s) o están dañados",
                report.missing.len() + report.corrupt.len()
            )));
        }
        Ok(RestoreReport {
            schema_version: manifest.schema_version,
            migrations_applied: migrated.applied,
            versions: manifest.versions,
            blobs: manifest.blobs.len(),
        })
    }
    .await;
    if result.is_err() {
        // No deja un directorio a medias que alguien pudiera arrancar.
        if let Ok(mut entries) = tokio::fs::read_dir(data_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let _ = if path.is_dir() {
                    tokio::fs::remove_dir_all(&path).await
                } else {
                    tokio::fs::remove_file(&path).await
                };
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------------------------
// Check
// ---------------------------------------------------------------------------------------------

/// Integridad entre la base y los blobs. Con `verify_hashes`, lee cada blob entero.
pub async fn check(data_dir: &Path, verify_hashes: bool) -> Result<CheckReport, OpsError> {
    let store = Store::open(data_dir).await?;
    let rows = sqlx::query(
        "SELECT v.blob_sha256, b.size, f.name AS feed, v.package_id, v.version
         FROM package_version v
         JOIN blob b ON b.sha256 = v.blob_sha256
         JOIN feed f ON f.id = v.feed_id
         ORDER BY v.blob_sha256",
    )
    .fetch_all(&store.reader)
    .await?;
    let mut report = CheckReport {
        versions: rows.len() as u64,
        hashes_verified: verify_hashes,
        ..CheckReport::default()
    };
    // Agrupa versiones por blob (varias versiones pueden compartir contenido).
    let mut by_blob: Vec<(String, u64, Vec<String>)> = Vec::new();
    for r in &rows {
        let sha: String = r.get("blob_sha256");
        let label = format!(
            "{}/{}@{}",
            r.get::<String, _>("feed"),
            r.get::<String, _>("package_id"),
            r.get::<String, _>("version")
        );
        match by_blob.last_mut() {
            Some((last, _, versions)) if *last == sha => versions.push(label),
            _ => by_blob.push((sha, r.get::<i64, _>("size") as u64, vec![label])),
        }
    }
    report.blobs_checked = by_blob.len();
    for (sha, size, versions) in by_blob {
        let path = data_dir.join(blob_rel(&sha));
        let meta = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) => {
                report.missing.push(BlobProblem {
                    sha256: sha,
                    detail: e.to_string(),
                    versions,
                });
                continue;
            }
        };
        if meta.len() != size {
            report.corrupt.push(BlobProblem {
                sha256: sha,
                detail: format!("{} bytes; esperado {size}", meta.len()),
                versions,
            });
            continue;
        }
        if verify_hashes {
            let (actual, _) = blocking(move || hash_file(&path)).await?;
            if actual != sha {
                report.corrupt.push(BlobProblem {
                    sha256: sha,
                    detail: format!("el contenido tiene hash {actual}"),
                    versions,
                });
            }
        }
    }
    let referenced: std::collections::HashSet<String> = rows
        .iter()
        .map(|r| r.get::<String, _>("blob_sha256"))
        .collect();
    report.orphan_files = store
        .blobs()
        .list_older_than(std::time::Duration::ZERO)
        .await?
        .into_iter()
        .filter(|sha| !referenced.contains(sha))
        .collect();
    report.orphan_files.sort();
    report.staging_files = std::fs::read_dir(data_dir.join("staging"))
        .map(|d| d.count())
        .unwrap_or(0);
    store.close().await;
    Ok(report)
}
