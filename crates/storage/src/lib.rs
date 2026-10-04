//! Persistencia: metadatos en SQLite y blobs direccionados por SHA-256 (ADR-004, ADR-005).

pub mod blobs;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onepack_core::{Feed, FeedName, NewVersion, PublishError, PublishedVersion};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};

pub use blobs::{BlobStore, StagedBlob, StagingError, StagingWriter};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

const DB_FILE: &str = "metadata.sqlite";

#[derive(Debug)]
pub enum StoreError {
    /// No hay base de datos en el directorio de datos.
    NotInitialized(PathBuf),
    /// Hay migraciones sin aplicar: hay que ejecutar `onepackd migrate`.
    PendingMigrations(usize),
    /// La base fue migrada por una versión más nueva de onepackd.
    SchemaTooNew {
        version: i64,
    },
    Io(io::Error),
    Database(sqlx::Error),
    Migrate(sqlx::migrate::MigrateError),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInitialized(p) => write!(
                f,
                "no existe {}; ejecuta `onepackd migrate` para inicializar el directorio de datos",
                p.display()
            ),
            Self::PendingMigrations(n) => write!(
                f,
                "hay {n} migración(es) pendiente(s); ejecuta `onepackd migrate` (se hará un backup previo)"
            ),
            Self::SchemaTooNew { version } => write!(
                f,
                "la base de datos tiene la migración {version}, más nueva que esta versión de onepackd"
            ),
            Self::Io(e) => write!(f, "error de E/S: {e}"),
            Self::Database(e) => write!(f, "error de base de datos: {e}"),
            Self::Migrate(e) => write!(f, "error de migración: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub applied: usize,
    /// Copia de la base previa a migrar, si había datos (ADR-020).
    pub backup: Option<PathBuf>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    pub staging_removed: usize,
    pub orphan_blobs_removed: usize,
}

pub struct Store {
    /// Un único escritor: SQLite admite un solo escritor a la vez (ADR-004).
    writer: SqlitePool,
    reader: SqlitePool,
    blobs: BlobStore,
}

fn connect_options(db: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(db)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
}

/// Aplica las migraciones pendientes. Si la base ya tenía esquema, guarda antes una copia
/// consistente en `backups/` con `VACUUM INTO`.
pub async fn migrate(data_dir: &Path) -> Result<MigrationReport, StoreError> {
    tokio::fs::create_dir_all(data_dir).await?;
    let db = data_dir.join(DB_FILE);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options(&db).create_if_missing(true))
        .await?;

    let applied = applied_versions(&pool).await?;
    check_not_newer(&applied)?;
    let pending = MIGRATOR
        .iter()
        .filter(|m| !applied.contains(&m.version))
        .count();
    let mut report = MigrationReport::default();
    if pending == 0 {
        pool.close().await;
        return Ok(report);
    }

    if let Some(current) = applied.iter().max() {
        let dir = data_dir.join("backups");
        tokio::fs::create_dir_all(&dir).await?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let backup = dir.join(format!("metadata-pre-migrate-v{current}-{stamp}.sqlite"));
        sqlx::query("VACUUM INTO ?")
            .bind(backup.to_string_lossy().into_owned())
            .execute(&pool)
            .await?;
        report.backup = Some(backup);
    }

    MIGRATOR.run(&pool).await.map_err(StoreError::Migrate)?;
    report.applied = pending;
    pool.close().await;
    Ok(report)
}

async fn applied_versions(pool: &SqlitePool) -> Result<Vec<i64>, StoreError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?;
    if !exists {
        return Ok(Vec::new());
    }
    Ok(
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success = 1")
            .fetch_all(pool)
            .await?,
    )
}

fn check_not_newer(applied: &[i64]) -> Result<(), StoreError> {
    match applied.iter().find(|v| !MIGRATOR.version_exists(**v)) {
        Some(&version) => Err(StoreError::SchemaTooNew { version }),
        None => Ok(()),
    }
}

