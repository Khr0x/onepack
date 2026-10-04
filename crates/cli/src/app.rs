//! Estado común de una ejecución: opciones globales, contexto, credencial y cliente HTTP.

use std::cell::OnceCell;
use std::path::PathBuf;
use std::time::Duration;

use onepack_api_client::Capabilities;
use onepack_api_client::client::{Client, ClientConfig};

use crate::config::{Config, Context};
use crate::credentials::{self, Credential};
use crate::error::{CliError, CliResult, exit};
use crate::output::Out;

/// Opciones globales (ver `main.rs`).
#[derive(Debug, Clone)]
pub struct Global {
    pub context: Option<String>,
    pub url: Option<String>,
    pub token_env: Option<String>,
    pub timeout: Option<u64>,
    pub ca_cert: Option<PathBuf>,
}

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct App {
    pub global: Global,
    pub out: Out,
    pub config: Config,
    capabilities: OnceCell<Capabilities>,
}

/// Contexto resuelto: nombre (si viene de la configuración) y destino.
#[derive(Debug, Clone)]
pub struct Target {
    pub name: Option<String>,
    pub url: String,
    pub ca_cert: Option<PathBuf>,
}

impl App {
    pub fn new(global: Global, out: Out) -> CliResult<Self> {
        Ok(Self {
            global,
            out,
            config: Config::load()?,
            capabilities: OnceCell::new(),
        })
    }

    /// `--url` tiene prioridad sobre el contexto (útil en pipelines sin configuración).
    pub fn target(&self) -> CliResult<Target> {
        if let Some(url) = &self.global.url {
            return Ok(Target {
                name: None,
                url: url.trim_end_matches('/').to_owned(),
                ca_cert: self.global.ca_cert.clone(),
            });
        }
        let (name, Context { url, ca_cert }) =
            self.config.resolve(self.global.context.as_deref())?;
        Ok(Target {
            name: Some(name),
            url,
            ca_cert: self.global.ca_cert.clone().or(ca_cert),
        })
    }

    pub fn credential(&self, target: &Target) -> CliResult<Option<Credential>> {
        credentials::resolve(target.name.as_deref(), self.global.token_env.as_deref())
    }

    pub fn client_for(
        &self,
        target: &Target,
        token: Option<String>,
        default_timeout: Duration,
    ) -> CliResult<Client> {
        let ca_cert_pem = match &target.ca_cert {
            Some(path) => Some(
                std::fs::read(path).map_err(|e| CliError::io(&path.display().to_string(), &e))?,
            ),
            None => None,
        };
        Ok(Client::new(ClientConfig {
            base_url: target.url.clone(),
            token,
            timeout: self
                .global
                .timeout
                .map_or(default_timeout, Duration::from_secs),
            ca_cert_pem,
        })?)
    }

    /// Cliente autenticado con la credencial del contexto.
    pub fn client_with_timeout(&self, default_timeout: Duration) -> CliResult<Client> {
        let target = self.target()?;
        let credential = self.credential(&target)?;
        self.client_for(&target, credential.map(|c| c.token), default_timeout)
    }

    pub fn client(&self) -> CliResult<Client> {
        self.client_with_timeout(DEFAULT_TIMEOUT)
    }

    /// Falla con un error claro si el servidor no ofrece la capacidad (N/N-1, ADR-015).
    pub fn require(&self, client: &Client, capability: &str) -> CliResult<()> {
        let caps = match self.capabilities.get() {
            Some(c) => c,
            None => {
                let c = client.capabilities()?;
                self.capabilities.get_or_init(|| c)
            }
        };
        if caps.supports(capability) {
            Ok(())
        } else {
            Err(CliError::new(
                exit::INCOMPATIBLE,
                "CAPABILITY_MISSING",
                format!(
                    "el servidor (versión {}) no ofrece la capacidad {capability:?} que necesita este comando",
                    caps.server_version
                ),
            )
            .with_action("actualiza onepackd o usa una versión del CLI acorde al servidor"))
        }
    }
}

/// Escapa un segmento de ruta.
pub fn seg(value: &str) -> String {
    onepack_api_client::client::encode_query(value)
}
