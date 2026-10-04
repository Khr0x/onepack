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
                    "URL inválida {url:?}: debe empezar por https:// o http://"
                )));
            }
            if url.starts_with("http://") {
                app.out
                    .note("aviso: HTTP sin TLS expone el token en la red; úsalo solo en local");
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
                    "contexto {} -> {}{}",
                    view.name,
                    view.url,
                    if view.current { " (actual)" } else { "" }
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
                    &["", "NOMBRE", "URL"],
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
                    format!("no existe el contexto {name:?}"),
                ));
            }
            app.config.current = Some(name.clone());
            app.config.save()?;
            app.out.data(&serde_json::json!({ "current": name }), || {
                format!("contexto actual: {name}")
            });
        }
        ContextCommand::Remove { name, yes } => {
            if !app.config.contexts.contains_key(&name) {
                return Err(CliError::new(
                    exit::NOT_FOUND,
                    "CONTEXT_MISSING",
                    format!("no existe el contexto {name:?}"),
                ));
            }
            app.out
                .confirm(&format!("¿Eliminar el contexto {name} y su token?"), yes)?;
            // Sin keychain no hay token que borrar; el contexto se elimina igualmente.
            let removed_token = credentials::delete(&name).unwrap_or(false);
            app.config.contexts.remove(&name);
            if app.config.current.as_deref() == Some(name.as_str()) {
                app.config.current = None;
            }
            app.config.save()?;
            app.out.data(
                &serde_json::json!({ "name": name, "removed_token": removed_token }),
                || format!("contexto {name} eliminado"),
            );
        }
    }
    Ok(exit::OK)
}

pub fn login(app: &App, token_stdin: bool) -> CliResult<i32> {
    let target = app.target()?;
    let name = target.name.clone().ok_or_else(|| {
        CliError::usage("login guarda el token de un contexto; no se puede usar con --url")
            .with_action("crea un contexto con `onepack context add`")
    })?;
    // Si no hay keychain, se falla antes de pedir el token (ADR-015: sin texto plano).
    credentials::ensure_available(&name)?;
    let token = app.out.read_secret(
        &format!("Token para {} ({}): ", name, target.url),
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
            "sesión iniciada en {name} como {} (token {}, caduca {})",
            me.principal,
            me.token_id,
            opt(&me.token_expires_at)
        )
    });
    Ok(exit::OK)
}

