//! CLI errors and documented exit codes (ADR-015). Changing an exit code
//! is a breaking change.

use onepack_api_client::ErrorDetail;
use onepack_api_client::client::{ClientError, TransportKind};
use serde::Serialize;

/// Exit codes. See `docs/cli.md`.
pub mod exit {
    pub const OK: i32 = 0;
    /// Unclassified error.
    pub const ERROR: i32 = 1;
    /// Incorrect usage: arguments, invalid input, or confirmation required with --no-input.
    pub const USAGE: i32 = 2;
    /// No valid credential (missing, expired, or revoked).
    pub const AUTH: i32 = 3;
    /// Valid credential without sufficient permission.
    pub const FORBIDDEN: i32 = 4;
    pub const NOT_FOUND: i32 = 5;
    /// Already exists (e.g. a published version with different content).
    pub const CONFLICT: i32 = 6;
    /// Network, DNS, TLS, timeouts, request limits, or server errors: retryable.
    pub const UNAVAILABLE: i32 = 7;
    /// The server does not offer a capability the command needs.
    pub const INCOMPATIBLE: i32 = 8;
    /// No system keychain available.
    pub const KEYCHAIN: i32 = 9;
    /// `doctor` found at least one failure.
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
                        "check the context URL and DNS resolution (`onepack doctor`)",
                    ),
                    TransportKind::Connect => (
                        "CONNECT_FAILED",
                        "check that the server is running and reachable (`onepack doctor`)",
                    ),
                    TransportKind::Tls => (
                        "TLS_FAILED",
                        "check the server certificate; with a private CA use --ca-cert",
                    ),
                    TransportKind::Timeout => ("TIMEOUT", "retry or increase --timeout"),
                    TransportKind::Other => ("NETWORK_ERROR", "retry (`onepack doctor`)"),
                };
                Self::new(exit::UNAVAILABLE, code, message).with_action(action)
            }
            ClientError::InvalidResponse(m) => Self::new(
                exit::INCOMPATIBLE,
                "INVALID_RESPONSE",
                format!("unexpected response from the server: {m}"),
            )
            .with_action("check that the URL points to a compatible onepack server"),
            ClientError::Config(m) => Self::new(exit::USAGE, "INVALID_CONFIG", m),
        }
    }
}
