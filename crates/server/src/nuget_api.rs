//! Superficie compatible con NuGet V3 (ADR-009): contenido, registros, búsqueda,
//! autocompletado y publicación. Los documentos del protocolo los genera `onepack_nuget::v3`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::multipart::MultipartError;
use axum::extract::{Extension, Multipart, Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use hyper::ext::ReasonPhrase;
use onepack_core::{Access, AuthContext, Denial, Feed, NewVersion, PublishError, QuotaKind};
use onepack_nuget::v3::{self, FeedUrls, SearchQuery};
use onepack_nuget::{NuGetVersion, PackageError, PackageId, read_nuspec_from, read_package_from};
use onepack_storage::{StagingError, StoreError, VersionChange};
use serde_json::json;
use tokio_util::io::ReaderStream;

use crate::AppState;
use crate::auth::Surface;

pub enum ApiError {
    /// Sin credencial válida.
    Unauthenticated,
    /// La operación requiere el rol Administrator.
    AdminRequired,
    /// Error con estado y código propios (validación, conflictos de la API administrativa).
    Coded(StatusCode, &'static str, String),
    NotFound,
    Forbidden(Denial),
    BadRequest(String),
    Conflict(String),
    PayloadTooLarge,
    /// El paquete no supera la inspección (ADR-014).
    Package(PackageError),
    Quota(QuotaKind),
    /// La versión está bloqueada (ADR-013).
    Blocked,
    UploadTimeout,
    /// No quedan plazas de subida.
    Busy,
    RateLimited(Duration),
    /// Modo mantenimiento (ADR-017): no se aceptan mutaciones.
    Maintenance,
    InsufficientStorage,
    Internal(String),
}

/// Frase de estado de las descargas bloqueadas. Los clientes .NET la muestran tal cual
/// ("Response status code does not indicate success: 410 (...)"), así que es lo que verá
/// quien ejecute `restore`. Solo ASCII.
const BLOCKED_REASON_PHRASE: &[u8] = b"PACKAGE_BLOCKED - version blocked by the registry";

impl ApiError {
    /// Estado HTTP, código estable y mensaje.
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "AUTH_REQUIRED",
                "a valid credential is required; it may be missing, expired or revoked".to_owned(),
            ),
            Self::AdminRequired => (
                StatusCode::FORBIDDEN,
                "AUTH_ADMIN_REQUIRED",
                "the operation requires the Administrator role".to_owned(),
            ),
            Self::Coded(status, code, message) => (*status, *code, message.clone()),
            Self::NotFound | Self::Forbidden(Denial::NotFound) => {
                (StatusCode::NOT_FOUND, "NOT_FOUND", "not found".to_owned())
            }
            Self::Forbidden(d @ Denial::MissingScope(access)) => (
                StatusCode::FORBIDDEN,
                d.code(),
                format!(
                    "the credential is valid, but lacks the {} permission on this feed",
                    access.scope()
                ),
            ),
            Self::Forbidden(d @ Denial::PrefixDenied) => (
                StatusCode::FORBIDDEN,
                d.code(),
                "the credential cannot publish this package id on this feed".to_owned(),
            ),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", m.clone()),
            Self::Conflict(m) => (StatusCode::CONFLICT, "PACKAGE_VERSION_EXISTS", m.clone()),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "PACKAGE_TOO_LARGE",
                "the package exceeds the maximum allowed size".to_owned(),
            ),
            Self::Package(e) => {
                let status = if e.code() == "PACKAGE_LIMIT_EXCEEDED" {
                    StatusCode::PAYLOAD_TOO_LARGE
                } else {
                    StatusCode::BAD_REQUEST
                };
                let message = e.to_string();
                let message = message
                    .strip_prefix(&format!("{}: ", e.code()))
                    .unwrap_or(&message)
                    .to_owned();
                (status, e.code(), message)
            }
            Self::Quota(kind) => {
                let (status, what) = match kind {
                    QuotaKind::Versions => (StatusCode::FORBIDDEN, "number of versions"),
                    QuotaKind::Storage => (StatusCode::PAYLOAD_TOO_LARGE, "storage"),
                };
                (
                    status,
                    kind.code(),
                    format!("the feed reached its {what} quota; the package was not published"),
                )
            }
            Self::Blocked => (
                StatusCode::GONE,
                "PACKAGE_BLOCKED",
                "this version is blocked by the registry and cannot be downloaded; \
                 pick another version or ask whoever maintains the feed"
                    .to_owned(),
            ),
            Self::UploadTimeout => (
                StatusCode::REQUEST_TIMEOUT,
                "UPLOAD_TIMEOUT",
                "the upload exceeded the maximum time".to_owned(),
            ),
            Self::Busy => (
                StatusCode::SERVICE_UNAVAILABLE,
                "UPLOADS_BUSY",
                "the server is handling the maximum number of concurrent uploads; retry".to_owned(),
            ),
            Self::RateLimited(_) => (
                StatusCode::TOO_MANY_REQUESTS,
                "RATE_LIMITED",
                "too many requests; retry later".to_owned(),
            ),
            Self::Maintenance => (
                StatusCode::SERVICE_UNAVAILABLE,
                "MAINTENANCE",
                "the registry is under maintenance (e.g. a backup); reads are still available"
                    .to_owned(),
            ),
            Self::InsufficientStorage => {
                tracing::error!("storage full");
                (
                    StatusCode::INSUFFICIENT_STORAGE,
                    "STORAGE_FULL",
                    "the server is out of space; the package was not published".to_owned(),
                )
            }
            Self::Internal(m) => {
                tracing::error!(error = %m, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "internal error".to_owned(),
                )
            }
        }
    }

    /// Respuesta en el formato de cada superficie: texto `CÓDIGO: mensaje` para los clientes
    /// NuGet, que lo muestran tal cual, y JSON para `/api`.
    pub fn render(self, surface: Surface) -> Response {
        let retry_after = match &self {
            Self::RateLimited(wait) => Some(wait.as_secs().max(1)),
            Self::Busy => Some(5),
            Self::Maintenance => Some(crate::ops::MAINTENANCE_RETRY_SECS),
            _ => None,
        };
        let blocked = matches!(self, Self::Blocked);
        let unauthenticated = matches!(self, Self::Unauthenticated);
        let (status, code, message) = self.parts();
        crate::ops::metrics().record_error(code);
        let mut res = match surface {
            Surface::NuGet if status == StatusCode::NOT_FOUND => (status, message).into_response(),
            Surface::NuGet => (status, format!("{code}: {message}")).into_response(),
            Surface::Admin => (
                status,
                Json(json!({ "error": {
                    "code": code,
                    "message": message,
                    "action": suggested_action(code),
                    "request_id": crate::request_id::current(),
                } })),
            )
                .into_response(),
        };
        if unauthenticated {
            let challenge = match surface {
                Surface::Admin => "Bearer realm=\"onepack\"",
                Surface::NuGet => "Basic realm=\"onepack\"",
            };
            res.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static(challenge),
            );
        }
        if let Some(secs) = retry_after {
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        if blocked {
            res.extensions_mut()
                .insert(ReasonPhrase::from_static(BLOCKED_REASON_PHRASE));
        }
        res
    }
}

