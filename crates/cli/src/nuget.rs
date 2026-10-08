//! `onepack nuget init` y `onepack exec` (ADR-016): configuración de NuGet sin secretos y
//! credenciales solo en el entorno del proceso hijo.

use std::path::{Path, PathBuf};
use std::process::Command;

use onepack_api_client::WhoAmI;
use serde::Serialize;

use crate::app::{App, DEFAULT_TIMEOUT, Target};
use crate::error::{CliError, CliResult, exit};
use crate::nuget_config::{Document, diff, source_key};
use crate::{ExecArgs, NugetCommand};

/// Nombres con los que `dotnet` y `nuget.exe` encuentran el archivo, en orden de preferencia.
const CONFIG_NAMES: [&str; 3] = ["NuGet.Config", "nuget.config", "NuGet.config"];

/// `NuGet.Config` del directorio, si existe con alguno de sus nombres habituales.
pub fn find_config_in(dir: &Path) -> Option<PathBuf> {
    CONFIG_NAMES
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

pub fn feed_url(base: &str, feed: &str) -> String {
    format!("{base}/nuget/{feed}/v3/index.json")
}

#[derive(Serialize)]
struct SourceView {
    key: String,
    url: String,
}

#[derive(Serialize)]
struct InitResult {
    path: String,
    created: bool,
    changed: bool,
    written: bool,
    sources: Vec<SourceView>,
    diff: String,
    notes: Vec<String>,
}

pub fn init(app: &App, cmd: NugetCommand) -> CliResult<i32> {
    let NugetCommand::Init {
        feeds,
        patterns,
        config,
        dry_run,
        yes,
    } = cmd;
    let target = app.target()?;
    let cwd = std::env::current_dir().map_err(|e| CliError::io("current directory", &e))?;
    let path = config
        .or_else(|| find_config_in(&cwd))
        .unwrap_or_else(|| cwd.join(CONFIG_NAMES[0]));
    let created = !path.exists();
    let old = if created {
        String::new()
    } else {
        std::fs::read_to_string(&path).map_err(|e| CliError::io(&path.display().to_string(), &e))?
    };
    let mut doc = if created {
        Document::new_config()
    } else {
        Document::parse(&old).map_err(|e| {
            CliError::new(
                exit::USAGE,
                "INVALID_NUGET_CONFIG",
                format!("{}: {e}", path.display()),
            )
        })?
    };
    if doc.root().is_none() {
        return Err(CliError::new(
            exit::USAGE,
            "INVALID_NUGET_CONFIG",
            format!("{} has no <configuration> element", path.display()),
        ));
    }

    let mut notes = Vec::new();
    let mut sources = Vec::new();
    for feed in &feeds {
        let key = source_key(feed);
        let url = feed_url(&target.url, feed);
        let insecure = url.starts_with("http://");
        notes.extend(doc.upsert_source(&key, &url, &patterns, insecure));
        if insecure {
            notes.push(format!(
                "{key} uses HTTP without TLS (allowInsecureConnections): use it only locally"
            ));
        }
        sources.push(SourceView { key, url });
    }
    for key in doc.cleartext_credentials() {
        notes.push(format!(
            "{} contains a plain-text password for {key}; delete it and use `onepack exec` or environment variables",
            path.display()
        ));
    }
    let new = doc.render();
    let changes = diff(&old, &new);
    let changed = !changes.is_empty();

    if !app.out.json {
        for note in &notes {
            app.out.note(format!("warning: {note}"));
        }
        if changed {
            println!("--- {}\n{changes}", path.display());
        } else {
            app.out
                .note(format!("{} is already up to date", path.display()));
        }
    }
    let mut written = false;
    if changed && !dry_run {
        app.out
            .confirm(&format!("Write {}?", path.display()), yes)?;
        std::fs::write(&path, &new).map_err(|e| CliError::io(&path.display().to_string(), &e))?;
        written = true;
        if !app.out.json {
            app.out.note(format!(
                "{} updated. Restore with: onepack exec --feed {} -- dotnet restore",
                path.display(),
                feeds.join(" --feed ")
            ));
        }
    }
    if app.out.json {
        app.out.data(
            &InitResult {
                path: path.display().to_string(),
                created,
                changed,
                written,
                sources,
                diff: changes,
                notes,
            },
            String::new,
        );
    }
    Ok(exit::OK)
}

/// Variable con la que NuGet lee las credenciales de una fuente.
pub fn credential_var(source: &str) -> String {
    format!("NuGetPackageSourceCredentials_{source}")
}

/// Comprueba la credencial antes de lanzar el comando. Con un token revocado o caducado, o sin
/// acceso al feed, NuGet solo dice que no puede cargar el índice de servicio (NU1301).
/// Si el servidor no responde, se avisa y se sigue: el comando puede tirar de la caché.
fn check_access(app: &App, target: &Target, token: &str, feeds: &[String]) -> CliResult<()> {
    let me: WhoAmI = match app
        .client_for(target, Some(token.to_owned()), DEFAULT_TIMEOUT)?
        .get("/whoami")
    {
        Ok(me) => me,
        Err(e) => {
            let e = CliError::from(e);
            if e.exit == exit::AUTH {
                return Err(e.with_action(
                    "the credential is not valid (revoked, expired, or belonging to a disabled principal); \
                     ask an administrator for a new token",
                ));
            }
            app.out.note(format!(
                "warning: could not check the credential ({}); running anyway",
                e.code
            ));
            return Ok(());
        }
    };
    if me.administrator {
        return Ok(());
    }
    for feed in feeds {
        if !me.grants.iter().any(|g| &g.feed == feed) {
            return Err(CliError::new(
                exit::FORBIDDEN,
                "AUTH_SCOPE_MISSING",
                format!("{} has no access to feed {feed} (or the feed does not exist)", me.principal),
            )
            .with_action(format!(
                "ask an administrator to run `onepack grant add --principal {} --feed {feed} --role reader`",
                me.principal
            )));
        }
    }
    Ok(())
}

pub fn exec(app: &App, args: ExecArgs) -> CliResult<i32> {
    if args.source_name.is_some() && args.feeds.len() > 1 {
        return Err(CliError::usage("--source-name accepts only one --feed"));
    }
    let target = app.target()?;
    let credential = app.credential(&target)?.ok_or_else(|| {
        CliError::new(
            exit::AUTH,
            "AUTH_REQUIRED",
            "there is no credential for this context",
        )
        .with_action("run `onepack login` or use --token-env <VAR>")
    })?;
    let (program, rest) = args
        .command
        .split_first()
        .ok_or_else(|| CliError::usage("missing the command to run"))?;
    check_access(app, &target, &credential.token, &args.feeds)?;

    let mut child = Command::new(program);
    child.args(rest);
    for feed in &args.feeds {
        let source = args.source_name.clone().unwrap_or_else(|| source_key(feed));
        // Solo en el entorno del hijo: ni el shell padre ni ningún archivo lo ven.
        child.env(
            credential_var(&source),
            format!("Username=onepack;Password={}", credential.token),
        );
    }
    let status = child.status().map_err(|e| {
        CliError::new(
            exit::ERROR,
            "EXEC_FAILED",
            format!("could not run {program:?}: {e}"),
        )
    })?;
    Ok(status.code().unwrap_or_else(|| {
        // Terminado por una señal: convención de las shells (128 + señal).
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            status.signal().map_or(exit::ERROR, |s| 128 + s)
        }
        #[cfg(not(unix))]
        {
            exit::ERROR
        }
    }))
}
