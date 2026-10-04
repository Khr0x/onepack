//! API administrativa `/api/v1` (Bearer, ADR-011): identidad y disponibilidad de versiones.
//! La gestión completa llega en la Fase 6.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use onepack_core::{Access, AuthContext};
use onepack_storage::{StoreError, VersionChange};
use serde_json::json;

use crate::AppState;
use crate::auth::Surface;
use crate::nuget_api::{ApiError, authorized_feed, id_key, version_key};

/// Identidad y permisos de la credencial usada. Útil para diagnosticar (`onepack doctor`).
pub async fn whoami(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
) -> Response {
    let feeds = match state.store.feeds().await {
        Ok(feeds) => feeds,
        Err(e) => {
            tracing::error!(error = %e, "no se pudieron leer los feeds");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let grants: Vec<_> = auth
        .grants
        .iter()
        .filter_map(|g| {
            let feed = feeds.iter().find(|f| f.id == g.feed_id)?;
            Some(json!({
                "feed": feed.name.as_str(),
                "role": g.role.as_str(),
                "publish_patterns": g.publish_patterns.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
            }))
        })
        .collect();
    Json(json!({
        "principal": auth.principal.name.as_str(),
        "kind": auth.principal.kind.as_str(),
        "administrator": auth.principal.is_admin,
        "token_id": auth.token_id,
        "grants": grants,
    }))
    .into_response()
}

/// Error de la API administrativa: el mismo `ApiError`, en JSON.
pub struct AdminError(ApiError);

impl From<ApiError> for AdminError {
    fn from(e: ApiError) -> Self {
        Self(e)
    }
}

impl From<StoreError> for AdminError {
    fn from(e: StoreError) -> Self {
        Self(e.into())
    }
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        self.0.render(Surface::Admin)
    }
}

type AdminResult<T> = Result<T, AdminError>;

/// Longitud máxima del motivo de un bloqueo.
const MAX_REASON_CHARS: usize = 500;

/// Estado de una versión. Cualquiera con lectura la ve; el motivo de un bloqueo solo lo ve
/// quien puede mantener el feed.
pub async fn package_version(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, version)): Path<(String, String, String)>,
) -> AdminResult<Response> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let (id, version) = (id_key(&id)?, version_key(&version)?);
    let v = state
        .store
        .version(&feed, &id, &version)
        .await?
        .ok_or(ApiError::NotFound)?;
    let reason = if auth.check_feed(feed.id, Access::Maintain).is_ok() {
        state.store.blocked_reason(&feed, &id, &version).await?
    } else {
        None
    };
    Ok(Json(json!({
        "feed": feed.name.as_str(),
        "id": v.package_id,
        "version": v.version,
        "listed": v.listed,
        "availability": if v.blocked { "blocked" } else { "available" },
        "blocked_reason": reason,
        "sha256": v.blob_sha256,
        "size": v.size,
        "published_at": v.published_at,
    }))
    .into_response())
}

pub async fn block(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, version)): Path<(String, String, String)>,
    body: Bytes,
) -> AdminResult<Response> {
    set_blocked(&state, &auth, &feed_name, &id, &version, true, &body).await
}

pub async fn unblock(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, version)): Path<(String, String, String)>,
    body: Bytes,
) -> AdminResult<Response> {
    set_blocked(&state, &auth, &feed_name, &id, &version, false, &body).await
}

/// Bloqueo y desbloqueo (ADR-013): rol Maintainer y motivo obligatorio, que se audita.
async fn set_blocked(
    state: &AppState,
    auth: &AuthContext,
    feed_name: &str,
    id: &str,
    version: &str,
    blocked: bool,
    body: &[u8],
) -> AdminResult<Response> {
    let feed = authorized_feed(state, auth, feed_name, Access::Maintain).await?;
    let (id, version) = (id_key(id)?, version_key(version)?);
    let reason = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("reason")?.as_str().map(|r| r.trim().to_owned()))
        .filter(|r| !r.is_empty() && r.chars().count() <= MAX_REASON_CHARS)
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "el cuerpo debe ser JSON con \"reason\": un motivo de 1 a {MAX_REASON_CHARS} caracteres"
            ))
        })?;
    let change = state
        .store
        .set_blocked(&feed, &id, &version, blocked, &reason, &auth.actor())
        .await?;
    if change == VersionChange::NotFound {
        return Err(ApiError::NotFound.into());
    }
    if change == VersionChange::Changed {
        tracing::info!(
            feed = %feed.name,
            package = %format!("{id}@{version}"),
            blocked,
            "disponibilidad cambiada"
        );
    }
    Ok(Json(json!({
        "feed": feed.name.as_str(),
        "id": id,
        "version": version,
        "availability": if blocked { "blocked" } else { "available" },
        "changed": change == VersionChange::Changed,
    }))
    .into_response())
}
