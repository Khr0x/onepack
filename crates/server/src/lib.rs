//! Servidor `onepackd`: composición de la aplicación HTTP.

pub mod admin_api;
pub mod auth;
pub mod nuget_api;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{get, put};
use onepack_storage::Store;
use tower_http::trace::TraceLayer;

pub struct AppState {
    /// URL pública explícita: base de todas las URLs absolutas (ADR-018).
    pub public_url: String,
    pub store: Arc<Store>,
    pub max_package_bytes: u64,
    pub touched: auth::TouchTracker,
}

pub fn app(store: Arc<Store>, public_url: &str, max_package_bytes: u64) -> Router {
    let state = Arc::new(AppState {
        public_url: public_url.trim_end_matches('/').to_owned(),
        store,
        max_package_bytes,
        touched: auth::TouchTracker::default(),
    });

    Router::new()
        .route("/nuget/{feed}/v3/index.json", get(nuget_api::service_index))
        .route(
            "/nuget/{feed}/v3/flat/{id}/index.json",
            get(nuget_api::flat_versions),
        )
        .route(
            "/nuget/{feed}/v3/flat/{id}/{version}/{file}",
            get(nuget_api::flat_file),
        )
        // El cliente NuGet hace el PUT sobre el @id de PackagePublish con barra final.
        .route("/nuget/{feed}/v2/package", put(nuget_api::publish))
        .route("/nuget/{feed}/v2/package/", put(nuget_api::publish))
        .route(
            "/nuget/{feed}/v3/registration/{id}/index.json",
            get(nuget_api::registration_index),
        )
        .route(
            "/nuget/{feed}/v3/registration/{id}/page/{lower}/{upper}",
            get(nuget_api::registration_page),
        )
        .route(
            "/nuget/{feed}/v3/registration/{id}/{leaf}",
            get(nuget_api::registration_leaf),
        )
        .route("/nuget/{feed}/v3/query", get(nuget_api::search))
        .route(
            "/nuget/{feed}/v3/autocomplete",
            get(nuget_api::autocomplete),
        )
        .route(
            "/nuget/{feed}/v2/package/{id}/{version}",
            axum::routing::delete(nuget_api::unlist).post(nuget_api::relist),
        )
        .route("/api/v1/whoami", get(admin_api::whoami))
        // Margen sobre el tamaño del paquete para las cabeceras multipart; el límite exacto
        // del paquete lo aplica el staging.
        .layer(DefaultBodyLimit::max(
            max_package_bytes as usize + 64 * 1024,
        ))
        // `layer` (no `route_layer`): también cubre rutas inexistentes, así que nada responde
        // sin autenticar y una ruta nueva queda protegida por defecto.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ))
        // Las trazas HTTP registran método y ruta, nunca cabeceras: los tokens no llegan al log.
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Limpieza periódica de staging y blobs huérfanos (ADR-006). La primera pasada es inmediata,
/// así que también recupera lo que dejó una caída anterior.
pub async fn gc_loop(store: Arc<Store>, interval: Duration, grace: Duration) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        match store.gc(grace).await {
            Ok(report) if report.staging_removed + report.orphan_blobs_removed > 0 => {
                tracing::info!(
                    staging = report.staging_removed,
                    orphan_blobs = report.orphan_blobs_removed,
                    "limpieza completada"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "fallo en la limpieza"),
        }
    }
}

/// Rellena los metadatos de versiones publicadas antes de la migración 0003 leyendo el
/// `.nuspec` de su blob. Es idempotente: sin versiones pendientes no hace nada.
pub async fn backfill_metadata(store: &Store) -> Result<usize, onepack_storage::StoreError> {
    let mut filled = 0;
    for missing in store.versions_missing_metadata().await? {
        let blob = store
            .blobs()
            .open_blob(&missing.blob_sha256)
            .await?
            .into_std()
            .await;
        let manifest = tokio::task::spawn_blocking(move || onepack_nuget::read_package_from(blob))
            .await
            .map_err(|e| onepack_storage::StoreError::Io(std::io::Error::other(e)))?;
        match manifest {
            Ok(m) => {
                let metadata = onepack_nuget::v3::catalog_metadata(&m).to_string();
                store
                    .set_metadata(
                        missing.version_id,
                        &metadata,
                        &onepack_nuget::v3::search_text(&m),
                    )
                    .await?;
                filled += 1;
            }
            Err(e) => tracing::error!(
                version_id = missing.version_id,
                error = %e,
                "no se pudieron leer los metadatos del paquete"
            ),
        }
    }
    Ok(filled)
}