/// Qué puede hacer quien recibe cada código (ADR-015). Nunca incluye datos de la petición.
pub fn suggested_action(code: &str) -> Option<&'static str> {
    Some(match code {
        "AUTH_REQUIRED" => {
            "log in with `onepack login` or check that the token has not expired or been revoked"
        }
        "AUTH_SCOPE_MISSING" => {
            "ask an administrator for a sufficient role: `onepack grant add --principal <principal> --feed <feed> --role <role>`"
        }
        "AUTH_PREFIX_DENIED" => {
            "ask an administrator to extend the grant's publish patterns (`--publish-pattern`)"
        }
        "AUTH_ADMIN_REQUIRED" => "use a credential of a principal with the Administrator role",
        "NOT_FOUND" | "FEED_NOT_FOUND" => {
            "check the name; if it exists, your credential may not have access"
        }
        "PACKAGE_VERSION_EXISTS" => {
            "publish a new version: versions are immutable (for the same file, use --skip-existing-identical)"
        }
        "FEED_QUOTA_VERSIONS" | "FEED_QUOTA_STORAGE" => {
            "ask an administrator to raise the quota: `onepack feed configure`"
        }
        "PACKAGE_BLOCKED" => "pick another version or ask whoever maintains the feed",
        "RATE_LIMITED" | "UPLOADS_BUSY" | "MAINTENANCE" => "retry after the Retry-After time",
        "PACKAGE_INVALID" | "PACKAGE_UNSAFE_PATH" => {
            "build the package with `dotnet pack` and check its contents"
        }
        "PACKAGE_LIMIT_EXCEEDED" | "PACKAGE_TOO_LARGE" => {
            "shrink the package or ask an administrator to raise the server limits"
        }
        "INVALID_REQUEST" | "INVALID_NAME" | "INVALID_CURSOR" => "check the request parameters",
        "LAST_ADMIN" => "create another administrator first",
        "INTERNAL" | "STORAGE_FULL" => {
            "contact whoever operates the server and give them the request_id"
        }
        _ => return None,
    })
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        self.render(Surface::NuGet)
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::StorageFull {
            Self::InsufficientStorage
        } else {
            Self::Internal(e.to_string())
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Io(io) => io.into(),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<StagingError> for ApiError {
    fn from(e: StagingError) -> Self {
        match e {
            StagingError::TooLarge { .. } => Self::PayloadTooLarge,
            StagingError::Io(io) => io.into(),
        }
    }
}

impl From<MultipartError> for ApiError {
    fn from(e: MultipartError) -> Self {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            Self::PayloadTooLarge
        } else {
            Self::BadRequest(format!("invalid multipart: {e}"))
        }
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// Única forma de obtener un `Feed` en los handlers: resuelve y autoriza a la vez. Un feed
/// inexistente y uno sin permiso de lectura responden igual (404) para no revelar cuál existe.
pub(crate) async fn authorized_feed(
    state: &AppState,
    auth: &AuthContext,
    name: &str,
    access: Access,
) -> ApiResult<Feed> {
    let feed = state.store.feed(name).await?.ok_or(ApiError::NotFound)?;
    if let Err(denial) = auth.check_feed(feed.id, access) {
        if access != Access::Read {
            let action = format!("feed.{}", access.scope());
            let visible = denial != Denial::NotFound;
            state
                .store
                .record_denied(
                    &action,
                    &auth.actor(),
                    visible.then_some(&feed),
                    None,
                    denial.code(),
                )
                .await;
        }
        return Err(ApiError::Forbidden(denial));
    }
    Ok(feed)
}

/// Convierte segmentos de URL en claves de identidad. Cualquier valor inválido es un 404:
/// nunca llega a la base ni al sistema de archivos texto sin validar.
pub(crate) fn id_key(raw: &str) -> ApiResult<String> {
    PackageId::parse(raw)
        .map(|id| id.identity())
        .map_err(|_| ApiError::NotFound)
}

pub(crate) fn version_key(raw: &str) -> ApiResult<String> {
    NuGetVersion::parse(raw)
        .map(|v| v.identity())
        .map_err(|_| ApiError::NotFound)
}

fn blocking_err(e: tokio::task::JoinError) -> ApiError {
    ApiError::Internal(e.to_string())
}

pub async fn service_index(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path(feed_name): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let base = format!("{}/nuget/{}", state.public_url, feed.name);
    let resource =
        |id: String, ty: &str, comment: &str| json!({ "@id": id, "@type": ty, "comment": comment });
    let mut resources = vec![
        resource(
            format!("{base}/v3/flat/"),
            "PackageBaseAddress/3.0.0",
            "Package content: versions, .nupkg and .nuspec.",
        ),
        resource(
            format!("{base}/v3/registration/"),
            "RegistrationsBaseUrl/3.6.0",
            "Package metadata, including SemVer 2.0.0 versions.",
        ),
        resource(
            format!("{base}/v2/package"),
            "PackagePublish/2.0.0",
            "Publish, unlist and relist.",
        ),
    ];
    // Mismo endpoint bajo los tipos que buscan las distintas versiones del cliente; 3.5.0
    // añade el filtro `packageType`, que también se implementa.
    for ty in [
        "SearchQueryService",
        "SearchQueryService/3.0.0-beta",
        "SearchQueryService/3.0.0-rc",
        "SearchQueryService/3.5.0",
    ] {
        resources.push(resource(format!("{base}/v3/query"), ty, "Package search."));
    }
    for ty in [
        "SearchAutocompleteService",
        "SearchAutocompleteService/3.0.0-beta",
        "SearchAutocompleteService/3.0.0-rc",
        "SearchAutocompleteService/3.5.0",
    ] {
        resources.push(resource(
            format!("{base}/v3/autocomplete"),
            ty,
            "Id and version autocomplete.",
        ));
    }
    Ok(Json(json!({ "version": "3.0.0", "resources": resources })))
}

fn feed_urls(state: &AppState, feed: &Feed) -> FeedUrls {
    FeedUrls::new(&format!("{}/nuget/{}", state.public_url, feed.name))
}

pub async fn registration_index(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let versions = state.store.versions(&feed, &id_key(&id)?).await?;
    v3::registration_index(&feed_urls(&state, &feed), &versions)
        .map(Json)
        .ok_or(ApiError::NotFound)
}

pub async fn registration_page(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, lower, upper)): Path<(String, String, String, String)>,
) -> ApiResult<impl IntoResponse> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let upper = upper.strip_suffix(".json").ok_or(ApiError::NotFound)?;
    let versions = state.store.versions(&feed, &id_key(&id)?).await?;
    v3::registration_page(&feed_urls(&state, &feed), &versions, &lower, upper)
        .map(Json)
        .ok_or(ApiError::NotFound)
}

