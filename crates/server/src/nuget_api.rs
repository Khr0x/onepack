//! Superficie compatible con NuGet V3 (ADR-009). Se anuncian `PackageBaseAddress/3.0.0` y
//! `PackagePublish/2.0.0`; registros y búsqueda llegan en la Fase 3.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::multipart::MultipartError;
use axum::extract::{Extension, Multipart, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use onepack_core::{Access, AuthContext, Denial, Feed, NewVersion, PublishError};
use onepack_nuget::{NuGetVersion, PackageId, read_nuspec_from, read_package_from};
use onepack_storage::{StagingError, StoreError};
use serde_json::json;
use tokio_util::io::ReaderStream;

use crate::AppState;

pub enum ApiError {
    NotFound,
    Forbidden(Denial),
    BadRequest(String),
    Conflict(String),
    PayloadTooLarge,
    InsufficientStorage,
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::NotFound | Self::Forbidden(Denial::NotFound) => {
                (StatusCode::NOT_FOUND, "no encontrado".to_owned())
            }
            Self::Forbidden(d @ Denial::MissingScope(access)) => (
                StatusCode::FORBIDDEN,
                format!(
                    "{}: la credencial es válida, pero no tiene el permiso {} en este feed",
                    d.code(),
                    access.scope()
                ),
            ),
            Self::Forbidden(d @ Denial::PrefixDenied) => (
                StatusCode::FORBIDDEN,
                format!(
                    "{}: la credencial no puede publicar este id de paquete en este feed",
                    d.code()
                ),
            ),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::Conflict(m) => (StatusCode::CONFLICT, m),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "el paquete supera el tamaño máximo permitido".to_owned(),
            ),
            Self::InsufficientStorage => {
                tracing::error!("almacenamiento lleno");
                (
                    StatusCode::INSUFFICIENT_STORAGE,
                    "no queda espacio en el servidor; el paquete no se publicó".to_owned(),
                )
            }
            Self::Internal(m) => {
                tracing::error!(error = %m, "error interno");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "error interno".to_owned(),
                )
            }
        };
        (status, message).into_response()
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
            Self::BadRequest(format!("multipart inválido: {e}"))
        }
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// Única forma de obtener un `Feed` en los handlers: resuelve y autoriza a la vez. Un feed
/// inexistente y uno sin permiso de lectura responden igual (404) para no revelar cuál existe.
async fn authorized_feed(
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
fn id_key(raw: &str) -> ApiResult<String> {
    PackageId::parse(raw)
        .map(|id| id.identity())
        .map_err(|_| ApiError::NotFound)
}

fn version_key(raw: &str) -> ApiResult<String> {
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
    Ok(Json(json!({
        "version": "3.0.0",
        "resources": [
            {
                "@id": format!("{base}/v3/flat/"),
                "@type": "PackageBaseAddress/3.0.0",
                "comment": "Contenido de paquetes: versiones, .nupkg y .nuspec."
            },
            {
                "@id": format!("{base}/v2/package"),
                "@type": "PackagePublish/2.0.0",
                "comment": "Publicación de paquetes."
            }
        ]
    })))
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
        let nuspec = tokio::task::spawn_blocking(move || read_nuspec_from(blob))
            .await
            .map_err(blocking_err)?
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(([(header::CONTENT_TYPE, "application/xml")], nuspec).into_response())
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn publish(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<AuthContext>>,
    Path(feed_name): Path<String>,
    mut multipart: Multipart,
) -> ApiResult<impl IntoResponse> {
    // Rol comprobado antes de leer el cuerpo; el prefijo, al conocer el id del paquete.
    let feed = authorized_feed(&state, &auth, &feed_name, Access::Publish).await?;

    let mut field = multipart
        .next_field()
        .await?
        .ok_or_else(|| ApiError::BadRequest("la solicitud no contiene el paquete".into()))?;

    // Streaming a staging con hash incremental; nada de la subida queda en memoria (ADR-006).
    let mut writer = state
        .store
        .blobs()
        .begin_staging(state.max_package_bytes)
        .await?;
    while let Some(chunk) = field.chunk().await? {
        writer.write(&chunk).await?;
    }
    let staged = writer.finish().await?;

    let file = std::fs::File::open(staged.path())?;
    let manifest = tokio::task::spawn_blocking(move || read_package_from(file))
        .await
        .map_err(blocking_err)?
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let new = NewVersion {
        package_id: manifest.id.as_str().to_owned(),
        package_key: manifest.id.identity(),
        version: manifest.version.normalized(),
        version_key: manifest.version.identity(),
        full_version: manifest.version.full(),
        is_prerelease: manifest.version.is_prerelease(),
        is_semver2: manifest.version.is_semver2(),
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
            tracing::info!(feed = %feed.name, package = %new.resource(), "paquete publicado");
            Ok(StatusCode::CREATED)
        }
        Err(PublishError::Conflict { identical }) => Err(ApiError::Conflict(format!(
            "{} ya existe en el feed {} {}; las versiones publicadas son inmutables",
            new.resource(),
            feed.name,
            if identical {
                "con el mismo contenido"
            } else {
                "con contenido distinto"
            }
        ))),
        Err(PublishError::StorageFull) => Err(ApiError::InsufficientStorage),
        Err(PublishError::Io(e)) => Err(e.into()),
        Err(PublishError::Database(e)) => Err(ApiError::Internal(e)),
    }
}
