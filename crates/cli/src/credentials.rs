//! Credenciales del CLI (ADR-015): keychain del sistema o variable de entorno. Nunca se
//! escriben en archivos y nunca hay fallback silencioso a texto plano.
//!
//! Orden de resolución:
//! 1. `--token-env VAR`: el token está en la variable `VAR` (pipelines).
//! 2. `ONEPACK_TOKEN`.
//! 3. El keychain del sistema, con la entrada del contexto (`onepack login`).
//!
//! `ONEPACK_KEYRING=off` desactiva el keychain (contenedores, CI): `login` falla de forma
//! explícita en vez de guardar el token en otro sitio.

use crate::error::{CliError, CliResult, exit};

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
        // El secreto nunca aparece en trazas ni en mensajes de error.
        f.debug_struct("Credential")
            .field("token_id", &token_id(&self.token))
            .field("source", &self.source)
            .finish()
    }
}

/// Parte pública del token (`opk_<id>_…`): identifica el token sin revelar el secreto.
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

fn entry(context: &str) -> CliResult<keyring::Entry> {
    if keyring_disabled() {
        return Err(keychain_unavailable("disabled with ONEPACK_KEYRING=off"));
    }
    keyring::Entry::new(SERVICE, context).map_err(keychain_unavailable)
}

/// Token para un contexto. `Ok(None)` si no hay ninguno configurado.
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
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(keychain_unavailable(e)),
    }
}

/// Comprueba que hay keychain sin leer ni modificar nada.
pub fn ensure_available(context: &str) -> CliResult<()> {
    entry(context).map(|_| ())
}

pub fn store(context: &str, token: &str) -> CliResult<()> {
    entry(context)?
        .set_password(token)
        .map_err(keychain_unavailable)
}

/// Borra la credencial del contexto. `false` si no había ninguna.
pub fn delete(context: &str) -> CliResult<bool> {
    match entry(context)?.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
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