pub async fn registration_leaf(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, leaf)): Path<(String, String, String)>,
) -> ApiResult<impl IntoResponse> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let version = leaf.strip_suffix(".json").ok_or(ApiError::NotFound)?;
    let published = state
        .store
        .version(&feed, &id_key(&id)?, &version_key(version)?)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(v3::registration_leaf(
        &feed_urls(&state, &feed),
        &published,
    )))
}

/// Parámetros comunes de búsqueda y autocompletado. Valores inválidos se tratan como ausentes.
struct SearchParams {
    query: SearchQuery,
    prerelease: bool,
    semver2: bool,
}

impl SearchParams {
    fn parse(params: &HashMap<String, String>) -> Self {
        let get = |key: &str| {
            params
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.as_str())
        };
        let number =
            |key: &str, default: usize| get(key).and_then(|v| v.parse().ok()).unwrap_or(default);
        Self {
            query: SearchQuery {
                q: get("q").unwrap_or_default().to_owned(),
                skip: number("skip", 0),
                take: number("take", v3::DEFAULT_TAKE),
                package_type: get("packageType").map(str::to_owned),
            },
            prerelease: get("prerelease").is_some_and(|v| v.eq_ignore_ascii_case("true")),
            // Sin `semVerLevel`, solo versiones compatibles con SemVer 1.0.0.
            semver2: get("semVerLevel")
                .and_then(|v| NuGetVersion::parse(v).ok())
                .is_some_and(|v| {
                    v.precedence_cmp(&NuGetVersion::parse("2.0.0").expect("valid version"))
                        .is_ge()
                }),
        }
    }
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path(feed_name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let p = SearchParams::parse(&params);
    let index = state
        .search
        .get(&state.store, &feed, p.prerelease, p.semver2)
        .await?;
    Ok(Json(v3::search(
        &feed_urls(&state, &feed),
        &index,
        &p.query,
    )))
}

