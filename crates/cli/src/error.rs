//! Errores del CLI y códigos de salida documentados (ADR-015). Cambiar un código de salida
//! es un cambio incompatible.

use onepack_api_client::ErrorDetail;
use onepack_api_client::client::{ClientError, TransportKind};
use serde::Serialize;

/// Códigos de salida. Ver `docs/cli.md`.
pub mod exit {
    pub const OK: i32 = 0;
    /// Error no clasificado.
    pub const ERROR: i32 = 1;
    /// Uso incorrecto: argumentos, entrada inválida o confirmación requerida con --no-input.
    pub const USAGE: i32 = 2;
    /// Sin credencial válida (ausente, caducada o revocada).
    pub const AUTH: i32 = 3;
    /// Credencial válida sin permiso suficiente.
    pub const FORBIDDEN: i32 = 4;
    pub const NOT_FOUND: i32 = 5;
    /// Ya existe (p. ej. una versión publicada con otro contenido).
    pub const CONFLICT: i32 = 6;
    /// Red, DNS, TLS, timeouts, límites de peticiones o errores del servidor: reintentable.
    pub const UNAVAILABLE: i32 = 7;
    /// El servidor no ofrece una capacidad que el comando necesita.
    pub const INCOMPATIBLE: i32 = 8;
    /// No hay keychain del sistema disponible.
    pub const KEYCHAIN: i32 = 9;
    /// `doctor` encontró al menos un fallo.
    pub const CHECKS_FAILED: i32 = 10;
}

#[derive(Debug, Clone, Serialize)]
pub struct CliError {
    #[serde(skip)]
    pub exit: i32,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

pub type CliResult<T = ()> = Result<T, CliError>;

impl CliError {
    pub fn new(exit: i32, code: &str, message: impl Into<String>) -> Self {
        Self {
            exit,
            code: code.to_owned(),
            message: message.into(),
            action: None,
            request_id: None,
        }
    }

    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(exit::USAGE, "INVALID_USAGE", message)
    }

    pub fn io(context: &str, e: &std::io::Error) -> Self {
        Self::new(exit::ERROR, "IO_ERROR", format!("{context}: {e}"))
    }
}

fn exit_for_status(status: u16) -> i32 {
    match status {
        400 | 422 => exit::USAGE,
        401 => exit::AUTH,
        403 => exit::FORBIDDEN,
        404 => exit::NOT_FOUND,
        409 => exit::CONFLICT,
        408 | 429 | 500..=599 => exit::UNAVAILABLE,
        _ => exit::ERROR,
    }
}

impl From<ClientError> for CliError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::Api { status, detail } => {
                let ErrorDetail {
                    code,
                    message,
                    action,
                    request_id,
                } = detail;
                Self {
                    exit: exit_for_status(status),
                    code,
                    message,
                    action,
                    request_id,
                }
            }
            ClientError::Transport { kind, message } => {
                let (code, action) = match kind {
                    TransportKind::Dns => (
                        "DNS_FAILED",
                        "comprueba la URL del contexto y la resolución DNS (`onepack doctor`)",
                    ),
                    TransportKind::Connect => (
                        "CONNECT_FAILED",
                        "comprueba que el servidor esté en marcha y accesible (`onepack doctor`)",
                    ),
                    TransportKind::Tls => (
                        "TLS_FAILED",
                        "comprueba el certificado del servidor; con una CA propia usa --ca-cert",
                    ),
                    TransportKind::Timeout => ("TIMEOUT", "reintenta o amplía --timeout"),
                    TransportKind::Other => ("NETWORK_ERROR", "reintenta (`onepack doctor`)"),
                };
                Self::new(exit::UNAVAILABLE, code, message).with_action(action)
            }
            ClientError::InvalidResponse(m) => Self::new(
                exit::INCOMPATIBLE,
                "INVALID_RESPONSE",
                format!("respuesta inesperada del servidor: {m}"),
            )
            .with_action("comprueba que la URL apunta a un servidor onepack compatible"),
            ClientError::Config(m) => Self::new(exit::USAGE, "INVALID_CONFIG", m),
        }
    }
}
