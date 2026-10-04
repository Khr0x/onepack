//! API administrativa `/api/v1` (Bearer, ADR-011, ADR-015). Los tipos de petición y
//! respuesta son los de `onepack_api_client`, compartidos con el CLI.
//!
//! Autorización: feeds y paquetes se rigen por el rol en el feed (como la superficie NuGet);
//! identidades, permisos, cuotas y auditoría requieren el rol Administrator.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use onepack_api_client as api;
use onepack_core::{
    Access, AuthContext, Feed, FeedName, FeedQuota, PrincipalKind, PrincipalName, PublishPattern,
    PublishedVersion, Role,
};
use onepack_nuget::v3::sort_versions;
use onepack_storage::{AuditQuery, DisableOutcome, FeedDetails, StoreError, VersionChange};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::AppState;
use crate::auth::Surface;
use crate::nuget_api::{ApiError, authorized_feed, id_key, set_listed, version_key};

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
type Params = Query<HashMap<String, String>>;

/// Longitud máxima del motivo de un bloqueo.
const MAX_REASON_CHARS: usize = 500;

fn invalid(code: &'static str, message: impl Into<String>) -> AdminError {
    ApiError::Coded(StatusCode::BAD_REQUEST, code, message.into()).into()
}

fn conflict(code: &'static str, message: impl Into<String>) -> AdminError {
    ApiError::Coded(StatusCode::CONFLICT, code, message.into()).into()
}

fn not_found(code: &'static str, message: impl Into<String>) -> AdminError {
    ApiError::Coded(StatusCode::NOT_FOUND, code, message.into()).into()
}

fn json<T: Serialize>(status: StatusCode, value: &T) -> Response {
    (status, Json(value)).into_response()
}

fn ok<T: Serialize>(value: &T) -> AdminResult<Response> {
    Ok(json(StatusCode::OK, value))
}

fn parse_body<T: DeserializeOwned>(body: &[u8]) -> AdminResult<T> {
    serde_json::from_slice(body)
        .map_err(|e| invalid("INVALID_REQUEST", format!("cuerpo JSON inválido: {e}")))
}

/// Identidades, permisos, cuotas y auditoría: solo administradores. El intento se audita.
async fn require_admin(state: &AppState, auth: &AuthContext, action: &str) -> AdminResult<()> {
    if auth.principal.is_admin {
        return Ok(());
    }
    state
        .store
        .record_denied(action, &auth.actor(), None, None, "AUTH_ADMIN_REQUIRED")
        .await;
    Err(ApiError::AdminRequired.into())
}

// ---------------------------------------------------------------------------------------------
// Paginación
// ---------------------------------------------------------------------------------------------

fn encode_cursor(key: &str) -> String {
    URL_SAFE_NO_PAD.encode(key)
}

fn decode_cursor(params: &HashMap<String, String>) -> AdminResult<Option<String>> {
    params
        .get("cursor")
        .map(|c| {
            URL_SAFE_NO_PAD
                .decode(c)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .ok_or_else(|| invalid("INVALID_CURSOR", "cursor inválido"))
        })
        .transpose()
}

fn page_size(params: &HashMap<String, String>) -> AdminResult<u32> {
    match params.get("limit") {
        None => Ok(api::DEFAULT_PAGE_SIZE),
        Some(v) => v
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=api::MAX_PAGE_SIZE).contains(n))
            .ok_or_else(|| {
                invalid(
                    "INVALID_REQUEST",
                    format!("limit debe estar entre 1 y {}", api::MAX_PAGE_SIZE),
                )
            }),
    }
}

/// Página de una lista ya ordenada y pequeña (feeds, principals, tokens, grants). El cursor
/// es la clave del último elemento devuelto.
fn paginate<T>(
    items: Vec<T>,
    params: &HashMap<String, String>,
    key: impl Fn(&T) -> String,
) -> AdminResult<api::Page<T>> {
    let limit = page_size(params)? as usize;
    let start = match decode_cursor(params)? {
        None => 0,
        Some(cursor) => {
            items
                .iter()
                .position(|i| key(i) == cursor)
                .ok_or_else(|| invalid("INVALID_CURSOR", "el cursor ya no es válido"))?
                + 1
        }
    };
    let more = items.len() > start + limit;
    let items: Vec<T> = items.into_iter().skip(start).take(limit).collect();
    let next_cursor = if more {
        items.last().map(|i| encode_cursor(&key(i)))
    } else {
        None
    };
    Ok(api::Page { items, next_cursor })
}

// ---------------------------------------------------------------------------------------------
// Identidad y capacidades
// ---------------------------------------------------------------------------------------------