pub async fn autocomplete(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path(feed_name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let p = SearchParams::parse(&params);
    let index = state
        .search
        .get(&state.store, &feed, p.prerelease, p.semver2)
        .await?;
    match params.get("id") {
        Some(id) => {
            let key = PackageId::parse(id)
                .map(|id| id.identity())
                .unwrap_or_default();
            Ok(Json(v3::autocomplete_versions(index.versions_of(&key))))
        }
        None => Ok(Json(v3::autocomplete_ids(&index, &p.query))),
    }
}

/// Unlist (`DELETE`) y relist (`POST`) de `PackagePublish`. Requieren poder publicar ese id
/// (rol Publisher y patrones del grant) y no afectan a la descarga (ADR-013).
pub(crate) async fn set_listed(
    state: &AppState,
    auth: &AuthContext,
    feed_name: &str,
    id: &str,
    version: &str,
    listed: bool,
) -> ApiResult<()> {
    let feed = authorized_feed(state, auth, feed_name, Access::Publish).await?;
    let (id, version) = (id_key(id)?, version_key(version)?);
    if let Err(denial) = auth.check_publish(feed.id, &id) {
        let action = if listed {
            "package.relist"
        } else {
            "package.unlist"
        };
        state
            .store
            .record_denied(
                action,
                &auth.actor(),
                Some(&feed),
                Some(&format!("{id}@{version}")),
                denial.code(),
            )
            .await;
        return Err(ApiError::Forbidden(denial));
    }
    let change = state
        .store
        .set_listed(&feed, &id, &version, listed, &auth.actor())
        .await?;
    if change == VersionChange::Changed {
        state.search.invalidate();
    }
    match change {
        VersionChange::NotFound => Err(ApiError::NotFound),
        VersionChange::Changed | VersionChange::Unchanged => Ok(()),
    }
}

pub async fn unlist(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, version)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    set_listed(&state, &auth, &feed_name, &id, &version, false).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn relist(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, version)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    set_listed(&state, &auth, &feed_name, &id, &version, true).await?;
    Ok(StatusCode::OK)
}

