//! Subcomandos de administración, paquetes y auditoría. Cada uno devuelve el código de
//! salida; los datos salen por `Out::data` (texto o JSON).

use std::time::Duration;

use onepack_api_client::client::api_error;
use onepack_api_client::{
    self as api, AuditEvent, ConfigureFeed, CreateFeed, CreatePrincipal, CreateToken, Grant,
    IssuedToken, PackageSummary, PackageVersion, Principal, Reason, SetGrant, Token, VersionState,
    WhoAmI, capability,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::app::{App, seg};
use crate::config::{Context, validate_name};
use crate::credentials;
use crate::error::{CliError, CliResult, exit};
use crate::output::{bytes, opt, table};
use crate::{
    AuditCommand, ContextCommand, FeedCommand, GrantCommand, KindArg, PackageCommand,
    PrincipalCommand, RoleArg, TokenCommand, VersionArgs,
};

const PUSH_TIMEOUT: Duration = Duration::from_secs(600);

// ---------------------------------------------------------------------------------------------
// Contextos y sesión
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct ContextView {
    name: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ca_cert: Option<String>,
    current: bool,
}

pub fn context(mut app: App, cmd: ContextCommand) -> CliResult<i32> {
    match cmd {
        ContextCommand::Add {
            name,
            url,
            ca_cert,
            make_current,
        } => {
            validate_name(&name)?;
            let url = url.trim_end_matches('/').to_owned();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(CliError::usage(format!(
                    "invalid URL {url:?}: it must start with https:// or http://"
                )));
            }
            if url.starts_with("http://") {
                app.out
                    .note("warning: HTTP without TLS exposes the token on the network; use it only locally");
            }
            app.config.contexts.insert(
                name.clone(),
                Context {
                    url: url.clone(),
                    ca_cert: ca_cert.clone(),
                },
            );
            if make_current || app.config.current.is_none() {
                app.config.current = Some(name.clone());
            }
            app.config.save()?;
            let view = ContextView {
                current: app.config.current.as_deref() == Some(name.as_str()),
                name,
                url,
                ca_cert: ca_cert.map(|p| p.display().to_string()),
            };
            app.out.data(&view, || {
                format!(
                    "context {} -> {}{}",
                    view.name,
                    view.url,
                    if view.current { " (current)" } else { "" }
                )
            });
        }
        ContextCommand::List => {
            let views: Vec<ContextView> = app
                .config
                .contexts
                .iter()
                .map(|(name, c)| ContextView {
                    name: name.clone(),
                    url: c.url.clone(),
                    ca_cert: c.ca_cert.as_ref().map(|p| p.display().to_string()),
                    current: app.config.current.as_deref() == Some(name.as_str()),
                })
                .collect();
            app.out.data(&views, || {
                table(
                    &["", "NAME", "URL"],
                    views
                        .iter()
                        .map(|v| {
                            vec![
                                if v.current { "*" } else { "" }.to_owned(),
                                v.name.clone(),
                                v.url.clone(),
                            ]
                        })
                        .collect(),
                )
            });
        }
        ContextCommand::Use { name } => {
            if !app.config.contexts.contains_key(&name) {
                return Err(CliError::new(
                    exit::NOT_FOUND,
                    "CONTEXT_MISSING",
                    format!("context {name:?} does not exist"),
                ));
            }
            app.config.current = Some(name.clone());
            app.config.save()?;
            app.out.data(&serde_json::json!({ "current": name }), || {
                format!("current context: {name}")
            });
        }
        ContextCommand::Remove { name, yes } => {
            if !app.config.contexts.contains_key(&name) {
                return Err(CliError::new(
                    exit::NOT_FOUND,
                    "CONTEXT_MISSING",
                    format!("context {name:?} does not exist"),
                ));
            }
            app.out
                .confirm(&format!("Remove context {name} and its token?"), yes)?;
            // Sin keychain no hay token que borrar; el contexto se elimina igualmente.
            let removed_token = credentials::delete(&name).unwrap_or(false);
            app.config.contexts.remove(&name);
            if app.config.current.as_deref() == Some(name.as_str()) {
                app.config.current = None;
            }
            app.config.save()?;
            app.out.data(
                &serde_json::json!({ "name": name, "removed_token": removed_token }),
                || format!("context {name} removed"),
            );
        }
    }
    Ok(exit::OK)
}