pub async fn capabilities() -> Json<api::Capabilities> {
    Json(api::Capabilities {
        server_version: env!("CARGO_PKG_VERSION").to_owned(),
        api_version: api::API_VERSION,
        capabilities: api::capability::ALL
            .iter()
            .map(|c| (*c).to_owned())
            .collect(),
    })
}

/// Identidad y permisos de la credencial usada. Útil para diagnosticar (`onepack doctor`).
pub async fn whoami(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
) -> AdminResult<Response> {
    let feeds = state.store.feeds().await?;
    let grants = auth
        .grants
        .iter()
        .filter_map(|g| {
            let feed = feeds.iter().find(|f| f.id == g.feed_id)?;
            Some(api::GrantSummary {
                feed: feed.name.as_str().to_owned(),
                role: g.role.as_str().to_owned(),
                publish_patterns: g
                    .publish_patterns
                    .iter()
                    .map(|p| p.as_str().to_owned())
                    .collect(),
            })
        })
        .collect();
    ok(&api::WhoAmI {
        principal: auth.principal.name.as_str().to_owned(),
        kind: auth.principal.kind.as_str().to_owned(),
        administrator: auth.principal.is_admin,
        token_id: auth.token_id.clone(),
        token_expires_at: state.store.token_expires_at(&auth.token_id).await?,
        grants,
    })
}

// ---------------------------------------------------------------------------------------------
// Feeds
// ---------------------------------------------------------------------------------------------

fn feed_dto(feed: &Feed, d: FeedDetails) -> api::Feed {
    api::Feed {
        name: feed.name.as_str().to_owned(),
        created_at: d.created_at,
        versions: d.usage.versions,
        storage_bytes: d.usage.storage_bytes,
        max_storage_bytes: d.quota.max_storage_bytes,
        max_versions: d.quota.max_versions,
    }
}

/// Feeds visibles para la credencial: todos para un administrador, los de sus grants para
/// el resto.
pub async fn list_feeds(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Query(params): Params,
) -> AdminResult<Response> {
    let visible: Vec<Feed> = state
        .store
        .feeds()
        .await?
        .into_iter()
        .filter(|f| auth.check_feed(f.id, Access::Read).is_ok())
        .collect();
    let page = paginate(visible, &params, |f| f.name.as_str().to_owned())?;
    let mut items = Vec::with_capacity(page.items.len());
    for feed in &page.items {
        items.push(feed_dto(feed, state.store.feed_details(feed).await?));
    }
    ok(&api::Page {
        items,
        next_cursor: page.next_cursor,
    })
}

pub async fn create_feed(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    body: Bytes,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "feed.create").await?;
    let req: api::CreateFeed = parse_body(&body)?;
    let name = FeedName::parse(&req.name).map_err(|e| invalid("INVALID_NAME", e.to_string()))?;
    if state.store.feed(name.as_str()).await?.is_some() {
        return Err(conflict(
            "FEED_EXISTS",
            format!("ya existe el feed {}", name.as_str()),
        ));
    }
    let feed = state.store.create_feed(&name, &auth.actor()).await?;
    let details = state.store.feed_details(&feed).await?;
    Ok(json(StatusCode::CREATED, &feed_dto(&feed, details)))
}

pub async fn get_feed(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path(feed_name): Path<String>,
) -> AdminResult<Response> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let details = state.store.feed_details(&feed).await?;
    ok(&feed_dto(&feed, details))
}

/// Cuotas del feed (reemplazo completo: un campo ausente es sin límite).
pub async fn configure_feed(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path(feed_name): Path<String>,
    body: Bytes,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "feed.quota").await?;
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let req: api::ConfigureFeed = parse_body(&body)?;
    let quota = FeedQuota {
        max_storage_bytes: req.max_storage_bytes,
        max_versions: req.max_versions,
    };
    state
        .store
        .set_feed_quota(&feed, quota, &auth.actor())
        .await?;
    let details = state.store.feed_details(&feed).await?;
    ok(&feed_dto(&feed, details))
}

// ---------------------------------------------------------------------------------------------
// Principals
// ---------------------------------------------------------------------------------------------

async fn find_principal(state: &AppState, name: &str) -> AdminResult<onepack_core::Principal> {
    state.store.principal(name).await?.ok_or_else(|| {
        not_found(
            "PRINCIPAL_NOT_FOUND",
            format!("no existe el principal {name:?}"),
        )
    })
}

