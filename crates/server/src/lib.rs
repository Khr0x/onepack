//! Servidor `onepackd`: composición de la aplicación HTTP.

pub mod feed_store;
pub mod nuget_api;

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, put};
use tower_http::trace::TraceLayer;

use crate::feed_store::FeedStore;

pub struct Config<'a> {
    pub data_dir: &'a Path,
    /// URL pública explícita: base de todas las URLs absolutas (ADR-018).
    pub public_url: &'a str,
    pub feed: &'a str,
    pub max_package_bytes: usize,
}

pub struct AppState {
    pub public_url: String,
    pub feed: String,
    pub store: FeedStore,
}

pub async fn app(config: Config<'_>) -> std::io::Result<Router> {
    let state = Arc::new(AppState {
        public_url: config.public_url.trim_end_matches('/').to_owned(),
        feed: config.feed.to_owned(),
        store: FeedStore::open(config.data_dir, config.feed).await?,
    });

    Ok(Router::new()
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
        .layer(DefaultBodyLimit::max(config.max_package_bytes))
        .layer(TraceLayer::new_for_http())
        .with_state(state))
}
