//! Contextos: a qué servidor habla el CLI. El archivo nunca contiene secretos; los tokens
//! van al keychain del sistema (ADR-015).
//!
//! Ubicación: `$ONEPACK_CONFIG_DIR`, o `$XDG_CONFIG_HOME/onepack`, o `~/.config/onepack`
//! (`%APPDATA%\onepack` en Windows), archivo `config.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult, exit};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default)]
    pub contexts: BTreeMap<String, Context>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Context {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_cert: Option<PathBuf>,
}

pub fn dir() -> CliResult<PathBuf> {
    if let Some(d) = std::env::var_os("ONEPACK_CONFIG_DIR") {
        return Ok(PathBuf::from(d));
    }
    if cfg!(windows) {
        if let Some(d) = std::env::var_os("APPDATA") {
            return Ok(PathBuf::from(d).join("onepack"));
        }
    } else if let Some(d) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(d).join("onepack"));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(".config").join("onepack"))
        .ok_or_else(|| {
            CliError::new(
                exit::ERROR,
                "CONFIG_DIR_UNKNOWN",
                "could not determine the configuration directory",
            )
            .with_action("set ONEPACK_CONFIG_DIR")
        })
}

fn path() -> CliResult<PathBuf> {
    Ok(dir()?.join("config.json"))
}

/// Nombre de contexto: letras, dígitos, `-`, `_` y `.`.
pub fn validate_name(name: &str) -> CliResult<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(CliError::usage(format!(
            "invalid context name {name:?}: use letters, digits, '-', '_' or '.'"
        )))
    }
}

impl Config {
    pub fn load() -> CliResult<Self> {
        let path = path()?;
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                CliError::new(
                    exit::ERROR,
                    "INVALID_CONFIG",
                    format!("{} is not valid: {e}", path.display()),
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(CliError::io(&path.display().to_string(), &e)),
        }
    }

    pub fn save(&self) -> CliResult<()> {
        let path = path()?;
        let dir = path.parent().expect("the file is inside a directory");
        std::fs::create_dir_all(dir).map_err(|e| CliError::io(&dir.display().to_string(), &e))?;
        let json = serde_json::to_vec_pretty(self).expect("the configuration is serializable");
        // Escritura atómica: un corte a mitad no deja el archivo truncado.
        let tmp = dir.join("config.json.tmp");
        std::fs::write(&tmp, json).map_err(|e| CliError::io(&tmp.display().to_string(), &e))?;
        std::fs::rename(&tmp, &path).map_err(|e| CliError::io(&path.display().to_string(), &e))
    }

    /// Contexto que se usa: el indicado o el actual.
    pub fn resolve(&self, name: Option<&str>) -> CliResult<(String, Context)> {
        let name = name.or(self.current.as_deref()).ok_or_else(|| {
            CliError::new(exit::USAGE, "CONTEXT_MISSING", "no context is configured")
                .with_action("create one with `onepack context add <name> --url <url>`")
        })?;
        let context = self.contexts.get(name).cloned().ok_or_else(|| {
            CliError::new(
                exit::USAGE,
                "CONTEXT_MISSING",
                format!("context {name:?} does not exist"),
            )
            .with_action("list the contexts with `onepack context list`")
        })?;
        Ok((name.to_owned(), context))
    }
}