pub fn login(app: &App, token_stdin: bool) -> CliResult<i32> {
    let target = app.target()?;
    let name = target.name.clone().ok_or_else(|| {
        CliError::usage("login stores the token of a context; it cannot be used with --url")
            .with_action("create a context with `onepack context add`")
    })?;
    // Si no hay keychain, se falla antes de pedir el token (ADR-015: sin texto plano).
    credentials::ensure_available(&name)?;
    let token = app.out.read_secret(
        &format!("Token for {} ({}): ", name, target.url),
        token_stdin,
    )?;
    // Se valida antes de guardarlo.
    let me: WhoAmI = app
        .client_for(&target, Some(token.clone()), crate::app::DEFAULT_TIMEOUT)?
        .get("/whoami")?;
    credentials::store(&name, &token)?;
    let view = serde_json::json!({
        "context": name,
        "principal": me.principal,
        "token_id": me.token_id,
        "token_expires_at": me.token_expires_at,
        "stored_in": "keychain",
    });
    app.out.data(&view, || {
        format!(
            "logged in to {name} as {} (token {}, expires {})",
            me.principal,
            me.token_id,
            opt(&me.token_expires_at)
        )
    });
    Ok(exit::OK)
}

pub fn logout(app: &App) -> CliResult<i32> {
    let target = app.target()?;
    let name = target
        .name
        .ok_or_else(|| CliError::usage("logout needs a context; it cannot be used with --url"))?;
    let removed = credentials::delete(&name)?;
    app.out.data(
        &serde_json::json!({ "context": name, "removed": removed }),
        || {
            if removed {
                format!("token for {name} removed from the keychain")
            } else {
                format!("{name} had no token in the keychain")
            }
        },
    );
    Ok(exit::OK)
}

pub fn whoami(app: &App) -> CliResult<i32> {
    let me: WhoAmI = app.client()?.get("/whoami")?;
    app.out.data(&me, || {
        let mut lines = vec![
            format!(
                "{} ({}{})",
                me.principal,
                me.kind,
                if me.administrator {
                    ", administrator"
                } else {
                    ""
                }
            ),
            format!(
                "token {} (expires {})",
                me.token_id,
                opt(&me.token_expires_at)
            ),
        ];
        for g in &me.grants {
            let patterns = if g.publish_patterns.is_empty() {
                String::new()
            } else {
                format!(" [{}]", g.publish_patterns.join(", "))
            };
            lines.push(format!("  {}: {}{patterns}", g.feed, g.role));
        }
        lines.join("\n")
    });
    Ok(exit::OK)
}

// ---------------------------------------------------------------------------------------------
// Feeds
// ---------------------------------------------------------------------------------------------

fn feed_text(f: &api::Feed) -> String {
    let limit = |v: Option<u64>, fmt: fn(u64) -> String| v.map_or("no limit".to_owned(), fmt);
    [
        format!("feed:            {}", f.name),
        format!("created:         {}", f.created_at),
        format!(
            "versions:        {} (quota: {})",
            f.versions,
            limit(f.max_versions, |v| v.to_string())
        ),
        format!(
            "storage:         {} (quota: {})",
            bytes(f.storage_bytes),
            limit(f.max_storage_bytes, bytes)
        ),
    ]
    .join("\n")
}

pub fn feed(app: &App, cmd: FeedCommand) -> CliResult<i32> {
    let client = app.client()?;
    app.require(&client, capability::FEEDS)?;
    match cmd {
        FeedCommand::Create { name } => {
            let f: api::Feed = client.post("/feeds", &CreateFeed { name })?;
            app.out.data(&f, || format!("feed created: {}", f.name));
        }
        FeedCommand::List => {
            let feeds: Vec<api::Feed> = client.list_all("/feeds")?;
            app.out.data(&feeds, || {
                table(
                    &["FEED", "VERSIONS", "SIZE"],
                    feeds
                        .iter()
                        .map(|f| {
                            vec![
                                f.name.clone(),
                                f.versions.to_string(),
                                bytes(f.storage_bytes),
                            ]
                        })
                        .collect(),
                )
            });
        }
        FeedCommand::Show { name } => {
            let f: api::Feed = client.get(&format!("/feeds/{}", seg(&name)))?;
            app.out.data(&f, || feed_text(&f));
        }
        FeedCommand::Configure {
            name,
            max_storage_mib,
            max_versions,
        } => {
            app.require(&client, capability::FEED_QUOTAS)?;
            let path = format!("/feeds/{}", seg(&name));
            let current: api::Feed = client.get(&path)?;
            let quota = ConfigureFeed {
                max_storage_bytes: match max_storage_mib {
                    Some(0) => None,
                    Some(mib) => Some(mib << 20),
                    None => current.max_storage_bytes,
                },
                max_versions: match max_versions {
                    Some(0) => None,
                    Some(n) => Some(n),
                    None => current.max_versions,
                },
            };
            let f: api::Feed = client.patch(&path, &quota)?;
            app.out.data(&f, || feed_text(&f));
        }
    }
    Ok(exit::OK)
}