pub async fn flat_versions(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    // El flat container incluye versiones listadas y no listadas (ADR-013).
    let mut versions: Vec<NuGetVersion> = state
        .store
        .versions(&feed, &id_key(&id)?)
        .await?
        .iter()
        .filter_map(|v| NuGetVersion::parse(&v.version).ok())
        .collect();
    if versions.is_empty() {
        return Err(ApiError::NotFound);
    }
    versions.sort_by(|a, b| a.precedence_cmp(b));
    let keys: Vec<String> = versions.iter().map(NuGetVersion::identity).collect();
    Ok(Json(json!({ "versions": keys })))
}

pub async fn flat_file(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path((feed_name, id, version, file)): Path<(String, String, String, String)>,
) -> ApiResult<Response> {
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Read).await?;
    let id = id_key(&id)?;
    let version = version_key(&version)?;
    let file = file.to_lowercase();
    let published = state
        .store
        .version(&feed, &id, &version)
        .await?
        .ok_or(ApiError::NotFound)?;
    // Sigue apareciendo en los metadatos, pero ni el .nupkg ni el .nuspec se sirven (ADR-013).
    if published.blocked {
        tracing::info!(feed = %feed.name, package = %format!("{id}@{version}"), "download of a blocked version rejected");
        return Err(ApiError::Blocked);
    }

    if file == format!("{id}.{version}.nupkg") {
        let blob = state
            .store
            .blobs()
            .open_blob(&published.blob_sha256)
            .await?;
        Ok((
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                (header::CONTENT_LENGTH, published.size.to_string()),
            ],
            Body::from_stream(ReaderStream::new(blob)),
        )
            .into_response())
    } else if file == format!("{id}.nuspec") {
        let blob = state
            .store
            .blobs()
            .open_blob(&published.blob_sha256)
            .await?
            .into_std()
            .await;
        let limits = state.limits.inspection.clone();
        let nuspec = tokio::task::spawn_blocking(move || read_nuspec_from(blob, &limits))
            .await
            .map_err(blocking_err)?
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(([(header::CONTENT_TYPE, "application/xml")], nuspec).into_response())
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn publish(
    state: State<Arc<AppState>>,
    auth: Extension<Arc<AuthContext>>,
    feed_name: Path<String>,
    multipart: Multipart,
) -> ApiResult<StatusCode> {
    let result = publish_inner(state, auth, feed_name, multipart).await;
    crate::ops::metrics().record_upload(match &result {
        Ok(_) => "published",
        Err(ApiError::Conflict(_)) => "conflict",
        Err(_) => "rejected",
    });
    result
}

async fn publish_inner(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path(feed_name): Path<String>,
    mut multipart: Multipart,
) -> ApiResult<StatusCode> {
    // Rol comprobado antes de leer el cuerpo; el prefijo, al conocer el id del paquete.
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Publish).await?;
    // Plaza de subida antes de leer el cuerpo: una ráfaga recibe 503 en lugar de acumular
    // tareas y archivos en staging. Se libera al terminar el handler.
    let _upload = state
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::Busy)?;
    let _in_flight = crate::ops::metrics().upload_started();

    // Streaming a staging con hash incremental; nada de la subida queda en memoria (ADR-006).
    let receive = async {
        let mut field = multipart.next_field().await?.ok_or_else(|| {
            ApiError::BadRequest("the request does not contain the package".into())
        })?;
        let mut writer = state
            .store
            .blobs()
            .begin_staging(state.limits.max_package_bytes)
            .await?;
        while let Some(chunk) = field.chunk().await? {
            writer.write(&chunk).await?;
        }
        Ok::<_, ApiError>(writer.finish().await?)
    };
    let staged = tokio::time::timeout(state.limits.upload_timeout, receive)
        .await
        .map_err(|_| ApiError::UploadTimeout)??;

    // Inspección con concurrencia limitada, en el pool de hilos bloqueantes: no ocupa los
    // hilos que atienden descargas (ADR-014).
    let manifest = {
        let _inspection = state
            .inspections
            .acquire()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let file = std::fs::File::open(staged.path())?;
        let limits = state.limits.inspection.clone();
        tokio::task::spawn_blocking(move || read_package_from(file, &limits))
            .await
            .map_err(blocking_err)?
    };
    let manifest = match manifest {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(feed = %feed.name, code = e.code(), error = %e, "package rejected by inspection");
            state
                .store
                .record_denied(
                    "package.publish",
                    &auth.actor(),
                    Some(&feed),
                    None,
                    e.code(),
                )
                .await;
            return Err(ApiError::Package(e));
        }
    };

    let new = NewVersion {
        package_id: manifest.id.as_str().to_owned(),
        package_key: manifest.id.identity(),
        version: manifest.version.normalized(),
        version_key: manifest.version.identity(),
        full_version: manifest.version.full(),
        is_prerelease: manifest.version.is_prerelease(),
        is_semver2: manifest.version.is_semver2(),
        metadata: v3::catalog_metadata(&manifest).to_string(),
        search_text: v3::search_text(&manifest),
    };
    if let Err(denial) = auth.check_publish(feed.id, &new.package_key) {
        state
            .store
            .record_denied(
                "package.publish",
                &auth.actor(),
                Some(&feed),
                Some(&new.resource()),
                denial.code(),
            )
            .await;
        return Err(ApiError::Forbidden(denial));
    }

    let actor = auth.actor();
    match state.store.publish(&feed, &new, staged, Some(&actor)).await {
        Ok(()) => {
            state.search.invalidate();
            tracing::info!(feed = %feed.name, package = %new.resource(), "package published");
            Ok(StatusCode::CREATED)
        }
        Err(PublishError::Conflict { identical }) => Err(ApiError::Conflict(format!(
            "{} already exists in feed {} {}; published versions are immutable",
            new.resource(),
            feed.name,
            if identical {
                "with the same content"
            } else {
                "with different content"
            }
        ))),
        Err(PublishError::QuotaExceeded(kind)) => {
            tracing::warn!(feed = %feed.name, package = %new.resource(), code = kind.code(), "feed quota exceeded");
            Err(ApiError::Quota(kind))
        }
        Err(PublishError::StorageFull) => Err(ApiError::InsufficientStorage),
        Err(PublishError::Io(e)) => Err(e.into()),
        Err(PublishError::Database(e)) => Err(ApiError::Internal(e)),
    }
}