async fn principal_dto(state: &AppState, name: &str) -> AdminResult<api::Principal> {
    state
        .store
        .list_principals()
        .await?
        .into_iter()
        .find(|p| p.name == name)
        .map(|p| api::Principal {
            name: p.name,
            kind: p.kind,
            administrator: p.is_admin,
            disabled: p.disabled,
            created_at: p.created_at,
        })
        .ok_or_else(|| {
            not_found(
                "PRINCIPAL_NOT_FOUND",
                format!("no existe el principal {name:?}"),
            )
        })
}

pub async fn list_principals(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Query(params): Params,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "principal.list").await?;
    let items: Vec<api::Principal> = state
        .store
        .list_principals()
        .await?
        .into_iter()
        .map(|p| api::Principal {
            name: p.name,
            kind: p.kind,
            administrator: p.is_admin,
            disabled: p.disabled,
            created_at: p.created_at,
        })
        .collect();
    ok(&paginate(items, &params, |p| p.name.clone())?)
}

pub async fn create_principal(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    body: Bytes,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "principal.create").await?;
    let req: api::CreatePrincipal = parse_body(&body)?;
    let name =
        PrincipalName::parse(&req.name).map_err(|e| invalid("INVALID_NAME", e.to_string()))?;
    let kind = PrincipalKind::parse(&req.kind)
        .ok_or_else(|| invalid("INVALID_REQUEST", "kind debe ser user o service"))?;
    if state.store.principal(name.as_str()).await?.is_some() {
        return Err(conflict(
            "PRINCIPAL_EXISTS",
            format!("ya existe el principal {}", name.as_str()),
        ));
    }
    state
        .store
        .create_principal(&name, kind, req.administrator, &auth.actor())
        .await?;
    Ok(json(
        StatusCode::CREATED,
        &principal_dto(&state, name.as_str()).await?,
    ))
}

pub async fn disable_principal(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path(name): Path<String>,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "principal.disable").await?;
    let principal = find_principal(&state, &name).await?;
    match state
        .store
        .disable_principal(&principal, &auth.actor())
        .await?
    {
        DisableOutcome::LastAdmin => Err(conflict(
            "LAST_ADMIN",
            "no se puede desactivar al último administrador activo",
        )),
        DisableOutcome::Disabled | DisableOutcome::AlreadyDisabled => {
            ok(&principal_dto(&state, &name).await?)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------------------------

/// Tokens del más reciente al más antiguo; `?principal=` filtra. Nunca incluye secretos.
pub async fn list_tokens(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Query(params): Params,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "token.list").await?;
    let principal = match params.get("principal") {
        Some(name) => Some(find_principal(&state, name).await?),
        None => None,
    };
    let items: Vec<api::Token> = state
        .store
        .list_tokens(principal.as_ref())
        .await?
        .into_iter()
        .map(|t| api::Token {
            id: t.id,
            principal: t.principal,
            name: t.name,
            created_at: t.created_at,
            expires_at: t.expires_at,
            last_used_at: t.last_used_at,
            revoked_at: t.revoked_at,
        })
        .collect();
    ok(&paginate(items, &params, |t| t.id.clone())?)
}

pub async fn create_token(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    body: Bytes,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "token.create").await?;
    let req: api::CreateToken = parse_body(&body)?;
    if !(1..=3650).contains(&req.expires_in_days) {
        return Err(invalid(
            "INVALID_REQUEST",
            "expires_in_days debe estar entre 1 y 3650",
        ));
    }
    let principal = find_principal(&state, &req.principal).await?;
    let issued = state
        .store
        .create_token(
            &principal,
            req.name.as_deref(),
            i64::from(req.expires_in_days) * 86_400,
            &auth.actor(),
        )
        .await?;
    Ok(json(
        StatusCode::CREATED,
        &api::IssuedToken {
            id: issued.id,
            principal: principal.name.as_str().to_owned(),
            token: issued.token,
            expires_at: issued.expires_at,
        },
    ))
}

pub async fn revoke_token(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path(id): Path<String>,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "token.revoke").await?;
    if !state.store.revoke_token(&id, &auth.actor()).await? {
        return Err(not_found(
            "TOKEN_NOT_FOUND",
            format!("no existe un token activo con id {id:?}"),
        ));
    }
    let token = state
        .store
        .list_tokens(None)
        .await?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| not_found("TOKEN_NOT_FOUND", "token no encontrado"))?;
    ok(&api::Token {
        id: token.id,
        principal: token.principal,
        name: token.name,
        created_at: token.created_at,
        expires_at: token.expires_at,
        last_used_at: token.last_used_at,
        revoked_at: token.revoked_at,
    })
}

// ---------------------------------------------------------------------------------------------
// Grants
// ---------------------------------------------------------------------------------------------

