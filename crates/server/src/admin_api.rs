//! API administrativa `/api/v1` (Bearer, ADR-011). En la Fase 4 solo expone la identidad;
//! la gestión completa llega en la Fase 6.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use onepack_core::AuthContext;
use serde_json::json;

use crate::AppState;

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
