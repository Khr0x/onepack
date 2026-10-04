//! `onepack doctor` (ADR-015): diagnóstico de punta a punta, como producto. Cada
//! comprobación da un estado, un código estable cuando falla y la acción a tomar. Nunca
//! muestra el token.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use onepack_api_client::client::{ClientError, TransportKind};
use onepack_api_client::{Capabilities, WhoAmI};
use serde::Serialize;

use crate::app::{App, DEFAULT_TIMEOUT, Target};
use crate::credentials::token_id;
use crate::error::{CliError, CliResult, exit};
use crate::nuget::{feed_url, find_config_in};
use crate::nuget_config::Document;
use crate::{DoctorArgs, ScopeArg};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Serialize)]
struct Check {
    name: &'static str,
    status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<String>,
}

#[derive(Serialize)]
struct Report {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<String>,
    url: Option<String>,
    checks: Vec<Check>,
}

struct Checks(Vec<Check>);

impl Checks {
    fn ok(&mut self, name: &'static str, message: impl Into<String>) {
        self.push(name, Status::Ok, None, message, None);
    }

    fn skip(&mut self, name: &'static str, message: impl Into<String>) {
        self.push(name, Status::Skip, None, message, None);
    }

    fn warn(&mut self, name: &'static str, code: &str, message: impl Into<String>, action: &str) {
        self.push(name, Status::Warn, Some(code), message, Some(action));
    }

    fn fail(&mut self, name: &'static str, code: &str, message: impl Into<String>, action: &str) {
        self.push(name, Status::Fail, Some(code), message, Some(action));
    }

    fn fail_with(&mut self, name: &'static str, e: CliError) {
        self.0.push(Check {
            name,
            status: Status::Fail,
            code: Some(e.code),
            message: e.message,
            action: e.action,
        });
    }

    fn push(
        &mut self,
        name: &'static str,
        status: Status,
        code: Option<&str>,
        message: impl Into<String>,
        action: Option<&str>,
    ) {
        self.0.push(Check {
            name,
            status,
            code: code.map(str::to_owned),
            message: message.into(),
            action: action.map(str::to_owned),
        });
    }
}

fn host_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let default = if scheme == "https" { 443 } else { 80 };
    if let Some(v6) = authority.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        let port = after
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .unwrap_or(default);
        return Some((host.to_owned(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host.to_owned(), port.parse().ok()?)),
        None => Some((authority.to_owned(), default)),
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host.starts_with("127.") || host == "::1"
}