// ---------------------------------------------------------------------------------------------
// Principals, tokens y grants
// ---------------------------------------------------------------------------------------------

fn principal_row(p: &Principal) -> Vec<String> {
    vec![
        p.name.clone(),
        p.kind.clone(),
        if p.administrator { "yes" } else { "no" }.to_owned(),
        if p.disabled { "disabled" } else { "active" }.to_owned(),
    ]
}

pub fn principal(app: &App, cmd: PrincipalCommand) -> CliResult<i32> {
    let client = app.client()?;
    app.require(&client, capability::PRINCIPALS)?;
    match cmd {
        PrincipalCommand::Create { name, kind, admin } => {
            let p: Principal = client.post(
                "/principals",
                &CreatePrincipal {
                    name,
                    kind: match kind {
                        KindArg::User => "user",
                        KindArg::Service => "service",
                    }
                    .to_owned(),
                    administrator: admin,
                },
            )?;
            app.out
                .data(&p, || format!("principal created: {} ({})", p.name, p.kind));
        }
        PrincipalCommand::List => {
            let list: Vec<Principal> = client.list_all("/principals")?;
            app.out.data(&list, || {
                table(
                    &["NAME", "KIND", "ADMIN", "STATUS"],
                    list.iter().map(principal_row).collect(),
                )
            });
        }
        PrincipalCommand::Disable { name, yes } => {
            app.out.confirm(
                &format!("Disable {name}? Its tokens will stop working"),
                yes,
            )?;
            let p: Principal = client.post(
                &format!("/principals/{}/disable", seg(&name)),
                &serde_json::json!({}),
            )?;
            app.out.data(&p, || format!("{} disabled", p.name));
        }
    }
    Ok(exit::OK)
}

pub fn token(app: &App, cmd: TokenCommand) -> CliResult<i32> {
    let client = app.client()?;
    app.require(&client, capability::TOKENS)?;
    match cmd {
        TokenCommand::Create {
            principal,
            name,
            expires_in_days,
        } => {
            let t: IssuedToken = client.post(
                "/tokens",
                &CreateToken {
                    principal,
                    name,
                    expires_in_days,
                },
            )?;
            if !app.out.json {
                app.out.note(format!(
                    "token {} for {} (expires {}); it will not be shown again",
                    t.id, t.principal, t.expires_at
                ));
            }
            // En texto, solo el token va a stdout para poder capturarlo.
            app.out.data(&t, || t.token.clone());
        }
        TokenCommand::List { principal } => {
            let path = match &principal {
                Some(p) => format!("/tokens?principal={}", seg(p)),
                None => "/tokens".to_owned(),
            };
            let list: Vec<Token> = client.list_all(&path)?;
            app.out.data(&list, || {
                table(
                    &["ID", "PRINCIPAL", "NAME", "EXPIRES", "LAST USED", "REVOKED"],
                    list.iter()
                        .map(|t| {
                            vec![
                                t.id.clone(),
                                t.principal.clone(),
                                opt(&t.name),
                                t.expires_at.clone(),
                                opt(&t.last_used_at),
                                opt(&t.revoked_at),
                            ]
                        })
                        .collect(),
                )
            });
        }
        TokenCommand::Revoke { id, yes } => {
            app.out.confirm(&format!("Revoke token {id}?"), yes)?;
            let t: Token = client.post(
                &format!("/tokens/{}/revoke", seg(&id)),
                &serde_json::json!({}),
            )?;
            app.out.data(&t, || format!("token {} revoked", t.id));
        }
    }
    Ok(exit::OK)
}

fn grant_row(g: &Grant) -> Vec<String> {
    vec![
        g.feed.clone(),
        g.principal.clone(),
        g.role.clone(),
        if g.publish_patterns.is_empty() {
            "*".to_owned()
        } else {
            g.publish_patterns.join(", ")
        },
    ]
}

