//! Cliente bloqueante de `/api/v1` y de las rutas NuGet que usa `onepack` (ADR-015).
//!
//! Los reintentos se hacen solo en peticiones idempotentes (`GET`, `PUT`, `DELETE`) y ante
//! fallos transitorios: conexión, `429`, `502`, `503` y `504`. Los errores nunca incluyen el
//! token.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;
use serde::de::DeserializeOwned;
use ureq::http::{Method, Request};
use ureq::tls::{PemItem, RootCerts, TlsConfig};
use ureq::{Agent, SendBody};

use crate::{Capabilities, ErrorDetail, Page};

const RETRIES: u32 = 3;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(10);
const MULTIPART_BOUNDARY: &str = "onepack-cli-boundary-8b1c2f";

#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// URL pública del servidor (sin `/api/v1`).
    pub base_url: String,
    pub token: Option<String>,
    pub timeout: Duration,
    /// CA adicional en PEM, para servidores con certificados de una CA propia.
    pub ca_cert_pem: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Dns,
    Connect,
    Tls,
    Timeout,
    Other,
}

#[derive(Debug)]
pub enum ClientError {
    /// El servidor respondió con un error.
    Api {
        status: u16,
        detail: ErrorDetail,
    },
    /// No se llegó a obtener respuesta.
    Transport {
        kind: TransportKind,
        message: String,
    },
    /// La respuesta no tiene el formato esperado (p. ej. no es un servidor onepack).
    InvalidResponse(String),
    Config(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api { status, detail } => {
                write!(f, "{}: {} (HTTP {status})", detail.code, detail.message)
            }
            Self::Transport { message, .. } => f.write_str(message),
            Self::InvalidResponse(m) => write!(f, "unexpected response from the server: {m}"),
            Self::Config(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ClientError {}

impl ClientError {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Api { detail, .. } => Some(&detail.code),
            _ => None,
        }
    }
}

/// Respuesta sin interpretar, para las rutas NuGet.
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub request_id: Option<String>,
}

impl RawResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Clone, Copy)]
enum Auth {
    Bearer,
    NuGetApiKey,
    Basic,
}

pub struct Client {
    base: String,
    token: Option<String>,
    agent: Agent,
}