/// Busca `NuGet.Config` en el directorio actual y sus padres, como hace `dotnet`.
fn locate_config(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if explicit.is_some() {
        return explicit;
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if let Some(found) = find_config_in(&dir) {
            return Some(found);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/')
        .eq_ignore_ascii_case(b.trim_end_matches('/'))
}

pub fn run(app: &App, args: DoctorArgs) -> CliResult<i32> {
    let mut checks = Checks(Vec::new());
    let target = match app.target() {
        Ok(t) => {
            checks.ok(
                "context",
                match &t.name {
                    Some(n) => format!("contexto {n} -> {}", t.url),
                    None => format!("--url {}", t.url),
                },
            );
            Some(t)
        }
        Err(e) => {
            checks.fail_with("context", e);
            None
        }
    };
    if let Some(target) = &target {
        diagnose(app, &args, target, &mut checks);
    }

    let ok = !checks.0.iter().any(|c| c.status == Status::Fail);
    let report = Report {
        ok,
        context: target.as_ref().and_then(|t| t.name.clone()),
        url: target.as_ref().map(|t| t.url.clone()),
        checks: checks.0,
    };
    app.out.data(&report, || {
        let mut lines: Vec<String> = report
            .checks
            .iter()
            .map(|c| {
                let mark = match c.status {
                    Status::Ok => "ok  ",
                    Status::Warn => "WARN",
                    Status::Fail => "FAIL",
                    Status::Skip => "--  ",
                };
                let mut line = format!("[{mark}] {:<15} {}", c.name, c.message);
                if let Some(code) = &c.code {
                    line.push_str(&format!(" ({code})"));
                }
                if let Some(action) = &c.action {
                    line.push_str(&format!("\n       acción: {action}"));
                }
                line
            })
            .collect();
        lines.push(if ok {
            "Sin fallos.".to_owned()
        } else {
            "Hay fallos.".to_owned()
        });
        lines.join("\n")
    });
    Ok(if ok { exit::OK } else { exit::CHECKS_FAILED })
}

fn diagnose(app: &App, args: &DoctorArgs, target: &Target, checks: &mut Checks) {
    // --- Red ---
    let Some((host, port)) = host_port(&target.url) else {
        checks.fail(
            "url",
            "INVALID_URL",
            format!("URL inválida: {}", target.url),
            "corrige la URL del contexto con `onepack context add`",
        );
        return;
    };
    let addrs: Vec<_> = match (host.as_str(), port).to_socket_addrs() {
        Ok(a) => a.collect(),
        Err(e) => {
            checks.fail(
                "dns",
                "DNS_FAILED",
                format!("no se pudo resolver {host}: {e}"),
                "comprueba el nombre del servidor y la configuración DNS",
            );
            return;
        }
    };
    checks.ok("dns", format!("{host} -> {} dirección(es)", addrs.len()));
    let timeout = app
        .global
        .timeout
        .map_or(DEFAULT_TIMEOUT, Duration::from_secs);
    match addrs
        .iter()
        .find_map(|a| TcpStream::connect_timeout(a, timeout.min(Duration::from_secs(10))).ok())
    {
        Some(_) => checks.ok("tcp", format!("conexión a {host}:{port}")),
        None => {
            checks.fail(
                "tcp",
                "CONNECT_FAILED",
                format!("no se pudo conectar a {host}:{port}"),
                "comprueba que onepackd esté en marcha y que no lo bloquee un firewall",
            );
            return;
        }
    }

    // --- TLS (sin credencial) ---
    let anonymous = match app.client_for(target, None, timeout) {
        Ok(c) => c,
        Err(e) => {
            checks.fail_with("tls", e);
            return;
        }
    };
    let probe = anonymous.probe(&format!("{}/api/v1/capabilities", target.url));
    if target.url.starts_with("https://") {
        match &probe {
            Err(ClientError::Transport {
                kind: TransportKind::Tls,
                message,
            }) => {
                checks.fail(
                    "tls",
                    "TLS_FAILED",
                    message.clone(),
                    "revisa el certificado del servidor; con una CA propia usa --ca-cert",
                );
                return;
            }
            Err(e) => {
                checks.fail_with("tls", CliError::from(clone_err(e)));
                return;
            }
            Ok(_) => checks.ok("tls", "certificado válido"),
        }
    } else if is_loopback(&host) {
        checks.ok("tls", "HTTP sin TLS en loopback");
    } else {
        checks.warn(
            "tls",
            "PLAINTEXT_HTTP",
            "HTTP sin TLS: el token viaja en claro",
            "usa https:// (TLS directo o detrás de un reverse proxy)",
        );
    }

    // --- Credencial ---
    let credential = match app.credential(target) {
        Ok(Some(c)) => {
            checks.ok(
                "credentials",
                format!(
                    "token opk_{}_… desde {}",
                    token_id(&c.token).unwrap_or("?"),
                    c.source.describe()
                ),
            );
            c
        }
        Ok(None) => {
            checks.fail(
                "credentials",
                "AUTH_REQUIRED",
                "no hay token para este contexto",
                "ejecuta `onepack login` o usa --token-env <VAR>",
            );
            return;
        }
        Err(e) => {
            checks.fail_with("credentials", e);
            return;
        }
    };
    let client = match app.client_for(target, Some(credential.token.clone()), timeout) {
        Ok(c) => c,
        Err(e) => {
            checks.fail_with("auth", e);
            return;
        }
    };
    let me: WhoAmI = match client.get("/whoami") {
        Ok(me) => me,
        Err(e) => {
            checks.fail_with("auth", e.into());
            return;
        }
    };
    let expires = me.token_expires_at.clone().unwrap_or_else(|| "?".into());
    checks.ok(
        "auth",
        format!(
            "{} ({}{}); el token caduca {expires}",
            me.principal,
            me.kind,
            if me.administrator {
                ", administrador"
            } else {
                ""
            }
        ),
    );

    // --- Servidor ---
    match client.capabilities() {
        Ok(Capabilities {
            server_version,
            api_version,
            capabilities,
        }) if capabilities.is_empty() => checks.warn(
            "server",
            "CAPABILITY_MISSING",
            format!(
                "onepackd {server_version} (API v{api_version}) sin API administrativa completa"
            ),
            "actualiza onepackd para usar todos los comandos del CLI",
        ),
        Ok(caps) => checks.ok(
            "server",
            format!(
                "onepackd {} (API v{}, {} capacidades)",
                caps.server_version,
                caps.api_version,
                caps.capabilities.len()
            ),
        ),
        Err(e) => checks.fail_with("server", e.into()),
    }

    let Some(feed) = &args.feed else {
        checks.skip("permissions", "sin --feed");
        checks.skip("service-index", "sin --feed");
        checks.skip("nuget-config", "sin --feed");
        return;
    };

    // --- Permisos ---
    let role = if me.administrator {
        Some(ScopeArg::Maintain)
    } else {
        me.grants
            .iter()
            .find(|g| &g.feed == feed)
            .map(|g| match g.role.as_str() {
                "maintainer" => ScopeArg::Maintain,
                "publisher" => ScopeArg::Publish,
                _ => ScopeArg::Read,
            })
    };
    let scope_name = |s: ScopeArg| match s {
        ScopeArg::Read => "packages:read",
        ScopeArg::Publish => "packages:publish",
        ScopeArg::Maintain => "packages:maintain",
    };
    let role_for = |s: ScopeArg| match s {
        ScopeArg::Read => "reader",
        ScopeArg::Publish => "publisher",
        ScopeArg::Maintain => "maintainer",
    };
    match role {
        None => checks.fail(
            "permissions",
            "FEED_NOT_FOUND",
            format!(
                "{} no tiene acceso al feed {feed} (o no existe)",
                me.principal
            ),
            &format!(
                "pide a un administrador: onepack grant add --principal {} --feed {feed} --role {}",
                me.principal,
                role_for(args.require)
            ),
        ),
        Some(have) if have < args.require => checks.fail(
            "permissions",
            "AUTH_SCOPE_MISSING",
            format!(
                "{} tiene {} en {feed}, pero falta {}",
                me.principal,
                scope_name(have),
                scope_name(args.require)
            ),
            &format!(
                "pide a un administrador: onepack grant add --principal {} --feed {feed} --role {}",
                me.principal,
                role_for(args.require)
            ),
        ),
        Some(have) => checks.ok("permissions", format!("{} en {feed}", scope_name(have))),
    }

    // --- Service index ---
    let index_url = feed_url(&target.url, feed);
    match client.nuget_get(&index_url) {
        Ok(res) if res.status == 200 => {
            let index: serde_json::Value = serde_json::from_slice(&res.body).unwrap_or_default();
            let ids: Vec<String> = index["resources"]
                .as_array()
                .map(|r| {
                    r.iter()
                        .filter_map(|x| x["@id"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let foreign: Vec<&String> = ids
                .iter()
                .filter(|id| !id.starts_with(&target.url))
                .collect();
            if ids.is_empty() {
                checks.fail(
                    "service-index",
                    "INVALID_SERVICE_INDEX",
                    "el service index no anuncia recursos",
                    "comprueba que la URL apunta a onepackd",
                );
            } else if let Some(first) = foreign.first() {
                checks.fail(
                    "service-index",
                    "PUBLIC_URL_MISMATCH",
                    format!("el servidor anuncia URLs con otra base ({first})"),
                    "haz que --public-url de onepackd coincida con la URL del contexto",
                );
            } else {
                checks.ok(
                    "service-index",
                    format!("{} recursos en {index_url}", ids.len()),
                );
            }
        }
        Ok(res) => checks.fail_with(
            "service-index",
            onepack_api_client::client::api_error(&res).into(),
        ),
        Err(e) => checks.fail_with("service-index", e.into()),
    }

    // --- NuGet.Config ---
    nuget_config_checks(args.config.clone(), feed, &index_url, checks);
}

fn nuget_config_checks(explicit: Option<PathBuf>, feed: &str, url: &str, checks: &mut Checks) {
    let init = format!("onepack nuget init --feed {feed} --pattern '<Prefijo>.*'");
    let Some(path) = locate_config(explicit) else {
        checks.warn(
            "nuget-config",
            "NUGET_CONFIG_MISSING",
            "no hay NuGet.Config en este directorio ni en sus padres",
            &init,
        );
        return;
    };
    let doc = match read_doc(&path) {
        Ok(d) => d,
        Err(message) => {
            checks.fail(
                "nuget-config",
                "INVALID_NUGET_CONFIG",
                message,
                "corrige el XML del archivo",
            );
            return;
        }
    };
    let Some((key, _)) = doc.sources().into_iter().find(|(_, v)| same_url(v, url)) else {
        checks.warn(
            "nuget-config",
            "SOURCE_MISSING",
            format!("{} no declara {url}", path.display()),
            &init,
        );
        return;
    };
    checks.ok(
        "nuget-config",
        format!("fuente {key} en {}", path.display()),
    );
    if doc
        .cleartext_credentials()
        .iter()
        .any(|k| k.eq_ignore_ascii_case(&key))
    {
        checks.warn(
            "credentials-file",
            "CLEARTEXT_CREDENTIALS",
            format!(
                "{} guarda la contraseña de {key} en texto plano",
                path.display()
            ),
            "bórrala y usa `onepack exec` o NuGetPackageSourceCredentials_* en CI",
        );
    }
    match doc.mapping() {
        None => checks.warn(
            "source-mapping",
            "SOURCE_MAPPING_MISSING",
            "sin packageSourceMapping: un paquete interno podría resolverse desde otra fuente",
            &init,
        ),
        Some(mapping) => match mapping.iter().find(|(k, _)| k == &key) {
            Some((_, patterns)) if !patterns.is_empty() => {
                checks.ok("source-mapping", format!("{key}: {}", patterns.join(", ")));
            }
            _ => checks.warn(
                "source-mapping",
                "SOURCE_MAPPING_MISSING",
                format!("packageSourceMapping no asigna ningún patrón a {key}"),
                &init,
            ),
        },
    }
}

fn read_doc(path: &Path) -> Result<Document, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Document::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn clone_err(e: &ClientError) -> ClientError {
    match e {
        ClientError::Api { status, detail } => ClientError::Api {
            status: *status,
            detail: detail.clone(),
        },
        ClientError::Transport { kind, message } => ClientError::Transport {
            kind: *kind,
            message: message.clone(),
        },
        ClientError::InvalidResponse(m) => ClientError::InvalidResponse(m.clone()),
        ClientError::Config(m) => ClientError::Config(m.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_and_port() {
        assert_eq!(
            host_port("https://packages.example.com/x"),
            Some(("packages.example.com".into(), 443))
        );
        assert_eq!(
            host_port("http://127.0.0.1:8080"),
            Some(("127.0.0.1".into(), 8080))
        );
        assert_eq!(host_port("http://[::1]:9000/"), Some(("::1".into(), 9000)));
        assert_eq!(host_port("nada"), None);
    }
}