pub fn logout(app: &App) -> CliResult<i32> {
    let target = app.target()?;
    let name = target.name.ok_or_else(|| {
        CliError::usage("logout requiere un contexto; no se puede usar con --url")
    })?;
    let removed = credentials::delete(&name)?;
    app.out.data(
        &serde_json::json!({ "context": name, "removed": removed }),
        || {
            if removed {
                format!("token de {name} eliminado del keychain")
            } else {
                format!("{name} no tenía token en el keychain")
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
                    ", administrador"
                } else {
                    ""
                }
            ),
            format!(
                "token {} (caduca {})",
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
    let limit = |v: Option<u64>, fmt: fn(u64) -> String| v.map_or("sin límite".to_owned(), fmt);
    [
        format!("feed:            {}", f.name),
        format!("creado:          {}", f.created_at),
        format!(
            "versiones:       {} (cuota: {})",
            f.versions,
            limit(f.max_versions, |v| v.to_string())
        ),
        format!(
            "almacenamiento:  {} (cuota: {})",
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
            app.out.data(&f, || format!("feed creado: {}", f.name));
        }
        FeedCommand::List => {
            let feeds: Vec<api::Feed> = client.list_all("/feeds")?;
            app.out.data(&feeds, || {
                table(
                    &["FEED", "VERSIONES", "TAMAÑO"],
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
        if p.administrator { "sí" } else { "no" }.to_owned(),
        if p.disabled { "desactivado" } else { "activo" }.to_owned(),
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
                .data(&p, || format!("principal creado: {} ({})", p.name, p.kind));
        }
        PrincipalCommand::List => {
            let list: Vec<Principal> = client.list_all("/principals")?;
            app.out.data(&list, || {
                table(
                    &["NOMBRE", "TIPO", "ADMIN", "ESTADO"],
                    list.iter().map(principal_row).collect(),
                )
            });
        }
        PrincipalCommand::Disable { name, yes } => {
            app.out.confirm(
                &format!("¿Desactivar {name}? Sus tokens dejarán de funcionar"),
                yes,
            )?;
            let p: Principal = client.post(
                &format!("/principals/{}/disable", seg(&name)),
                &serde_json::json!({}),
            )?;
            app.out.data(&p, || format!("{} desactivado", p.name));
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
                    "token {} para {} (caduca {}); no se volverá a mostrar",
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
                    &[
                        "ID",
                        "PRINCIPAL",
                        "NOMBRE",
                        "CADUCA",
                        "ÚLTIMO USO",
                        "REVOCADO",
                    ],
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
            app.out.confirm(&format!("¿Revocar el token {id}?"), yes)?;
            let t: Token = client.post(
                &format!("/tokens/{}/revoke", seg(&id)),
                &serde_json::json!({}),
            )?;
            app.out.data(&t, || format!("token {} revocado", t.id));
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
                format!("{} es {} en {}", g.principal, g.role, g.feed)
            });
        }
        GrantCommand::Remove {
            principal,
            feed,
            yes,
        } => {
            app.out
                .confirm(&format!("¿Quitar el acceso de {principal} a {feed}?"), yes)?;
            let _: serde_json::Value =
                client.delete(&format!("/feeds/{}/grants/{}", seg(&feed), seg(&principal)))?;
            app.out.data(
                &serde_json::json!({ "principal": principal, "feed": feed, "removed": true }),
                || format!("acceso de {principal} a {feed} eliminado"),
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
                    &["FEED", "PRINCIPAL", "ROL", "PUBLICA"],
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
        format!("listado:         {}", if v.listed { "sí" } else { "no" }),
        format!("disponibilidad:  {}", v.availability),
    ];
    if let Some(reason) = &v.blocked_reason {
        lines.push(format!("motivo:          {reason}"));
    }
    lines.push(format!("sha256:          {}", v.sha256));
    lines.push(format!("tamaño:          {}", bytes(v.size)));
    lines.push(format!("publicado:       {}", v.published_at));
    lines.join("\n")
}

fn state_text(s: &VersionState, verb: &str) -> String {
    if s.changed {
        format!("{} {} {verb}", s.id, s.version)
    } else {
        format!("{} {} ya estaba {verb}", s.id, s.version)
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
                    &["PAQUETE", "VERSIONES", "ÚLTIMA"],
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
                            &[
                                "VERSIÓN",
                                "LISTADA",
                                "DISPONIBILIDAD",
                                "TAMAÑO",
                                "PUBLICADA",
                            ],
                            list.iter()
                                .map(|v| {
                                    vec![
                                        v.version.clone(),
                                        if v.listed { "sí" } else { "no" }.to_owned(),
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
                                    "{id} {version} ya existe en {feed} con contenido distinto"
                                ),
                            )
                            .with_action(
                                "publica una versión nueva: las versiones son inmutables",
                            ));
                        }
                        "skipped"
                    }
                    _ => return Err(api_error(&res).into()),
                };
                if !app.out.json {
                    app.out.note(match status {
                        "pushed" => format!("publicado {id} {version} en {feed}"),
                        _ => format!("omitido {id} {version}: ya existe con el mismo contenido"),
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
            app.out.data(&s, || state_text(&s, "oculta"));
        }
        PackageCommand::Relist(v) => {
            let client = app.client()?;
            app.require(&client, capability::PACKAGE_LISTING)?;
            let s: VersionState = client.post(
                &format!("{}/relist", version_path(&v)),
                &serde_json::json!({}),
            )?;
            app.out.data(&s, || state_text(&s, "listada"));
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
                    "¿Bloquear la descarga de {} {} en {}?",
                    version.id, version.version, version.feed
                ),
                yes,
            )?;
            let s: VersionState = client.post(
                &format!("{}/block", version_path(&version)),
                &Reason { reason },
            )?;
            app.out.data(&s, || state_text(&s, "bloqueada"));
        }
        PackageCommand::Unblock { version, reason } => {
            let client = app.client()?;
            app.require(&client, capability::PACKAGE_AVAILABILITY)?;
            let s: VersionState = client.post(
                &format!("{}/unblock", version_path(&version)),
                &Reason { reason },
            )?;
            app.out.data(&s, || state_text(&s, "disponible"));
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
            &["FECHA", "ACTOR", "ACCIÓN", "FEED", "RECURSO", "RESULTADO"],
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
