//! CLI credentials (ADR-015): system keychain or environment variable. They are never
//! written to files and there is never a silent fallback to plaintext.
//!
//! Resolution order:
//! 1. `--token-env VAR`: the token is in the `VAR` variable (pipelines).
//! 2. `ONEPACK_TOKEN`.
//! 3. The system keychain, with the context entry (`onepack login`).
//!
//! `ONEPACK_KEYRING=off` disables the keychain (containers, CI): `login` fails
//! explicitly instead of storing the token somewhere else.

use crate::error::{CliError, CliResult, exit};

static KEYCHAIN: std::sync::LazyLock<keyring_core::Result<()>> = std::sync::LazyLock::new(|| {
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new()?;
    #[cfg(target_os = "windows")]
    let store = windows_native_keyring_store::Store::new()?;
    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    ))]
    let store = dbus_secret_service_keyring_store::Store::new()?;
    #[cfg(all(any(unix, windows), not(any(target_os = "ios", target_os = "android"))))]
    {
        keyring_core::set_default_store(store);
        Ok(())
    }
    #[cfg(not(all(any(unix, windows), not(any(target_os = "ios", target_os = "android")))))]
    Err(keyring_core::Error::NoDefaultStore)
});

const SERVICE: &str = "onepack";
pub const TOKEN_ENV: &str = "ONEPACK_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Env(String),
    Keychain,
}

impl Source {
    pub fn describe(&self) -> String {
        match self {
            Self::Env(var) => format!("environment variable {var}"),
            Self::Keychain => "system keychain".to_owned(),
        }
    }
}

#[derive(Clone)]
pub struct Credential {
    pub token: String,
    pub source: Source,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The secret never appears in traces or error messages.
        f.debug_struct("Credential")
            .field("token_id", &token_id(&self.token))
            .field("source", &self.source)
            .finish()
    }
}

/// Public part of the token (`opk_<id>_…`): identifies the token without revealing the secret.
pub fn token_id(token: &str) -> Option<&str> {
    token
        .strip_prefix("opk_")
        .and_then(|rest| rest.split_once('_'))
        .map(|(id, _)| id)
}

fn keychain_unavailable(detail: impl std::fmt::Display) -> CliError {
    CliError::new(
        exit::KEYCHAIN,
        "KEYCHAIN_UNAVAILABLE",
        format!("no system keychain is available: {detail}"),
    )
    .with_action(
        "in CI or containers, pass the token in an environment variable: --token-env <VAR> or ONEPACK_TOKEN",
    )
}

fn keyring_disabled() -> bool {
    std::env::var("ONEPACK_KEYRING").is_ok_and(|v| v.eq_ignore_ascii_case("off"))
}

fn entry(context: &str) -> CliResult<keyring_core::Entry> {
    if keyring_disabled() {
        return Err(keychain_unavailable("disabled with ONEPACK_KEYRING=off"));
    }
    KEYCHAIN.as_ref().map_err(keychain_unavailable)?;
    keyring_core::Entry::new(SERVICE, context).map_err(keychain_unavailable)
}

/// Token for a context. `Ok(None)` if none is configured.
pub fn resolve(context: Option<&str>, token_env: Option<&str>) -> CliResult<Option<Credential>> {
    if let Some(var) = token_env {
        return match std::env::var(var) {
            Ok(token) if !token.trim().is_empty() => Ok(Some(Credential {
                token: token.trim().to_owned(),
                source: Source::Env(var.to_owned()),
            })),
            _ => Err(CliError::new(
                exit::AUTH,
                "AUTH_REQUIRED",
                format!("the variable {var} is not set or is empty"),
            )
            .with_action(format!("set {var} to the token before running the command"))),
        };
    }
    if let Ok(token) = std::env::var(TOKEN_ENV)
        && !token.trim().is_empty()
    {
        return Ok(Some(Credential {
            token: token.trim().to_owned(),
            source: Source::Env(TOKEN_ENV.to_owned()),
        }));
    }
    let Some(context) = context else {
        return Ok(None);
    };
    if keyring_disabled() {
        return Ok(None);
    }
    match entry(context)?.get_password() {
        Ok(token) => Ok(Some(Credential {
            token,
            source: Source::Keychain,
        })),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(e) => Err(keychain_unavailable(e)),
    }
}

/// Checks that a keychain is available without reading or modifying anything.
pub fn ensure_available(context: &str) -> CliResult<()> {
    entry(context).map(|_| ())
}

pub fn store(context: &str, token: &str) -> CliResult<()> {
    entry(context)?
        .set_password(token)
        .map_err(keychain_unavailable)
}

/// Deletes the context credential. `false` if there was none.
pub fn delete(context: &str) -> CliResult<bool> {
    match entry(context)?.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring_core::Error::NoEntry) => Ok(false),
        Err(e) => Err(keychain_unavailable(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_id_is_the_public_part() {
        assert_eq!(
            token_id("opk_0123456789abcdef_secretsecret"),
            Some("0123456789abcdef")
        );
        assert_eq!(token_id("garbage"), None);
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let c = Credential {
            token: "opk_0123456789abcdef_supersecret".into(),
            source: Source::Keychain,
        };
        let shown = format!("{c:?}");
        assert!(!shown.contains("supersecret"));
        assert!(shown.contains("0123456789abcdef"));
    }
}
