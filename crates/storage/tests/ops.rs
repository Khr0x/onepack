//! Recuperación (Fase 7, ADR-017, ADR-020): mantenimiento, backup, restore, comprobación de
//! integridad y actualización del esquema desde la versión anterior.

use std::borrow::Cow;
use std::path::Path;

use onepack_core::{Feed, FeedName, NewVersion};
use onepack_storage::{OpsError, Store, backup, check, migrate, restore, schema_status};
use tempfile::TempDir;

async fn seeded() -> (TempDir, Feed) {
    let dir = TempDir::new().unwrap();
    migrate(dir.path()).await.unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let feed = store
        .create_feed(&FeedName::parse("internal").unwrap(), "test")
        .await
        .unwrap();
    // Dos versiones con el mismo contenido y una distinta.
    for (id, v, content) in [
        ("Hemia.Core", "1.0.0", b"contenido A".as_slice()),
        ("Hemia.Core", "1.1.0", b"contenido B"),
        ("Hemia.Copy", "1.0.0", b"contenido A"),
    ] {
        let mut w = store.blobs().begin_staging(1 << 20).await.unwrap();
        w.write(content).await.unwrap();
        let staged = w.finish().await.unwrap();
        store
            .publish(&feed, &version(id, v), staged, Some("test"))
            .await
            .unwrap();
    }
    store.close().await;
    (dir, feed)
}

fn version(id: &str, v: &str) -> NewVersion {
    NewVersion {
        package_id: id.into(),
        package_key: id.to_lowercase(),
        version: v.into(),
        version_key: v.into(),
        full_version: v.into(),
        is_prerelease: false,
        is_semver2: false,
        metadata: "{}".into(),
        search_text: id.to_lowercase(),
    }
}

fn blob_files(dir: &Path) -> Vec<std::path::PathBuf> {
    fn walk(d: &Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                walk(&e.path(), out);
            } else {
                out.push(e.path());
            }
        }
    }
    let mut out = Vec::new();
    walk(&dir.join("blobs"), &mut out);
    out.sort();
    out
}

#[tokio::test]
async fn backup_and_restore_roundtrip() {
    let (src, feed) = seeded().await;
    let out = TempDir::new().unwrap();
    let manifest = backup(src.path(), out.path(), "test", 3600).await.unwrap();
    assert_eq!(manifest.versions, 3);
    assert_eq!(manifest.feeds, 1);
    assert_eq!(
        manifest.blobs.len(),
        2,
        "el contenido compartido se copia una vez"
    );
    assert_eq!(manifest.schema_version, 5);

    // El mantenimiento termina al acabar el backup.
    let store = Store::open(src.path()).await.unwrap();
    assert_eq!(store.maintenance().await.unwrap(), None);
    store.close().await;

    let dst = TempDir::new().unwrap();
    let target = dst.path().join("restored");
    let report = restore(out.path(), &target).await.unwrap();
    assert_eq!(report.versions, 3);
    assert_eq!(report.migrations_applied, 0);

    let restored = Store::open(&target).await.unwrap();
    let v = restored
        .version(&feed, "hemia.core", "1.1.0")
        .await
        .unwrap()
        .unwrap();
    let bytes = std::fs::read(restored.blobs().path(&v.blob_sha256)).unwrap();
    assert_eq!(bytes, b"contenido B");
    restored.close().await;
    assert!(check(&target, true).await.unwrap().ok());
}

#[tokio::test]
async fn restore_requires_an_empty_target_and_a_valid_backup() {
    let (src, _) = seeded().await;
    let out = TempDir::new().unwrap();
    backup(src.path(), out.path(), "test", 3600).await.unwrap();

    // Destino ocupado.
    assert!(matches!(
        restore(out.path(), src.path()).await,
        Err(OpsError::NotEmpty(_))
    ));
    // Backup en un destino ocupado.
    assert!(matches!(
        backup(src.path(), out.path(), "test", 3600).await,
        Err(OpsError::NotEmpty(_))
    ));

    // Un blob alterado en el backup: no se restaura nada.
    let tampered = blob_files(out.path()).pop().unwrap();
    std::fs::write(&tampered, b"alterado").unwrap();
    let dst = TempDir::new().unwrap();
    let err = restore(out.path(), dst.path()).await.unwrap_err();
    assert!(matches!(err, OpsError::InvalidBackup(_)), "{err}");
    assert_eq!(std::fs::read_dir(dst.path()).unwrap().count(), 0);

    // Sin manifiesto tampoco.
    std::fs::remove_file(out.path().join("manifest.json")).unwrap();
    assert!(matches!(
        restore(out.path(), dst.path()).await,
        Err(OpsError::InvalidBackup(_))
    ));
}

