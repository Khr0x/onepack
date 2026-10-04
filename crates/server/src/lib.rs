//! Servidor `onepackd`: composición de la aplicación HTTP.

pub mod nuget_api;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, put};
use onepack_storage::Store;
use tower_http::trace::TraceLayer;

pub struct AppState {
    /// URL pública explícita: base de todas las URLs absolutas (ADR-018).
    pub public_url: String,
    pub store: Arc<Store>,
    pub max_package_bytes: u64,
}

pub fn app(store: Arc<Store>, public_url: &str, max_package_bytes: u64) -> Router {
    let state = Arc::new(AppState {
        public_url: public_url.trim_end_matches('/').to_owned(),
        store,
        max_package_bytes,
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
        // Margen sobre el tamaño del paquete para las cabeceras multipart; el límite exacto
        // del paquete lo aplica el staging.
        .layer(DefaultBodyLimit::max(
            max_package_bytes as usize + 64 * 1024,
        ))
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