pub fn grant(app: &App, cmd: GrantCommand) -> CliResult<i32> {
    let client = app.client()?;
    app.require(&client, capability::GRANTS)?;
    match cmd {
        GrantCommand::Add {
            principal,
            feed,
            role,
            publish_patterns,
        } => {
            let g: Grant = client.put(
                &format!("/feeds/{}/grants/{}", seg(&feed), seg(&principal)),
                &SetGrant {
                    role: match role {
                        RoleArg::Reader => "reader",
                        RoleArg::Publisher => "publisher",
                        RoleArg::Maintainer => "maintainer",
                    }
                    .to_owned(),
                    publish_patterns,
                },
            )?;
            app.out.data(&g, || {
                format!("{} is {} on {}", g.principal, g.role, g.feed)
            });
        }
        GrantCommand::Remove {
            principal,
            feed,
            yes,
        } => {
            app.out
                .confirm(&format!("Remove {principal}'s access to {feed}?"), yes)?;
            let _: serde_json::Value =
                client.delete(&format!("/feeds/{}/grants/{}", seg(&feed), seg(&principal)))?;
            app.out.data(
                &serde_json::json!({ "principal": principal, "feed": feed, "removed": true }),
                || format!("{principal}'s access to {feed} removed"),
            );
        }
        GrantCommand::List { principal, feed } => {
            let mut query = Vec::new();
            if let Some(p) = &principal {
                query.push(format!("principal={}", seg(p)));
            }
            if let Some(f) = &feed {
                query.push(format!("feed={}", seg(f)));
            }
            let path = if query.is_empty() {
                "/grants".to_owned()
            } else {
                format!("/grants?{}", query.join("&"))
            };
            let list: Vec<Grant> = client.list_all(&path)?;
            app.out.data(&list, || {
                table(
                    &["FEED", "PRINCIPAL", "ROLE", "PUBLISHES"],
                    list.iter().map(grant_row).collect(),
                )
            });
        }
    }
    Ok(exit::OK)
}

// ---------------------------------------------------------------------------------------------
// Paquetes
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct PushResult {
    file: String,
    id: String,
    version: String,
    sha256: String,
    /// `pushed` o `skipped` (ya existía con el mismo contenido).
    status: &'static str,
}

fn version_text(v: &PackageVersion) -> String {
    let mut lines = vec![
        format!("{} {} ({})", v.id, v.version, v.feed),
        format!("listed:          {}", if v.listed { "yes" } else { "no" }),
        format!("availability:    {}", v.availability),
    ];
    if let Some(reason) = &v.blocked_reason {
        lines.push(format!("reason:          {reason}"));
    }
    lines.push(format!("sha256:          {}", v.sha256));
    lines.push(format!("size:            {}", bytes(v.size)));
    lines.push(format!("published:       {}", v.published_at));
    lines.join("\n")
}

fn state_text(s: &VersionState, verb: &str) -> String {
    if s.changed {
        format!("{} {} {verb}", s.id, s.version)
    } else {
        format!("{} {} was already {verb}", s.id, s.version)
    }
}