impl Client {
    pub fn new(config: ClientConfig) -> Result<Self, ClientError> {
        let base = config.base_url.trim_end_matches('/').to_owned();
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            return Err(ClientError::Config(format!(
                "invalid URL {base:?}: it must start with https:// or http://"
            )));
        }
        let root_certs = match &config.ca_cert_pem {
            None => RootCerts::PlatformVerifier,
            Some(pem) => {
                let certs: Vec<_> = ureq::tls::parse_pem(pem)
                    .filter_map(|item| match item {
                        Ok(PemItem::Certificate(c)) => Some(c),
                        _ => None,
                    })
                    .collect();
                if certs.is_empty() {
                    return Err(ClientError::Config(
                        "the CA file contains no PEM certificates".into(),
                    ));
                }
                RootCerts::new_with_certs(&certs)
            }
        };
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(config.timeout))
            .http_status_as_error(false)
            .user_agent(format!("onepack/{}", env!("CARGO_PKG_VERSION")))
            .tls_config(TlsConfig::builder().root_certs(root_certs).build())
            .build()
            .into();
        Ok(Self {
            base,
            token: config.token,
            agent,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn token(&self) -> Result<&str, ClientError> {
        self.token.as_deref().ok_or_else(|| ClientError::Api {
            status: 401,
            detail: ErrorDetail {
                code: "AUTH_REQUIRED".into(),
                message: "there is no credential for this context".into(),
                action: Some("run `onepack login` or set the --token-env variable".into()),
                request_id: None,
            },
        })
    }

    fn send(
        &self,
        method: Method,
        url: &str,
        auth: Auth,
        body: Option<(&str, &[u8])>,
        retry: bool,
    ) -> Result<RawResponse, ClientError> {
        let token = self.token()?;
        let idempotent = retry && matches!(method, Method::GET | Method::PUT | Method::DELETE);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut req = Request::builder().method(method.clone()).uri(url);
            req = match auth {
                Auth::Bearer => req.header("Authorization", format!("Bearer {token}")),
                Auth::NuGetApiKey => req.header("X-NuGet-ApiKey", token),
                Auth::Basic => req.header(
                    "Authorization",
                    format!("Basic {}", STANDARD.encode(format!("onepack:{token}"))),
                ),
            };
            let result = match body {
                Some((content_type, bytes)) => self.agent.run(
                    req.header("Content-Type", content_type)
                        .body(bytes)
                        .map_err(|e| ClientError::Config(e.to_string()))?,
                ),
                None => self.agent.run(
                    req.body(SendBody::none())
                        .map_err(|e| ClientError::Config(e.to_string()))?,
                ),
            };
            let retry_wait = match result {
                Ok(mut res) => {
                    let status = res.status().as_u16();
                    let header = |name: &str| {
                        res.headers()
                            .get(name)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned)
                    };
                    let request_id = header("x-request-id");
                    let retry_after = header("retry-after").and_then(|v| v.parse::<u64>().ok());
                    let transient = matches!(status, 429 | 502 | 503 | 504);
                    if idempotent && transient && attempt < RETRIES {
                        Duration::from_secs(retry_after.unwrap_or(attempt as u64))
                    } else {
                        let body = res.body_mut().read_to_vec().map_err(transport)?;
                        return Ok(RawResponse {
                            status,
                            body,
                            request_id,
                        });
                    }
                }
                Err(e) => {
                    let e = transport(e);
                    let retryable = matches!(
                        e,
                        ClientError::Transport {
                            kind: TransportKind::Connect | TransportKind::Timeout,
                            ..
                        }
                    );
                    if idempotent && retryable && attempt < RETRIES {
                        Duration::from_secs(attempt as u64)
                    } else {
                        return Err(e);
                    }
                }
            };
            std::thread::sleep(retry_wait.min(MAX_RETRY_WAIT));
        }
    }

    fn api<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<T, ClientError> {
        let url = format!("{}/api/v1{path}", self.base);
        let res = self.send(
            method,
            &url,
            Auth::Bearer,
            body.as_deref().map(|b| ("application/json", b)),
            true,
        )?;
        if !(200..300).contains(&res.status) {
            return Err(api_error(&res));
        }
        let body = if res.body.is_empty() {
            b"null".as_slice()
        } else {
            &res.body
        };
        serde_json::from_slice(body).map_err(|e| ClientError::InvalidResponse(e.to_string()))
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        self.api(Method::GET, path, None)
    }

    pub fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        self.api(Method::POST, path, Some(to_json(body)?))
    }

    pub fn put<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        self.api(Method::PUT, path, Some(to_json(body)?))
    }

    pub fn patch<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        self.api(Method::PATCH, path, Some(to_json(body)?))
    }

    pub fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        self.api(Method::DELETE, path, None)
    }

    /// Recorre todas las páginas de un listado. `path` puede llevar ya parámetros.
    pub fn list_all<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>, ClientError> {
        let separator = if path.contains('?') { '&' } else { '?' };
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let url = match &cursor {
                Some(c) => format!("{path}{separator}cursor={}", encode_query(c)),
                None => path.to_owned(),
            };
            let page: Page<T> = self.get(&url)?;
            items.extend(page.items);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(items),
            }
        }
    }

    /// Capacidades del servidor. Un servidor anterior a la Fase 6 no tiene el endpoint: se
    /// trata como uno sin capacidades administrativas.
    pub fn capabilities(&self) -> Result<Capabilities, ClientError> {
        match self.get::<Capabilities>("/capabilities") {
            Err(ClientError::Api { status: 404, .. }) => Ok(Capabilities {
                server_version: "unknown".into(),
                api_version: 1,
                capabilities: Vec::new(),
            }),
            other => other,
        }
    }

    /// Publica un `.nupkg` por `PackagePublish` (la misma ruta que `dotnet nuget push`).
    /// No se reintenta: un reintento tras un éxito no confirmado daría `409`.
    pub fn nuget_push(&self, feed: &str, nupkg: &[u8]) -> Result<RawResponse, ClientError> {
        let mut body = format!(
            "--{MULTIPART_BOUNDARY}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"package.nupkg\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(nupkg);
        body.extend_from_slice(format!("\r\n--{MULTIPART_BOUNDARY}--\r\n").as_bytes());
        let content_type = format!("multipart/form-data; boundary={MULTIPART_BOUNDARY}");
        self.send(
            Method::PUT,
            &format!("{}/nuget/{feed}/v2/package", self.base),
            Auth::NuGetApiKey,
            Some((&content_type, &body)),
            false,
        )
    }

    /// `GET` autenticado como lo haría un cliente NuGet (Basic con el token).
    pub fn nuget_get(&self, url: &str) -> Result<RawResponse, ClientError> {
        self.send(Method::GET, url, Auth::Basic, None, true)
    }

    /// `GET` sin credenciales: sirve para identificar el servidor (p. ej. en `doctor`).
    pub fn probe(&self, url: &str) -> Result<RawResponse, ClientError> {
        let res = self
            .agent
            .run(
                Request::builder()
                    .method(Method::GET)
                    .uri(url)
                    .body(SendBody::none())
                    .map_err(|e| ClientError::Config(e.to_string()))?,
            )
            .map_err(transport);
        let mut res = res?;
        let request_id = res
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        Ok(RawResponse {
            status: res.status().as_u16(),
            body: res.body_mut().read_to_vec().map_err(transport)?,
            request_id,
        })
    }
}