/// `?principal=` y `?feed=` filtran.
pub async fn list_grants(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Query(params): Params,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "grant.list").await?;
    let principal_id = match params.get("principal") {
        Some(name) => Some(find_principal(&state, name).await?.id),
        None => None,
    };
    let feed_id = match params.get("feed") {
        Some(name) => Some(authorized_feed(&state, &auth, name, Access::Read).await?.id),
        None => None,
    };
    let items: Vec<api::Grant> = state
        .store
        .list_grants(principal_id, feed_id)
        .await?
        .into_iter()
        .map(|g| api::Grant {
            principal: g.principal,
            feed: g.feed,
            role: g.role,
            publish_patterns: g.publish_patterns,
        })
        .collect();
    ok(&paginate(items, &params, |g| {
        format!("{}\u{1f}{}", g.feed, g.principal)
    })?)
}

pub async fn set_grant(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, principal_name)): Path<(String, String)>,
    body: Bytes,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "grant.set").await?;
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let principal = find_principal(&state, &principal_name).await?;
    let req: api::SetGrant = parse_body(&body)?;
    let role = Role::parse(&req.role).ok_or_else(|| {
        invalid(
            "INVALID_REQUEST",
            "role debe ser reader, publisher o maintainer",
        )
    })?;
    let patterns = req
        .publish_patterns
        .iter()
        .map(|p| PublishPattern::parse(p))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| invalid("INVALID_REQUEST", e.to_string()))?;
    state
        .store
        .set_grant(&principal, &feed, role, &patterns, &auth.actor())
        .await?;
    ok(&api::Grant {
        principal: principal.name.as_str().to_owned(),
        feed: feed.name.as_str().to_owned(),
        role: role.as_str().to_owned(),
        publish_patterns: patterns.iter().map(|p| p.as_str().to_owned()).collect(),
    })
}

pub async fn remove_grant(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, principal_name)): Path<(String, String)>,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "grant.remove").await?;
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let principal = find_principal(&state, &principal_name).await?;
    if !state
        .store
        .remove_grant(&principal, &feed, &auth.actor())
        .await?
    {
        return Err(not_found(
            "GRANT_NOT_FOUND",
            format!(
                "{} no tiene acceso a {}",
                principal.name.as_str(),
                feed.name.as_str()
            ),
        ));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------------------------
// Paquetes
// ---------------------------------------------------------------------------------------------

fn version_dto(
    feed: &Feed,
    v: &PublishedVersion,
    blocked_reason: Option<String>,
) -> api::PackageVersion {
    api::PackageVersion {
        feed: feed.name.as_str().to_owned(),
        id: v.package_id.clone(),
        version: v.version.clone(),
        listed: v.listed,
        availability: if v.blocked { "blocked" } else { "available" }.to_owned(),
        blocked_reason,
        sha256: v.blob_sha256.clone(),
        size: v.size,
        published_at: v.published_at.clone(),
    }
}

/// Motivo de bloqueo, solo para quien puede mantener el feed.
async fn visible_reason(
    state: &AppState,
    auth: &AuthContext,
    feed: &Feed,
    v: &PublishedVersion,
) -> AdminResult<Option<String>> {
    if v.blocked && auth.check_feed(feed.id, Access::Maintain).is_ok() {
        Ok(state
            .store
            .blocked_reason(feed, &v.package_key, &v.version_key)
            .await?)
    } else {
        Ok(None)
    }
}

pub async fn list_packages(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path(feed_name): Path<String>,
    Query(params): Params,
) -> AdminResult<Response> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let limit = page_size(&params)?;
    let after = decode_cursor(&params)?;
    // Un elemento de más indica si hay otra página.
    let mut rows = state
        .store
        .list_packages(&feed, after.as_deref(), limit + 1)
        .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut versions = state.store.versions(&feed, &row.package_key).await?;
        sort_versions(&mut versions);
        items.push(api::PackageSummary {
            id: row.package_id.clone(),
            versions: versions.len() as u64,
            latest_version: versions
                .last()
                .map(|v| v.version.clone())
                .unwrap_or_default(),
        });
    }
    let next_cursor = if more {
        rows.last().map(|r| encode_cursor(&r.package_key))
    } else {
        None
    };
    ok(&api::Page { items, next_cursor })
}

/// Versiones de un paquete, de menor a mayor precedencia.
pub async fn package_versions(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id)): Path<(String, String)>,
) -> AdminResult<Response> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let mut versions = state.store.versions(&feed, &id_key(&id)?).await?;
    if versions.is_empty() {
        return Err(ApiError::NotFound.into());
    }
    sort_versions(&mut versions);
    let mut items = Vec::with_capacity(versions.len());
    for v in &versions {
        let reason = visible_reason(&state, &auth, &feed, v).await?;
        items.push(version_dto(&feed, v, reason));
    }
    ok(&items)
}

