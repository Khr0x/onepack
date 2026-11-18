//! `onepackd` server: HTTP application composition.

pub mod admin_api;
pub mod auth;
pub mod limits;
pub mod nuget_api;
pub mod ops;
pub mod request_id;
pub mod search_cache;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{get, post, put};
use onepack_nuget::InspectionLimits;
use onepack_storage::Store;
use tokio::sync::Semaphore;
use tower_http::trace::TraceLayer;

pub use limits::{Limits, RateLimit};

pub struct AppState {
    /// Explicit public URL: base for all absolute URLs (ADR-018).
    pub public_url: String,
    pub store: Arc<Store>,
    pub limits: Limits,
    pub touched: auth::TouchTracker,
    pub uploads: Arc<Semaphore>,
    pub inspections: Semaphore,
    pub principal_limiter: limits::RateLimiter<i64>,
    pub ip_limiter: limits::RateLimiter<IpAddr>,
    pub search: search_cache::SearchCache,
}

pub fn app(store: Arc<Store>, public_url: &str, limits: Limits) -> Router {
    app_with_ops(store, public_url, limits).0
}

/// The application and the operation listener router (`/metrics`, probes), which share
/// state.
pub fn app_with_ops(store: Arc<Store>, public_url: &str, limits: Limits) -> (Router, Router) {
    let max_package_bytes = limits.max_package_bytes;
    let state = Arc::new(AppState {
        public_url: public_url.trim_end_matches('/').to_owned(),
        store,
        touched: auth::TouchTracker::default(),
        uploads: Arc::new(Semaphore::new(limits.max_concurrent_uploads.max(1))),
        inspections: Semaphore::new(limits.max_concurrent_inspections.max(1)),
        principal_limiter: limits::RateLimiter::new(limits.principal_rate),
        ip_limiter: limits::RateLimiter::new(limits.ip_rate),
        search: search_cache::SearchCache::default(),
        limits,
    });

    let router = Router::new()
        .route("/nuget/{feed}/v3/index.json", get(nuget_api::service_index))
        .route(
            "/nuget/{feed}/v3/flat/{id}/index.json",
            get(nuget_api::flat_versions),
        )
        .route(
            "/nuget/{feed}/v3/flat/{id}/{version}/{file}",
            get(nuget_api::flat_file),
        )
        // The NuGet client issues the PUT against the PackagePublish @id with a trailing slash.
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
        .route("/healthz", get(ops::healthz))
        .route("/readyz", get(ops::readyz))
        .route("/api/v1/capabilities", get(admin_api::capabilities))
        .route("/api/v1/whoami", get(admin_api::whoami))
        .route(
            "/api/v1/feeds",
            get(admin_api::list_feeds).post(admin_api::create_feed),
        )
        .route(
            "/api/v1/feeds/{feed}",
            get(admin_api::get_feed).patch(admin_api::configure_feed),
        )
        .route(
            "/api/v1/principals",
            get(admin_api::list_principals).post(admin_api::create_principal),
        )
        .route(
            "/api/v1/principals/{name}/disable",
            post(admin_api::disable_principal),
        )
        .route(
            "/api/v1/tokens",
            get(admin_api::list_tokens).post(admin_api::create_token),
        )
        .route("/api/v1/tokens/{id}/revoke", post(admin_api::revoke_token))
        .route("/api/v1/grants", get(admin_api::list_grants))
        .route(
            "/api/v1/feeds/{feed}/grants/{principal}",
            put(admin_api::set_grant).delete(admin_api::remove_grant),
        )
        .route(
            "/api/v1/feeds/{feed}/packages",
            get(admin_api::list_packages),
        )
        .route(
            "/api/v1/feeds/{feed}/packages/{id}",
            get(admin_api::package_versions),
        )
        .route(
            "/api/v1/feeds/{feed}/packages/{id}/{version}",
            get(admin_api::package_version),
        )
        .route(
            "/api/v1/feeds/{feed}/packages/{id}/{version}/unlist",
            post(admin_api::unlist),
        )
        .route(
            "/api/v1/feeds/{feed}/packages/{id}/{version}/relist",
            post(admin_api::relist),
        )
        .route(
            "/api/v1/feeds/{feed}/packages/{id}/{version}/block",
            post(admin_api::block),
        )
        .route(
            "/api/v1/feeds/{feed}/packages/{id}/{version}/unblock",
            post(admin_api::unblock),
        )
        .route("/api/v1/audit", get(admin_api::list_audit))
        // Margin over the package size for the multipart headers; the exact package
        // limit is enforced by staging.
        .layer(DefaultBodyLimit::max(
            max_package_bytes as usize + 64 * 1024,
        ))
        // Inside authentication: during maintenance, mutations receive 503 (ADR-017).
        .layer(middleware::from_fn_with_state(
            state.clone(),
            ops::maintenance_guard,
        ))
        // `layer` (not `route_layer`): it also covers nonexistent routes, so nothing responds
        // unauthenticated and a new route is protected by default.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ))
        // Before authenticating: it also throttles bursts of invalid credentials.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            limits::limit_by_ip,
        ))
        // HTTP traces record method and route, never headers: tokens do not reach the log.
        .layer(TraceLayer::new_for_http().make_span_with(
            |req: &axum::http::Request<axum::body::Body>| {
                tracing::info_span!(
                    "request",
                    method = %req.method(),
                    uri = %req.uri(),
                    request_id = %request_id::current().unwrap_or_default(),
                )
            },
        ))
        .layer(middleware::from_fn(ops::track))
        // The outermost one: IP-limit rejections also carry a request_id.
        .layer(middleware::from_fn(request_id::assign))
        .with_state(state.clone());
    (router, ops::ops_router(state))
}

/// Periodic cleanup of staging and orphaned blobs (ADR-006). The first pass is immediate,
/// so it also recovers what a previous crash left behind.
pub async fn gc_loop(store: Arc<Store>, interval: Duration, grace: Duration) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        // During a backup (ADR-017) nothing is deleted.
        match store.maintenance().await {
            Ok(Some(_)) => {
                tracing::info!("cleanup postponed: maintenance in progress");
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!(error = %e, "could not query maintenance mode");
                continue;
            }
        }
        match store.gc(grace).await {
            Ok(report) if report.staging_removed + report.orphan_blobs_removed > 0 => {
                tracing::info!(
                    staging = report.staging_removed,
                    orphan_blobs = report.orphan_blobs_removed,
                    "limpieza completada"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "cleanup failed"),
        }
    }
}

/// Fills in the metadata of versions published before migration 0003 by reading the
/// `.nuspec` from their blob. It is idempotent: with no pending versions it does nothing.
pub async fn backfill_metadata(
    store: &Store,
    limits: &InspectionLimits,
) -> Result<usize, onepack_storage::StoreError> {
    let mut filled = 0;
    for missing in store.versions_missing_metadata().await? {
        let blob = store
            .blobs()
            .open_blob(&missing.blob_sha256)
            .await?
            .into_std()
            .await;
        let limits = limits.clone();
        let manifest =
            tokio::task::spawn_blocking(move || onepack_nuget::read_package_from(blob, &limits))
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
                "could not read the package metadata"
            ),
        }
    }
    Ok(filled)
}