fn to_json<B: Serialize>(body: &B) -> Result<Vec<u8>, ClientError> {
    serde_json::to_vec(body).map_err(|e| ClientError::Config(e.to_string()))
}

/// Interpreta un error: JSON de `/api/v1` o texto `CÓDIGO: mensaje` de las rutas NuGet.
pub fn api_error(res: &RawResponse) -> ClientError {
    let detail = serde_json::from_slice::<crate::ErrorBody>(&res.body)
        .map(|b| b.error)
        .unwrap_or_else(|_| {
            let text = res.text();
            let (code, message) = match text.split_once(": ") {
                Some((code, message))
                    if !code.is_empty()
                        && code.chars().all(|c| c.is_ascii_uppercase() || c == '_') =>
                {
                    (code.to_owned(), message.trim().to_owned())
                }
                _ => (
                    match res.status {
                        401 => "AUTH_REQUIRED",
                        403 => "FORBIDDEN",
                        404 => "NOT_FOUND",
                        _ => "HTTP_ERROR",
                    }
                    .to_owned(),
                    if text.trim().is_empty() {
                        format!("HTTP {}", res.status)
                    } else {
                        text.trim().to_owned()
                    },
                ),
            };
            ErrorDetail {
                code,
                message,
                action: None,
                request_id: None,
            }
        });
    let mut detail = detail;
    if detail.request_id.is_none() {
        detail.request_id = res.request_id.clone();
    }
    ClientError::Api {
        status: res.status,
        detail,
    }
}

fn transport(e: ureq::Error) -> ClientError {
    let message = e.to_string();
    let kind = match &e {
        ureq::Error::HostNotFound => TransportKind::Dns,
        ureq::Error::ConnectionFailed => TransportKind::Connect,
        ureq::Error::Timeout(_) => TransportKind::Timeout,
        ureq::Error::Tls(_) | ureq::Error::Pem(_) => TransportKind::Tls,
        ureq::Error::Io(io) => match io.kind() {
            std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected => TransportKind::Connect,
            std::io::ErrorKind::TimedOut => TransportKind::Timeout,
            _ if message.to_lowercase().contains("certificate")
                || message.to_lowercase().contains("tls") =>
            {
                TransportKind::Tls
            }
            _ => TransportKind::Other,
        },
        _ => TransportKind::Other,
    };
    ClientError::Transport { kind, message }
}

/// Escapa un valor para la query string.
pub fn encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(status: u16, body: &str) -> RawResponse {
        RawResponse {
            status,
            body: body.as_bytes().to_vec(),
            request_id: Some("r1".into()),
        }
    }

    #[test]
    fn parses_api_and_nuget_errors() {
        let e = api_error(&raw(
            403,
            r#"{"error":{"code":"AUTH_SCOPE_MISSING","message":"m","action":"a"}}"#,
        ));
        assert_eq!(e.code(), Some("AUTH_SCOPE_MISSING"));
        let ClientError::Api { detail, .. } = e else {
            panic!()
        };
        assert_eq!(detail.request_id.as_deref(), Some("r1"));

        let e = api_error(&raw(409, "PACKAGE_VERSION_EXISTS: A@1.0.0 already exists"));
        assert_eq!(e.code(), Some("PACKAGE_VERSION_EXISTS"));
        assert_eq!(api_error(&raw(404, "")).code(), Some("NOT_FOUND"));
        assert_eq!(api_error(&raw(500, "boom")).code(), Some("HTTP_ERROR"));
    }

    #[test]
    fn rejects_urls_without_scheme() {
        let config = ClientConfig {
            base_url: "packages.example.com".into(),
            token: None,
            timeout: Duration::from_secs(1),
            ca_cert_pem: None,
        };
        assert!(matches!(Client::new(config), Err(ClientError::Config(_))));
    }

    #[test]
    fn query_encoding() {
        assert_eq!(encode_query("a b/c=d"), "a%20b%2Fc%3Dd");
    }
}