pub async fn package_version(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, version)): Path<(String, String, String)>,
) -> AdminResult<Response> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let v = state
        .store
        .version(&feed, &id_key(&id)?, &version_key(&version)?)
        .await?
        .ok_or(ApiError::NotFound)?;
    let reason = visible_reason(&state, &auth, &feed, &v).await?;
    ok(&version_dto(&feed, &v, reason))
}

async fn version_state(
    state: &AppState,
    feed: &Feed,
    id: &str,
    version: &str,
    changed: bool,
) -> AdminResult<Response> {
    let v = state
        .store
        .version(feed, &id_key(id)?, &version_key(version)?)
        .await?
        .ok_or(ApiError::NotFound)?;
    ok(&api::VersionState {
        feed: feed.name.as_str().to_owned(),
        id: v.package_id,
        version: v.version,
        listed: v.listed,
        availability: if v.blocked { "blocked" } else { "available" }.to_owned(),
        changed,
    })
}

/// Unlist y relist: mismas reglas que en el protocolo NuGet (Publisher y patrones).
async fn change_listing(
    state: &AppState,
    auth: &AuthContext,
    feed_name: &str,
    id: &str,
    version: &str,
    listed: bool,
) -> AdminResult<Response> {
    let feed = authorized_feed(state, auth, feed_name, Access::Read).await?;
    let before = state
        .store
        .version(&feed, &id_key(id)?, &version_key(version)?)
        .await?
        .map(|v| v.listed);
    set_listed(state, auth, feed_name, id, version, listed).await?;
    version_state(state, &feed, id, version, before != Some(listed)).await
}

pub async fn unlist(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, version)): Path<(String, String, String)>,
) -> AdminResult<Response> {
    change_listing(&state, &auth, &feed_name, &id, &version, false).await
}

pub async fn relist(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, version)): Path<(String, String, String)>,
) -> AdminResult<Response> {
    change_listing(&state, &auth, &feed_name, &id, &version, true).await
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
    let (id_k, version_k) = (id_key(id)?, version_key(version)?);
    let reason = serde_json::from_slice::<api::Reason>(body)
        .ok()
        .map(|r| r.reason.trim().to_owned())
        .filter(|r| !r.is_empty() && r.chars().count() <= MAX_REASON_CHARS)
        .ok_or_else(|| {
            invalid(
                "INVALID_REQUEST",
                format!(
                    "el cuerpo debe ser JSON con \"reason\": un motivo de 1 a {MAX_REASON_CHARS} caracteres"
                ),
            )
        })?;
    let change = state
        .store
        .set_blocked(&feed, &id_k, &version_k, blocked, &reason, &auth.actor())
        .await?;
    if change == VersionChange::NotFound {
        return Err(ApiError::NotFound.into());
    }
    if change == VersionChange::Changed {
        tracing::info!(
            feed = %feed.name,
            package = %format!("{id_k}@{version_k}"),
            blocked,
            "disponibilidad cambiada"
        );
    }
    version_state(state, &feed, id, version, change == VersionChange::Changed).await
}

// ---------------------------------------------------------------------------------------------
// Auditoría
// ---------------------------------------------------------------------------------------------

/// Eventos del más reciente al más antiguo. `?feed=` y `?action=` (prefijo) filtran.
pub async fn list_audit(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Query(params): Params,
) -> AdminResult<Response> {
    require_admin(&state, &auth, "audit.list").await?;
    let limit = page_size(&params)?;
    let before_id = decode_cursor(&params)?
        .map(|c| c.parse::<i64>())
        .transpose()
        .map_err(|_| invalid("INVALID_CURSOR", "cursor inválido"))?;
    let feed_id = match params.get("feed") {
        Some(name) => Some(authorized_feed(&state, &auth, name, Access::Read).await?.id),
        None => None,
    };
    let mut rows = state
        .store
        .list_audit(&AuditQuery {
            before_id,
            limit: limit + 1,
            feed_id,
            action_prefix: params.get("action").cloned(),
        })
        .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if more {
        rows.last().map(|r| encode_cursor(&r.id.to_string()))
    } else {
        None
    };
    let items = rows
        .into_iter()
        .map(|r| api::AuditEvent {
            id: r.id,
            occurred_at: r.occurred_at,
            actor: r.actor,
            action: r.action,
            feed: r.feed,
            resource: r.resource,
            outcome: r.outcome,
            detail: r.detail.and_then(|d| serde_json::from_str(&d).ok()),
        })
        .collect();
    ok(&api::Page { items, next_cursor })
}
