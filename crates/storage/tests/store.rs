use std::sync::Arc;
use std::time::Duration;

use onepack_core::{Feed, FeedName, NewVersion, PublishError};
use onepack_storage::{StagedBlob, Store, StoreError, migrate};
use tempfile::TempDir;

async fn store() -> (Store, TempDir) {
    let dir = TempDir::new().unwrap();
    migrate(dir.path()).await.unwrap();
    (Store::open(dir.path()).await.unwrap(), dir)
}

async fn feed(store: &Store, name: &str) -> Feed {
    store
        .create_feed(&FeedName::parse(name).unwrap())
        .await
        .unwrap()
}

async fn stage(store: &Store, content: &[u8]) -> StagedBlob {
    let mut w = store.blobs().begin_staging(1 << 20).await.unwrap();
    w.write(content).await.unwrap();
    w.finish().await.unwrap()
}

fn version(id: &str, v: &str) -> NewVersion {
    NewVersion {
        package_id: id.into(),
        package_key: id.to_lowercase(),
        version: v.into(),
        version_key: v.to_lowercase(),
        full_version: v.into(),
        is_prerelease: v.contains('-'),
        is_semver2: false,
    }
}

fn staging_files(dir: &TempDir) -> usize {
    std::fs::read_dir(dir.path().join("staging"))
        .unwrap()
        .count()
}

#[tokio::test]
async fn open_requires_migrated_data_dir() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(
        Store::open(dir.path()).await,
        Err(StoreError::NotInitialized(_))
    ));

    let report = migrate(dir.path()).await.unwrap();
    assert_eq!(report.applied, 1);
    assert_eq!(report.backup, None, "una base nueva no necesita backup");
    assert_eq!(
        migrate(dir.path()).await.unwrap().applied,
        0,
        "migrar es idempotente"
    );
    assert!(Store::open(dir.path()).await.is_ok());
}

#[tokio::test]
async fn refuses_schema_from_newer_version() {
    let (store, dir) = store().await;
    store.close().await;
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("metadata.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES (9999, 'futura', 1, x'00', 0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    assert!(matches!(
        Store::open(dir.path()).await,
        Err(StoreError::SchemaTooNew { version: 9999 })
    ));
    assert!(matches!(
        migrate(dir.path()).await,
        Err(StoreError::SchemaTooNew { version: 9999 })
    ));
}

#[tokio::test]
async fn publish_persists_blob_and_metadata() {
    let (store, dir) = store().await;
    let feed = feed(&store, "internal").await;
    let staged = stage(&store, b"package-bytes").await;
    let sha = staged.sha256().to_owned();

    store
        .publish(&feed, &version("Hemia.Logging", "1.0.0"), staged)
        .await
        .unwrap();

    let v = store
        .version(&feed, "hemia.logging", "1.0.0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v.package_id, "Hemia.Logging");
    assert_eq!(v.blob_sha256, sha);
    assert_eq!(v.size, 13);
    assert!(v.listed);
    assert_eq!(
        std::fs::read(store.blobs().path(&sha)).unwrap(),
        b"package-bytes"
    );
    assert_eq!(staging_files(&dir), 0);
}

#[tokio::test]
async fn duplicate_identity_conflicts_and_reports_if_identical() {
    let (store, dir) = store().await;
    let feed = feed(&store, "internal").await;
    store
        .publish(
            &feed,
            &version("A", "1.0.0"),
            stage(&store, b"original").await,
        )
        .await
        .unwrap();

    let same = store
        .publish(
            &feed,
            &version("A", "1.0.0"),
            stage(&store, b"original").await,
        )
        .await;
    assert!(matches!(
        same,
        Err(PublishError::Conflict { identical: true })
    ));

    let different = store
        .publish(
            &feed,
            &version("a", "1.0.0"),
            stage(&store, b"changed").await,
        )
        .await;
    assert!(matches!(
        different,
        Err(PublishError::Conflict { identical: false })
    ));

    let v = store.version(&feed, "a", "1.0.0").await.unwrap().unwrap();
    assert_eq!(
        std::fs::read(store.blobs().path(&v.blob_sha256)).unwrap(),
        b"original"
    );
    assert_eq!(
        staging_files(&dir),
        0,
        "los conflictos no dejan archivos en staging"
    );
}

#[tokio::test]
async fn same_package_in_two_feeds_is_independent_and_deduplicated() {
    let (store, _dir) = store().await;
    let internal = feed(&store, "internal").await;
    let customer = feed(&store, "customer-a").await;

    store
        .publish(
            &internal,
            &version("A", "1.0.0"),
            stage(&store, b"same").await,
        )
        .await
        .unwrap();
    store
        .publish(
            &customer,
            &version("A", "1.0.0"),
            stage(&store, b"same").await,
        )
        .await
        .unwrap();

    let a = store
        .version(&internal, "a", "1.0.0")
        .await
        .unwrap()
        .unwrap();
    let b = store
        .version(&customer, "a", "1.0.0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.blob_sha256, b.blob_sha256);
    assert_eq!(
        store
            .blobs()
            .list_older_than(Duration::ZERO)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_publishes_of_same_identity_confirm_exactly_one() {
    let (store, _dir) = store().await;
    let store = Arc::new(store);
    let feed = feed(&store, "internal").await;

    let tasks: Vec<_> = (0..8)
        .map(|i| {
            let store = store.clone();
            let feed = feed.clone();
            tokio::spawn(async move {
                let content = format!("contenido-{i}");
                let staged = stage(&store, content.as_bytes()).await;
                (
                    content,
                    store
                        .publish(&feed, &version("Race", "1.0.0"), staged)
                        .await,
                )
            })
        })
        .collect();

    let mut winners = Vec::new();
    for task in tasks {
        match task.await.unwrap() {
            (content, Ok(())) => winners.push(content),
            (_, Err(PublishError::Conflict { identical: false })) => {}
            (_, other) => panic!("resultado inesperado: {other:?}"),
        }
    }
    assert_eq!(winners.len(), 1, "exactamente una publicación confirmada");

    let v = store
        .version(&feed, "race", "1.0.0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read(store.blobs().path(&v.blob_sha256)).unwrap(),
        winners[0].as_bytes()
    );
}

#[tokio::test]
async fn gc_removes_orphans_after_grace_and_keeps_referenced_blobs() {
    let (store, dir) = store().await;
    let feed = feed(&store, "internal").await;
    store
        .publish(
            &feed,
            &version("A", "1.0.0"),
            stage(&store, b"referenced").await,
        )
        .await
        .unwrap();

    // Huérfano: blob persistido sin metadatos (caída entre ADR-006 pasos 5 y 6).
    let orphan = stage(&store, b"orphan").await;
    let orphan_sha = orphan.sha256().to_owned();
    store.blobs().persist(orphan).await.unwrap();
    // Subida abandonada en staging.
    std::fs::write(dir.path().join("staging").join("dead.upload"), b"partial").unwrap();

    let report = store.gc(Duration::from_secs(3600)).await.unwrap();
    assert_eq!(
        report.orphan_blobs_removed, 0,
        "dentro del periodo de gracia no se borra nada"
    );
    assert_eq!(report.staging_removed, 0);

    let report = store.gc(Duration::ZERO).await.unwrap();
    assert_eq!(report.orphan_blobs_removed, 1);
    assert_eq!(report.staging_removed, 1);
    assert!(!store.blobs().path(&orphan_sha).exists());

    let v = store.version(&feed, "a", "1.0.0").await.unwrap().unwrap();
    assert!(
        store.blobs().path(&v.blob_sha256).exists(),
        "el blob referenciado se conserva"
    );
}