impl Store {
    /// Abre un directorio de datos ya migrado. No crea ni migra la base.
    pub async fn open(data_dir: &Path) -> Result<Self, StoreError> {
        let db = data_dir.join(DB_FILE);
        if !tokio::fs::try_exists(&db).await? {
            return Err(StoreError::NotInitialized(db));
        }
        let writer = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(connect_options(&db))
            .await?;
        let reader = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(connect_options(&db))
            .await?;

        let applied = applied_versions(&reader).await?;
        check_not_newer(&applied)?;
        let pending = MIGRATOR
            .iter()
            .filter(|m| !applied.contains(&m.version))
            .count();
        if pending > 0 {
            return Err(StoreError::PendingMigrations(pending));
        }

        Ok(Self {
            writer,
            reader,
            blobs: BlobStore::open(data_dir).await?,
        })
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    pub async fn close(&self) {
        self.writer.close().await;
        self.reader.close().await;
    }

    pub async fn create_feed(&self, name: &FeedName) -> Result<Feed, StoreError> {
        let mut tx = self.writer.begin().await?;
        let id: i64 =
            sqlx::query_scalar("INSERT INTO feed (name, format) VALUES (?, 'nuget') RETURNING id")
                .bind(name.as_str())
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query("INSERT INTO audit_event (action, feed_id, resource, outcome) VALUES ('feed.create', ?, ?, 'success')")
            .bind(id)
            .bind(name.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Feed {
            id,
            name: name.clone(),
        })
    }

    pub async fn feed(&self, name: &str) -> Result<Option<Feed>, StoreError> {
        let row = sqlx::query("SELECT id, name FROM feed WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.reader)
            .await?;
        Ok(row.and_then(|r| {
            FeedName::parse(r.get::<&str, _>("name"))
                .ok()
                .map(|name| Feed {
                    id: r.get("id"),
                    name,
                })
        }))
    }

    pub async fn feeds(&self) -> Result<Vec<Feed>, StoreError> {
        let rows = sqlx::query("SELECT id, name FROM feed ORDER BY name")
            .fetch_all(&self.reader)
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                FeedName::parse(r.get::<&str, _>("name"))
                    .ok()
                    .map(|name| Feed {
                        id: r.get("id"),
                        name,
                    })
            })
            .collect())
    }

    /// Publica una versión siguiendo ADR-006: el blob queda durable antes de confirmar los
    /// metadatos, y metadatos + auditoría se confirman en una única transacción corta.
    pub async fn publish(
        &self,
        feed: &Feed,
        version: &NewVersion,
        staged: StagedBlob,
    ) -> Result<(), PublishError> {
        // Comprobación previa barata; la garantía la da la restricción UNIQUE.
        if let Some(existing) = self
            .version(feed, &version.package_key, &version.version_key)
            .await
            .map_err(db_err)?
        {
            let identical = existing.blob_sha256 == staged.sha256();
            self.audit_conflict(feed, version, identical).await;
            return Err(PublishError::Conflict { identical });
        }

        let sha256 = staged.sha256().to_owned();
        let size = staged.size() as i64;
        self.blobs.persist(staged).await?;

        #[cfg(feature = "fault-injection")]
        if std::env::var_os("ONEPACK_FAULT_ABORT_AFTER_BLOB_PERSIST").is_some() {
            std::process::abort();
        }

        match self.insert_version(feed, version, &sha256, size).await {
            Ok(()) => Ok(()),
            Err(e)
                if e.as_database_error()
                    .is_some_and(|d| d.is_unique_violation()) =>
            {
                // Carrera con otra publicación de la misma identidad: ganó la otra.
                let existing = self
                    .version(feed, &version.package_key, &version.version_key)
                    .await
                    .map_err(db_err)?;
                let identical = existing.is_some_and(|v| v.blob_sha256 == sha256);
                self.audit_conflict(feed, version, identical).await;
                Err(PublishError::Conflict { identical })
            }
            Err(e) => Err(sqlx_to_publish(e)),
        }
    }

    async fn insert_version(
        &self,
        feed: &Feed,
        v: &NewVersion,
        sha256: &str,
        size: i64,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.writer.begin().await?;
        sqlx::query(
            "INSERT INTO blob (sha256, size) VALUES (?, ?) ON CONFLICT (sha256) DO NOTHING",
        )
        .bind(sha256)
        .bind(size)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO package (feed_id, normalized_package_id, package_id) VALUES (?, ?, ?)
             ON CONFLICT (feed_id, normalized_package_id) DO NOTHING",
        )
        .bind(feed.id)
        .bind(&v.package_key)
        .bind(&v.package_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO package_version (feed_id, normalized_package_id, normalized_version, package_id,
                 version, full_version, is_prerelease, is_semver2, blob_sha256)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(feed.id)
        .bind(&v.package_key)
        .bind(&v.version_key)
        .bind(&v.package_id)
        .bind(&v.version)
        .bind(&v.full_version)
        .bind(v.is_prerelease)
        .bind(v.is_semver2)
        .bind(sha256)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO audit_event (action, feed_id, resource, outcome, detail)
             VALUES ('package.publish', ?, ?, 'success', json_object('sha256', ?, 'size', ?))",
        )
        .bind(feed.id)
        .bind(v.resource())
        .bind(sha256)
        .bind(size)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }

    async fn audit_conflict(&self, feed: &Feed, v: &NewVersion, identical: bool) {
        let result = sqlx::query(
            "INSERT INTO audit_event (action, feed_id, resource, outcome, detail)
             VALUES ('package.publish', ?, ?, 'conflict', json_object('identical', json(?)))",
        )
        .bind(feed.id)
        .bind(v.resource())
        .bind(if identical { "true" } else { "false" })
        .execute(&self.writer)
        .await;
        if let Err(e) = result {
            tracing::error!(error = %e, "no se pudo auditar el conflicto de publicación");
        }
    }

    pub async fn version(
        &self,
        feed: &Feed,
        package_key: &str,
        version_key: &str,
    ) -> Result<Option<PublishedVersion>, StoreError> {
        let row = sqlx::query(
            "SELECT v.package_id, v.version, v.normalized_version, v.blob_sha256, b.size, v.listed
             FROM package_version v JOIN blob b ON b.sha256 = v.blob_sha256
             WHERE v.feed_id = ? AND v.normalized_package_id = ? AND v.normalized_version = ?",
        )
        .bind(feed.id)
        .bind(package_key)
        .bind(version_key)
        .fetch_optional(&self.reader)
        .await?;
        Ok(row.map(|r| to_published(&r)))
    }

    /// Versiones de un paquete, sin orden garantizado: el orden depende del formato.
    pub async fn versions(
        &self,
        feed: &Feed,
        package_key: &str,
    ) -> Result<Vec<PublishedVersion>, StoreError> {
        let rows = sqlx::query(
            "SELECT v.package_id, v.version, v.normalized_version, v.blob_sha256, b.size, v.listed
             FROM package_version v JOIN blob b ON b.sha256 = v.blob_sha256
             WHERE v.feed_id = ? AND v.normalized_package_id = ?",
        )
        .bind(feed.id)
        .bind(package_key)
        .fetch_all(&self.reader)
        .await?;
        Ok(rows.iter().map(to_published).collect())
    }

    /// Elimina subidas abandonadas y blobs que ninguna versión referencia, con un periodo de
    /// gracia para no competir con publicaciones en curso (ADR-006).
    pub async fn gc(&self, grace: Duration) -> Result<GcReport, StoreError> {
        let mut report = GcReport {
            staging_removed: self.blobs.clean_staging(grace).await?,
            ..Default::default()
        };
        for sha256 in self.blobs.list_older_than(grace).await? {
            let referenced: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM package_version WHERE blob_sha256 = ?)",
            )
            .bind(&sha256)
            .fetch_one(&self.writer)
            .await?;
            if referenced {
                continue;
            }
            sqlx::query("DELETE FROM blob WHERE sha256 = ?")
                .bind(&sha256)
                .execute(&self.writer)
                .await?;
            self.blobs.remove(&sha256).await?;
            tracing::info!(sha256, "blob huérfano eliminado");
            report.orphan_blobs_removed += 1;
        }
        Ok(report)
    }

    /// Comprobaciones de arranque: el directorio de datos admite escritura y hay espacio libre.
    pub async fn check_data_dir(data_dir: &Path, min_free_bytes: u64) -> Result<u64, StoreError> {
        let probe = data_dir.join("staging").join(".write-probe");
        tokio::fs::write(&probe, b"ok").await?;
        tokio::fs::remove_file(&probe).await?;
        let free = fs4::available_space(data_dir)?;
        if free < min_free_bytes {
            tracing::warn!(
                free_bytes = free,
                min_free_bytes,
                "poco espacio libre en el directorio de datos"
            );
        }
        Ok(free)
    }
}

fn to_published(r: &sqlx::sqlite::SqliteRow) -> PublishedVersion {
    PublishedVersion {
        package_id: r.get("package_id"),
        version: r.get("version"),
        version_key: r.get("normalized_version"),
        blob_sha256: r.get("blob_sha256"),
        size: r.get::<i64, _>("size") as u64,
        listed: r.get("listed"),
    }
}

fn db_err(e: StoreError) -> PublishError {
    match e {
        StoreError::Io(e) => PublishError::from(e),
        other => PublishError::Database(other.to_string()),
    }
}

fn sqlx_to_publish(e: sqlx::Error) -> PublishError {
    match e {
        sqlx::Error::Io(io) => PublishError::from(io),
        // SQLITE_FULL (13): base de datos o disco lleno.
        e if e
            .as_database_error()
            .and_then(|d| d.code())
            .is_some_and(|c| c == "13") =>
        {
            PublishError::StorageFull
        }
        e => PublishError::Database(e.to_string()),
    }
}
