//! Superficie compatible con NuGet V3 (ADR-009). En la Fase 1 solo se anuncian
//! `PackageBaseAddress/3.0.0` y `PackagePublish/2.0.0`.

use std::sync::Arc;

use axum::Json;
use axum::extract::multipart::MultipartError;
use axum::extract::{Multipart, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use onepack_nuget::package::read_nuspec_bytes;
use onepack_nuget::{NuGetVersion, PackageId, read_package};
use serde_json::json;

use crate::AppState;
use crate::feed_store::PublishError;

pub enum ApiError {
    NotFound,
    BadRequest(String),
    Conflict(String),
    PayloadTooLarge,
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "no encontrado".to_owned()),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::Conflict(m) => (StatusCode::CONFLICT, m),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "el paquete supera el tamaño máximo permitido".to_owned(),
            ),
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
        Self::Internal(e.to_string())
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

fn check_feed(state: &AppState, feed: &str) -> ApiResult<()> {
    if feed == state.feed {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

/// Convierte segmentos de URL en claves de identidad. Cualquier valor inválido es un 404:
/// nunca llega al sistema de archivos texto sin validar.
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

pub async fn service_index(
    State(state): State<Arc<AppState>>,
    Path(feed): Path<String>,
) -> ApiResult<impl IntoResponse> {
    check_feed(&state, &feed)?;
    let base = format!("{}/nuget/{feed}", state.public_url);
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
    Path((feed, id)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    check_feed(&state, &feed)?;
    let versions = state.store.versions(&id_key(&id)?).await?;
    if versions.is_empty() {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({ "versions": versions })))
}

pub async fn flat_file(
    State(state): State<Arc<AppState>>,
    Path((feed, id, version, file)): Path<(String, String, String, String)>,
) -> ApiResult<Response> {
    check_feed(&state, &feed)?;
    let id = id_key(&id)?;
    let version = version_key(&version)?;
    let file = file.to_lowercase();

    let nupkg = state
        .store
        .read_nupkg(&id, &version)
        .await?
        .ok_or(ApiError::NotFound)?;

    if file == format!("{id}.{version}.nupkg") {
        Ok(([(header::CONTENT_TYPE, "application/octet-stream")], nupkg).into_response())
    } else if file == format!("{id}.nuspec") {
        let nuspec = read_nuspec_bytes(&nupkg).map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(([(header::CONTENT_TYPE, "application/xml")], nuspec).into_response())
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn publish(
    State(state): State<Arc<AppState>>,
    Path(feed): Path<String>,
    mut multipart: Multipart,
) -> ApiResult<impl IntoResponse> {
    check_feed(&state, &feed)?;
    // La validación de X-NuGet-ApiKey llega en la Fase 4 (ADR-011).

    let field = multipart
        .next_field()
        .await?
        .ok_or_else(|| ApiError::BadRequest("la solicitud no contiene el paquete".into()))?;
    // Spike: el paquete se lee en memoria (acotado por DefaultBodyLimit).
    // El streaming a staging llega en la Fase 2 (ADR-006).
    let nupkg = field.bytes().await?;

    let manifest = read_package(&nupkg).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    match state.store.publish(&manifest, &nupkg).await {
        Ok(()) => {
            tracing::info!(id = %manifest.id, version = %manifest.version, "paquete publicado");
            Ok(StatusCode::CREATED)
        }
        Err(PublishError::Conflict) => Err(ApiError::Conflict(format!(
            "{} {} ya existe en el feed {feed}; las versiones publicadas son inmutables",
            manifest.id, manifest.version
        ))),
        Err(PublishError::Io(e)) => Err(e.into()),
    }
}
