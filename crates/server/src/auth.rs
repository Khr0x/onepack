//! HTTP authentication (ADR-011).
//!
//! A single middleware protects **all** routes, including ones that do not exist: a
//! new route is protected by default. Each surface accepts only its mechanisms:
//!
//! | Surface | Mechanism |
//! |---|---|
//! | `/api/...` (administration) | `Authorization: Bearer <token>` |
//! | NuGet (read and publish) | `X-NuGet-ApiKey: <token>` or `Authorization: Basic` with the token as password |
//!
//! Per-feed authorization is decided by `AuthContext` in each handler.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use onepack_storage::AuthOutcome;

use crate::AppState;
use crate::nuget_api::ApiError;

/// Maximum frequency at which a token's `last_used_at` is written (ADR-010).
const TOUCH_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Admin,
    NuGet,
}

impl Surface {
    pub fn of(path: &str) -> Self {
        if path == "/api" || path.starts_with("/api/") {
            Self::Admin
        } else {
            Self::NuGet
        }
    }
}

/// Remembers when each token's use was last recorded.
#[derive(Default)]
pub struct TouchTracker(Mutex<HashMap<String, Instant>>);

impl TouchTracker {
    fn should_touch(&self, token_id: &str) -> bool {
        let mut seen = self.0.lock().expect("mutex not poisoned");
        let now = Instant::now();
        match seen.get(token_id) {
            Some(last) if now.duration_since(*last) < TOUCH_INTERVAL => false,
            _ => {
                seen.insert(token_id.to_owned(), now);
                true
            }
        }
    }
}

fn credential(surface: Surface, headers: &HeaderMap) -> Option<String> {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    match surface {
        Surface::Admin => authorization
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|t| t.trim().to_owned()),
        Surface::NuGet => {
            if let Some(key) = headers.get("X-NuGet-ApiKey").and_then(|v| v.to_str().ok()) {
                return Some(key.trim().to_owned());
            }
            let encoded = authorization?.strip_prefix("Basic ")?;
            let decoded = String::from_utf8(STANDARD.decode(encoded.trim()).ok()?).ok()?;
            // The user is informational; the token goes in the password.
            decoded
                .split_once(':')
                .map(|(_, password)| password.to_owned())
        }
    }
}

pub fn unauthorized(surface: Surface) -> Response {
    ApiError::Unauthenticated.render(surface)
}

pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    // Health probes: no credential and no data (Phase 7).
    if crate::ops::is_probe(req.uri().path()) {
        return next.run(req).await;
    }
    let surface = Surface::of(req.uri().path());
    let Some(token) = credential(surface, req.headers()) else {
        return unauthorized(surface);
    };

    match state.store.authenticate(&token).await {
        Ok(AuthOutcome::Valid(ctx)) => {
            if let Err(retry_after) = state.principal_limiter.check(ctx.principal.id) {
                tracing::warn!(principal = %ctx.principal.name, "per-principal rate limit reached");
                return ApiError::RateLimited(retry_after).render(surface);
            }
            if state.touched.should_touch(&ctx.token_id) {
                let store = state.store.clone();
                let id = ctx.token_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = store.touch_token(&id).await {
                        tracing::warn!(error = %e, "could not record token usage");
                    }
                });
            }
            req.extensions_mut().insert(Arc::new(ctx));
            next.run(req).await
        }
        Ok(AuthOutcome::Invalid(failure)) => {
            tracing::info!(
                reason = failure.reason(),
                token_id = failure.token_id(),
                "authentication rejected"
            );
            state.store.record_auth_failure(&failure).await;
            unauthorized(surface)
        }
        Err(e) => {
            tracing::error!(error = %e, "authentication error");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn surfaces() {
        assert_eq!(Surface::of("/api/v1/whoami"), Surface::Admin);
        assert_eq!(Surface::of("/api"), Surface::Admin);
        assert_eq!(Surface::of("/apis"), Surface::NuGet);
        assert_eq!(Surface::of("/nuget/x/v3/index.json"), Surface::NuGet);
    }

    #[test]
    fn each_surface_only_accepts_its_mechanisms() {
        let bearer = headers(&[("authorization", "Bearer opk_t")]);
        let basic = headers(&[(
            "authorization",
            &format!("Basic {}", STANDARD.encode("ci:opk_t")),
        )]);
        let api_key = headers(&[("x-nuget-apikey", "opk_t")]);

        assert_eq!(
            credential(Surface::Admin, &bearer).as_deref(),
            Some("opk_t")
        );
        assert_eq!(credential(Surface::Admin, &basic), None);
        assert_eq!(credential(Surface::Admin, &api_key), None);

        assert_eq!(credential(Surface::NuGet, &basic).as_deref(), Some("opk_t"));
        assert_eq!(
            credential(Surface::NuGet, &api_key).as_deref(),
            Some("opk_t")
        );
        assert_eq!(credential(Surface::NuGet, &bearer), None);
    }

    #[test]
    fn malformed_basic_is_ignored() {
        for value in [
            "Basic",
            "Basic !!!",
            &format!("Basic {}", STANDARD.encode("sin-dos-puntos")),
        ] {
            assert_eq!(
                credential(Surface::NuGet, &headers(&[("authorization", value)])),
                None
            );
        }
    }
}