#[tokio::test]
async fn maintenance_is_exclusive_and_expires() {
    let (src, _) = seeded().await;
    let store = Store::open(src.path()).await.unwrap();
    store
        .begin_maintenance("manual", 3600, "test")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.maintenance().await.unwrap().unwrap().reason, "manual");

    // Un backup no puede empezar mientras hay otro mantenimiento.
    let out = TempDir::new().unwrap();
    assert!(matches!(
        backup(src.path(), out.path(), "test", 3600).await,
        Err(OpsError::MaintenanceActive(_))
    ));
    assert!(store.end_maintenance("test").await.unwrap());
    assert!(!store.end_maintenance("test").await.unwrap());

    // Uno caducado (p. ej. de un backup interrumpido) no bloquea.
    store
        .begin_maintenance("viejo", 0, "test")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.maintenance().await.unwrap(), None);
    assert!(
        store
            .begin_maintenance("nuevo", 60, "test")
            .await
            .unwrap()
            .is_ok()
    );
    store.close().await;
}

#[tokio::test]
async fn check_detects_missing_corrupt_and_orphan_blobs() {
    let (src, _) = seeded().await;
    assert!(check(src.path(), true).await.unwrap().ok());

    let files = blob_files(src.path());
    // Blob compartido por dos versiones, eliminado a mano.
    let shared = files
        .iter()
        .find(|p| std::fs::read(p).unwrap() == b"contenido A")
        .unwrap()
        .clone();
    std::fs::remove_file(&shared).unwrap();
    // El otro, alterado con el mismo tamaño: solo lo detecta la verificación de hashes.
    let other = files.iter().find(|p| **p != shared).unwrap().clone();
    std::fs::write(&other, b"contenido X").unwrap();
    // Y un archivo que nadie referencia.
    let orphan = "f".repeat(64);
    let orphan_path = src.path().join("blobs/sha256/ff/ff").join(&orphan);
    std::fs::create_dir_all(orphan_path.parent().unwrap()).unwrap();
    std::fs::write(&orphan_path, b"huerfano").unwrap();

    let quick = check(src.path(), false).await.unwrap();
    assert_eq!(quick.missing.len(), 1);
    assert_eq!(quick.missing[0].versions.len(), 2);
    assert!(quick.corrupt.is_empty(), "sin hashes no se nota");

    let full = check(src.path(), true).await.unwrap();
    assert!(!full.ok());
    assert_eq!(full.missing.len(), 1);
    assert_eq!(full.corrupt.len(), 1);
    assert_eq!(full.corrupt[0].versions, ["internal/Hemia.Core@1.1.0"]);
    assert_eq!(full.orphan_files, [orphan]);

    // Con un blob dañado, el backup se niega en lugar de copiar basura.
    let out = TempDir::new().unwrap();
    assert!(matches!(
        backup(src.path(), out.path(), "test", 3600).await,
        Err(OpsError::CorruptSource { .. })
    ));
    let store = Store::open(src.path()).await.unwrap();
    assert_eq!(
        store.maintenance().await.unwrap(),
        None,
        "termina aunque falle"
    );
    store.close().await;
}

/// Actualización N-1 → N: un directorio creado con todas las migraciones menos la última
/// se actualiza conservando los datos y dejando un backup previo.
#[tokio::test]
async fn upgrade_from_previous_schema_keeps_data_and_backs_up_first() {
    let dir = TempDir::new().unwrap();
    let mut previous = sqlx::migrate!("../../migrations");
    let latest = previous.iter().map(|m| m.version).max().unwrap();
    previous.migrations = Cow::Owned(
        previous
            .iter()
            .filter(|m| m.version < latest)
            .cloned()
            .collect(),
    );
    let db = dir.path().join("metadata.sqlite");
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    previous.run(&pool).await.unwrap();
    sqlx::query("INSERT INTO feed (name, format) VALUES ('legacy', 'nuget')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let status = schema_status(dir.path()).await.unwrap();
    assert_eq!(status.current, Some(latest - 1));
    assert_eq!(status.pending, [latest]);
    assert!(matches!(
        Store::open(dir.path()).await,
        Err(onepack_storage::StoreError::PendingMigrations(1))
    ));

    let report = migrate(dir.path()).await.unwrap();
    assert_eq!(report.applied, 1);
    let backup_file = report.backup.expect("backup previo a migrar");
    assert!(backup_file.exists());

    let store = Store::open(dir.path()).await.unwrap();
    assert!(
        store.feed("legacy").await.unwrap().is_some(),
        "datos intactos"
    );
    store.close().await;
    assert!(schema_status(dir.path()).await.unwrap().pending.is_empty());
}