fn version_path(v: &VersionArgs) -> String {
    format!(
        "/feeds/{}/packages/{}/{}",
        seg(&v.feed),
        seg(&v.id),
        seg(&v.version)
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn package(app: &App, cmd: PackageCommand) -> CliResult<i32> {
    match cmd {
        PackageCommand::List { feed } => {
            let client = app.client()?;
            app.require(&client, capability::PACKAGES)?;
            let list: Vec<PackageSummary> =
                client.list_all(&format!("/feeds/{}/packages", seg(&feed)))?;
            app.out.data(&list, || {
                table(
                    &["PACKAGE", "VERSIONS", "LATEST"],
                    list.iter()
                        .map(|p| {
                            vec![
                                p.id.clone(),
                                p.versions.to_string(),
                                p.latest_version.clone(),
                            ]
                        })
                        .collect(),
                )
            });
        }
        PackageCommand::Inspect { feed, id, version } => {
            let client = app.client()?;
            app.require(&client, capability::PACKAGES)?;
            let base = format!("/feeds/{}/packages/{}", seg(&feed), seg(&id));
            match version {
                Some(version) => {
                    let v: PackageVersion = client.get(&format!("{base}/{}", seg(&version)))?;
                    app.out.data(&v, || version_text(&v));
                }
                None => {
                    let list: Vec<PackageVersion> = client.get(&base)?;
                    app.out.data(&list, || {
                        table(
                            &["VERSION", "LISTED", "AVAILABILITY", "SIZE", "PUBLISHED"],
                            list.iter()
                                .map(|v| {
                                    vec![
                                        v.version.clone(),
                                        if v.listed { "yes" } else { "no" }.to_owned(),
                                        v.availability.clone(),
                                        bytes(v.size),
                                        v.published_at.clone(),
                                    ]
                                })
                                .collect(),
                        )
                    });
                }
            }
        }
        PackageCommand::Push {
            feed,
            files,
            skip_existing_identical,
        } => {
            let client = app.client_with_timeout(PUSH_TIMEOUT)?;
            let mut results = Vec::new();
            for file in &files {
                let bytes = std::fs::read(file)
                    .map_err(|e| CliError::io(&file.display().to_string(), &e))?;
                // Validación local: se informa antes de subir nada.
                let manifest = onepack_nuget::read_package(&bytes).map_err(|e| {
                    CliError::new(exit::USAGE, e.code(), format!("{}: {e}", file.display()))
                })?;
                let id = manifest.id.as_str().to_owned();
                let version = manifest.version.normalized();
                let sha256 = sha256_hex(&bytes);
                let res = client.nuget_push(&feed, &bytes)?;
                let status = match res.status {
                    200..=299 => "pushed",
                    409 if skip_existing_identical => {
                        let existing: PackageVersion = client.get(&format!(
                            "/feeds/{}/packages/{}/{}",
                            seg(&feed),
                            seg(&id),
                            seg(&version)
                        ))?;
                        if existing.sha256 != sha256 {
                            return Err(CliError::new(
                                exit::CONFLICT,
                                "PACKAGE_VERSION_EXISTS",
                                format!(
                                    "{id} {version} already exists in {feed} with different content"
                                ),
                            )
                            .with_action("publish a new version: versions are immutable"));
                        }
                        "skipped"
                    }
                    _ => return Err(api_error(&res).into()),
                };
                if !app.out.json {
                    app.out.note(match status {
                        "pushed" => format!("pushed {id} {version} to {feed}"),
                        _ => format!(
                            "skipped {id} {version}: it already exists with the same content"
                        ),
                    });
                }
                results.push(PushResult {
                    file: file.display().to_string(),
                    id,
                    version,
                    sha256,
                    status,
                });
            }
            app.out.data(&results, String::new);
        }
        PackageCommand::Unlist(v) => {
            let client = app.client()?;
            app.require(&client, capability::PACKAGE_LISTING)?;
            let s: VersionState = client.post(
                &format!("{}/unlist", version_path(&v)),
                &serde_json::json!({}),
            )?;
            app.out.data(&s, || state_text(&s, "unlisted"));
        }
        PackageCommand::Relist(v) => {
            let client = app.client()?;
            app.require(&client, capability::PACKAGE_LISTING)?;
            let s: VersionState = client.post(
                &format!("{}/relist", version_path(&v)),
                &serde_json::json!({}),
            )?;
            app.out.data(&s, || state_text(&s, "listed"));
        }
        PackageCommand::Block {
            version,
            reason,
            yes,
        } => {
            let client = app.client()?;
            app.require(&client, capability::PACKAGE_AVAILABILITY)?;
            app.out.confirm(
                &format!(
                    "Block downloads of {} {} in {}?",
                    version.id, version.version, version.feed
                ),
                yes,
            )?;
            let s: VersionState = client.post(
                &format!("{}/block", version_path(&version)),
                &Reason { reason },
            )?;
            app.out.data(&s, || state_text(&s, "blocked"));
        }
        PackageCommand::Unblock { version, reason } => {
            let client = app.client()?;
            app.require(&client, capability::PACKAGE_AVAILABILITY)?;
            let s: VersionState = client.post(
                &format!("{}/unblock", version_path(&version)),
                &Reason { reason },
            )?;
            app.out.data(&s, || state_text(&s, "available"));
        }
    }
    Ok(exit::OK)
}

// ---------------------------------------------------------------------------------------------
// Auditoría
// ---------------------------------------------------------------------------------------------

pub fn audit(app: &App, cmd: AuditCommand) -> CliResult<i32> {
    let AuditCommand::List {
        feed,
        action,
        limit,
    } = cmd;
    let client = app.client()?;
    app.require(&client, capability::AUDIT)?;
    let mut query = vec![format!(
        "limit={}",
        limit.clamp(1, api::MAX_PAGE_SIZE as usize)
    )];
    if let Some(f) = &feed {
        query.push(format!("feed={}", seg(f)));
    }
    if let Some(a) = &action {
        query.push(format!("action={}", seg(a)));
    }
    // Solo la primera página: `--limit` es el número de eventos que se quieren ver.
    let page: api::Page<AuditEvent> = client.get(&format!("/audit?{}", query.join("&")))?;
    let events = page.items;
    app.out.data(&events, || {
        table(
            &["DATE", "ACTOR", "ACTION", "FEED", "RESOURCE", "OUTCOME"],
            events
                .iter()
                .map(|e| {
                    vec![
                        e.occurred_at.clone(),
                        opt(&e.actor),
                        e.action.clone(),
                        opt(&e.feed),
                        opt(&e.resource),
                        e.outcome.clone(),
                    ]
                })
                .collect(),
        )
    });
    Ok(exit::OK)
}
