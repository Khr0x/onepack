//! Pruebas bloqueantes de la Fase 6 para el CLI (ADR-015, ADR-016): el binario `onepack`
//! contra un `onepackd` real en proceso.

use std::io::{Cursor, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

use onepack_core::{FeedName, PrincipalKind, PrincipalName, Role};
use onepack_server::{Limits, app};
use onepack_storage::{Store, migrate};
use serde_json::{Value, json};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

struct Server {
    url: String,
    _dir: TempDir,
    admin: String,
    reader: String,
}

/// `onepackd` con un feed `internal`, un administrador y un lector de ese feed.
fn server() -> Server {
    let dir = TempDir::new().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (store, admin, reader) = rt.block_on(async {
        migrate(dir.path()).await.unwrap();
        let store = Arc::new(Store::open(dir.path()).await.unwrap());
        let feed = store
            .create_feed(&FeedName::parse("internal").unwrap(), "test")
            .await
            .unwrap();
        let mut tokens = Vec::new();
        for (name, admin) in [("admin", true), ("reader", false)] {
            let p = store
                .create_principal(
                    &PrincipalName::parse(name).unwrap(),
                    PrincipalKind::User,
                    admin,
                    "test",
                )
                .await
                .unwrap();
            if !admin {
                store
                    .set_grant(&p, &feed, Role::Reader, &[], "test")
                    .await
                    .unwrap();
            }
            tokens.push(
                store
                    .create_token(&p, None, 3600, "test")
                    .await
                    .unwrap()
                    .token,
            );
        }
        let reader = tokens.pop().unwrap();
        (store, tokens.pop().unwrap(), reader)
    });
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let router = app(store, &url, Limits::default());
    std::thread::spawn(move || {
        rt.block_on(async {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
    });
    Server {
        url,
        _dir: dir,
        admin,
        reader,
    }
}

/// Entorno aislado del CLI: configuración propia, sin keychain y sin variables heredadas.
struct Cli {
    config: TempDir,
    cwd: TempDir,
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout no es JSON ({e}): {}", self.stdout))
    }

    fn error_json(&self) -> Value {
        serde_json::from_str(&self.stderr)
            .unwrap_or_else(|e| panic!("stderr no es JSON ({e}): {}", self.stderr))
    }
}

impl Cli {
    fn new() -> Self {
        Self {
            config: TempDir::new().unwrap(),
            cwd: TempDir::new().unwrap(),
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_onepack"));
        cmd.args(args)
            .current_dir(self.cwd.path())
            .env("ONEPACK_CONFIG_DIR", self.config.path())
            .env("ONEPACK_KEYRING", "off")
            .env_remove("ONEPACK_TOKEN")
            .env_remove("ONEPACK_URL")
            .env_remove("ONEPACK_CONTEXT")
            .env_remove("ONEPACK_NO_INPUT")
            .env_remove("ONEPACK_TIMEOUT")
            .env_remove("ONEPACK_CA_CERT")
            .stdin(Stdio::null());
        cmd
    }

    fn run_with(&self, args: &[&str], token: Option<&str>, stdin: Option<&str>) -> Run {
        let mut cmd = self.command(args);
        if let Some(t) = token {
            cmd.env("ONEPACK_TOKEN", t);
        }
        let output: Output = match stdin {
            None => cmd.output().unwrap(),
            Some(input) => {
                let mut child = cmd
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(input.as_bytes())
                    .unwrap();
                child.wait_with_output().unwrap()
            }
        };
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn run(&self, args: &[&str], token: &str) -> Run {
        self.run_with(args, Some(token), None)
    }

    fn ok(&self, args: &[&str], token: &str) -> Run {
        let r = self.run(args, token);
        assert_eq!(
            r.code, 0,
            "{args:?}\nstdout: {}\nstderr: {}",
            r.stdout, r.stderr
        );
        r
    }

    fn files_contain(&self, needle: &str) -> bool {
        fn walk(dir: &Path, needle: &str) -> bool {
            std::fs::read_dir(dir).unwrap().flatten().any(|e| {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, needle)
                } else {
                    std::fs::read(&p)
                        .map(|b| String::from_utf8_lossy(&b).contains(needle))
                        .unwrap_or(false)
                }
            })
        }
        walk(self.config.path(), needle) || walk(self.cwd.path(), needle)
    }
}

fn nupkg(dir: &Path, id: &str, version: &str, extra: &str) -> PathBuf {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(format!("{id}.nuspec"), SimpleFileOptions::default())
        .unwrap();
    zip.write_all(
        format!(
            "<package><metadata><id>{id}</id><version>{version}</version><description>{extra}</description></metadata></package>"
        )
        .as_bytes(),
    )
    .unwrap();
    let path = dir.join(format!("{id}.{version}.{}.nupkg", extra.len()));
    std::fs::write(&path, zip.finish().unwrap().into_inner()).unwrap();
    path
}

fn context(cli: &Cli, s: &Server) {
    let r = cli.run_with(&["context", "add", "local", "--url", &s.url], None, None);
    assert_eq!(r.code, 0, "{}", r.stderr);
}

#[test]
fn full_flow_without_ui() {
    let s = server();
    let cli = Cli::new();
    context(&cli, &s);
    let a = s.admin.as_str();

    cli.ok(&["feed", "create", "payments"], a);
    cli.ok(
        &["principal", "create", "ci-payments", "--kind", "service"],
        a,
    );
    let issued = cli
        .ok(
            &[
                "token",
                "create",
                "--principal",
                "ci-payments",
                "--expires-in-days",
                "7",
                "--json",
            ],
            a,
        )
        .json();
    let ci = issued["token"].as_str().unwrap().to_owned();
    // En texto, solo el token va a stdout.
    let plain = cli.ok(&["token", "create", "--principal", "ci-payments"], a);
    assert!(plain.stdout.trim().starts_with("opk_"));
    assert_eq!(plain.stdout.trim().lines().count(), 1);

    cli.ok(
        &[
            "grant",
            "add",
            "--principal",
            "ci-payments",
            "--feed",
            "payments",
            "--role",
            "publisher",
            "--publish-pattern",
            "Hemia.Payments.*",
        ],
        a,
    );

    let pkgs = TempDir::new().unwrap();
    let first = nupkg(pkgs.path(), "Hemia.Payments.Core", "1.0.0", "a");
    let pushed = cli
        .ok(
            &[
                "package",
                "push",
                "--feed",
                "payments",
                first.to_str().unwrap(),
                "--json",
            ],
            &ci,
        )
        .json();
    assert_eq!(pushed[0]["status"], "pushed");

    // El mismo archivo otra vez: error sin la opción, éxito con ella.
    let dup = cli.run(
        &[
            "package",
            "push",
            "--feed",
            "payments",
            first.to_str().unwrap(),
        ],
        &ci,
    );
    assert_eq!(dup.code, 6, "{}", dup.stderr);
    assert!(dup.stderr.contains("PACKAGE_VERSION_EXISTS"));
    let skipped = cli
        .ok(
            &[
                "package",
                "push",
                "--feed",
                "payments",
                first.to_str().unwrap(),
                "--skip-existing-identical",
                "--json",
            ],
            &ci,
        )
        .json();
    assert_eq!(skipped[0]["status"], "skipped");
    // Misma versión, contenido distinto: conflicto también con la opción.
    let other = nupkg(pkgs.path(), "Hemia.Payments.Core", "1.0.0", "different");
    let conflict = cli.run(
        &[
            "package",
            "push",
            "--feed",
            "payments",
            other.to_str().unwrap(),
            "--skip-existing-identical",
        ],
        &ci,
    );
    assert_eq!(conflict.code, 6);
    assert!(conflict.stderr.contains("contenido distinto"));
    // Fuera de sus patrones: sin permiso.
    let foreign = nupkg(pkgs.path(), "Acme.Other", "1.0.0", "a");
    let denied = cli.run(
        &[
            "package",
            "push",
            "--feed",
            "payments",
            foreign.to_str().unwrap(),
        ],
        &ci,
    );
    assert_eq!(denied.code, 4, "{}", denied.stderr);
    assert!(denied.stderr.contains("AUTH_PREFIX_DENIED"));

    let list = cli
        .ok(&["package", "list", "--feed", "payments", "--json"], &ci)
        .json();
    assert_eq!(list[0]["id"], "Hemia.Payments.Core");
    let state = cli
        .ok(
            &[
                "package",
                "unlist",
                "--feed",
                "payments",
                "Hemia.Payments.Core",
                "1.0.0",
                "--json",
            ],
            &ci,
        )
        .json();
    assert_eq!(state["listed"], false);
    let blocked = cli.run(
        &[
            "package",
            "block",
            "--feed",
            "payments",
            "Hemia.Payments.Core",
            "1.0.0",
            "--reason",
            "incidente",
        ],
        a,
    );
    assert_eq!(
        blocked.code, 2,
        "sin --yes y sin terminal: {}",
        blocked.stderr
    );
    assert!(blocked.stderr.contains("CONFIRMATION_REQUIRED"));
    cli.ok(
        &[
            "package",
            "block",
            "--feed",
            "payments",
            "Hemia.Payments.Core",
            "1.0.0",
            "--reason",
            "incidente",
            "--yes",
        ],
        a,
    );
    let inspect = cli
        .ok(
            &[
                "package",
                "inspect",
                "--feed",
                "payments",
                "Hemia.Payments.Core",
                "1.0.0",
                "--json",
            ],
            a,
        )
        .json();
    assert_eq!(inspect["availability"], "blocked");
    assert_eq!(inspect["blocked_reason"], "incidente");

    let audit = cli
        .ok(&["audit", "list", "--action", "package.", "--json"], a)
        .json();
    let actions: Vec<&str> = audit
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert!(actions.contains(&"package.block") && actions.contains(&"package.unlist"));

    let revoke = cli.ok(
        &["token", "revoke", issued["id"].as_str().unwrap(), "--yes"],
        a,
    );
    assert!(revoke.stdout.contains("revocado"));
    let after = cli.run(&["whoami"], &ci);
    assert_eq!(after.code, 3, "token revocado: {}", after.stderr);

    // Ningún secreto en archivos del CLI.
    assert!(!cli.files_contain(&ci));
    assert!(!cli.files_contain(a));
}

#[test]
fn exit_codes_and_error_format() {
    let s = server();
    let cli = Cli::new();
    context(&cli, &s);

    let no_token = cli.run_with(&["whoami"], None, None);
    assert_eq!(no_token.code, 3);
    assert!(no_token.stderr.contains("AUTH_REQUIRED"));
    assert!(no_token.stderr.contains("acción:"));

    let bad = cli.run(&["whoami"], "opk_0000000000000000_bad");
    assert_eq!(bad.code, 3);

    let forbidden = cli.run(&["principal", "list", "--json"], &s.reader);
    assert_eq!(forbidden.code, 4);
    let err = forbidden.error_json();
    assert_eq!(err["error"]["code"], "AUTH_ADMIN_REQUIRED");
    assert_eq!(err["error"]["request_id"].as_str().map(str::len), Some(16));
    assert!(forbidden.stdout.is_empty(), "los errores van a stderr");

    let missing = cli.run(&["feed", "show", "nope"], &s.admin);
    assert_eq!(missing.code, 5);

    let exists = cli.run(&["feed", "create", "internal"], &s.admin);
    assert_eq!(exists.code, 6);

    let invalid = cli.run(&["feed", "create", "Bad Name"], &s.admin);
    assert_eq!(invalid.code, 2);

    let env_missing = cli.run_with(
        &["whoami", "--token-env", "NO_SUCH_VAR_ONEPACK"],
        None,
        None,
    );
    assert_eq!(env_missing.code, 3);
    assert!(env_missing.stderr.contains("NO_SUCH_VAR_ONEPACK"));

    let unreachable = cli.run(&["whoami", "--url", "http://127.0.0.1:9"], &s.admin);
    assert_eq!(unreachable.code, 7, "{}", unreachable.stderr);
    assert!(unreachable.stderr.contains("CONNECT_FAILED"));

    let no_context = Cli::new().run(&["whoami"], &s.admin);
    assert_eq!(no_context.code, 2);
    assert!(no_context.stderr.contains("CONTEXT_MISSING"));
}

#[test]
fn login_requires_a_keychain_and_never_falls_back_to_plain_text() {
    let s = server();
    let cli = Cli::new();
    context(&cli, &s);
    let r = cli.run_with(&["login", "--token-stdin"], None, Some(&s.admin));
    assert_eq!(r.code, 9, "{}", r.stderr);
    assert!(r.stderr.contains("KEYCHAIN_UNAVAILABLE"));
    assert!(r.stderr.contains("--token-env"));
    assert!(!cli.files_contain(&s.admin));
}

#[test]
fn contexts() {
    let s = server();
    let cli = Cli::new();
    context(&cli, &s);
    cli.run_with(
        &[
            "context",
            "add",
            "other",
            "--url",
            "https://p.example.test/",
        ],
        None,
        None,
    );
    let list = cli
        .run_with(&["context", "list", "--json"], None, None)
        .json();
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["name"], "local");
    assert_eq!(list[0]["current"], true);
    assert_eq!(list[1]["url"], "https://p.example.test");
    cli.run_with(&["context", "use", "other"], None, None);
    let me = cli.run(&["whoami", "--context", "local", "--json"], &s.admin);
    assert_eq!(me.json()["principal"], "admin");
    let removed = cli.run_with(&["context", "remove", "other", "--yes"], None, None);
    assert_eq!(removed.code, 0, "{}", removed.stderr);
    let list = cli
        .run_with(&["context", "list", "--json"], None, None)
        .json();
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn exec_injects_credentials_only_into_the_child() {
    let s = server();
    let cli = Cli::new();
    context(&cli, &s);
    let r = cli.ok(
        &[
            "exec", "--feed", "internal", "--feed", "customer-a", "--", "sh", "-c",
            "printf '%s\\n%s' \"$NuGetPackageSourceCredentials_onepack_internal\" \"$NuGetPackageSourceCredentials_onepack_customer_a\"",
        ],
        &s.reader,
    );
    let lines: Vec<&str> = r.stdout.lines().collect();
    let expected = format!("Username=onepack;Password={}", s.reader);
    assert_eq!(lines, [expected.as_str(), expected.as_str()]);
    // El proceso padre no recibe nada y no se escribe nada a disco.
    assert!(std::env::var("NuGetPackageSourceCredentials_onepack_internal").is_err());
    assert!(!cli.files_contain(&s.reader));

    // El código de salida del hijo se propaga.
    let failing = cli.run(
        &["exec", "--feed", "internal", "--", "sh", "-c", "exit 42"],
        &s.reader,
    );
    assert_eq!(failing.code, 42);
    let custom = cli.ok(
        &[
            "exec",
            "--feed",
            "internal",
            "--source-name",
            "corp",
            "--",
            "sh",
            "-c",
            "printf %s \"$NuGetPackageSourceCredentials_corp\"",
        ],
        &s.reader,
    );
    assert_eq!(custom.stdout, expected);
}

#[test]
fn nuget_init_preserves_existing_config_and_shows_a_diff() {
    let s = server();
    let cli = Cli::new();
    context(&cli, &s);
    let existing = r#"<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <!-- no tocar -->
  <packageSources>
    <add key="nuget.org" value="https://api.nuget.org/v3/index.json" protocolVersion="3" />
  </packageSources>
</configuration>
"#;
    let config = cli.cwd.path().join("NuGet.Config");
    std::fs::write(&config, existing).unwrap();

    let dry = cli.run_with(
        &[
            "nuget",
            "init",
            "--feed",
            "internal",
            "--pattern",
            "Hemia.*",
            "--dry-run",
        ],
        None,
        None,
    );
    assert_eq!(dry.code, 0, "{}", dry.stderr);
    assert!(
        dry.stdout.contains("+    <add key=\"onepack_internal\""),
        "{}",
        dry.stdout
    );
    assert!(
        dry.stdout
            .contains("+      <package pattern=\"Hemia.*\" />")
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        existing,
        "--dry-run no escribe"
    );

    let no_yes = cli.run_with(
        &[
            "nuget",
            "init",
            "--feed",
            "internal",
            "--pattern",
            "Hemia.*",
        ],
        None,
        None,
    );
    assert_eq!(no_yes.code, 2, "sin terminal hace falta --yes");

    let applied = cli
        .run_with(
            &[
                "nuget",
                "init",
                "--feed",
                "internal",
                "--pattern",
                "Hemia.*",
                "--yes",
                "--json",
            ],
            None,
            None,
        )
        .json();
    assert_eq!(applied["written"], true);
    let written = std::fs::read_to_string(&config).unwrap();
    assert!(written.contains("<!-- no tocar -->"));
    assert!(written.contains("https://api.nuget.org/v3/index.json"));
    assert!(written.contains(&format!("{}/nuget/internal/v3/index.json", s.url)));
    assert!(written.contains("<packageSource key=\"nuget.org\">"));
    assert!(!written.contains("Password"), "sin secretos");

    let again = cli
        .run_with(
            &[
                "nuget",
                "init",
                "--feed",
                "internal",
                "--pattern",
                "Hemia.*",
                "--json",
            ],
            None,
            None,
        )
        .json();
    assert_eq!(again["changed"], false, "idempotente");
}

#[test]
fn doctor_reports_missing_publish_scope_without_leaking_the_token() {
    let s = server();
    let cli = Cli::new();
    context(&cli, &s);
    cli.run_with(
        &[
            "nuget",
            "init",
            "--feed",
            "internal",
            "--pattern",
            "Hemia.*",
            "--yes",
        ],
        None,
        None,
    );
    let r = cli.run(
        &[
            "doctor",
            "--feed",
            "internal",
            "--require",
            "publish",
            "--json",
        ],
        &s.reader,
    );
    assert_eq!(r.code, 10, "{}", r.stdout);
    let report = r.json();
    assert_eq!(report["ok"], false);
    let check = |name: &str| {
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("falta {name}: {report}"))
            .clone()
    };
    let permissions = check("permissions");
    assert_eq!(permissions["status"], "fail");
    assert_eq!(permissions["code"], "AUTH_SCOPE_MISSING");
    assert!(
        permissions["action"]
            .as_str()
            .unwrap()
            .contains("--role publisher")
    );
    for ok in [
        "dns",
        "tcp",
        "tls",
        "credentials",
        "auth",
        "server",
        "service-index",
        "nuget-config",
        "source-mapping",
    ] {
        assert_eq!(check(ok)["status"], "ok", "{ok}: {}", check(ok));
    }
    let secret = s.reader.rsplit('_').next().unwrap();
    assert!(!r.stdout.contains(secret) && !r.stderr.contains(secret));

    // Con lectura basta: sin fallos.
    let fine = cli.run(&["doctor", "--feed", "internal"], &s.reader);
    assert_eq!(fine.code, 0, "{}", fine.stdout);
    assert!(fine.stdout.contains("Sin fallos."));

    // Un servidor inaccesible se diagnostica en la red, antes de la credencial.
    let down = cli.run(
        &["doctor", "--url", "http://127.0.0.1:9", "--json"],
        &s.reader,
    );
    assert_eq!(down.code, 10);
    assert!(down.stdout.contains("CONNECT_FAILED"));
}

/// Servidor mínimo que imita una versión anterior de onepackd.
fn old_server(capabilities: Option<Value>) -> String {
    use axum::Json;
    use axum::routing::get;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let mut router = axum::Router::new().route(
        "/api/v1/feeds",
        get(|| async { Json(json!({ "items": [], "next_cursor": null })) }),
    );
    if let Some(caps) = capabilities {
        router = router.route(
            "/api/v1/capabilities",
            get(move || async move { Json(caps) }),
        );
    }
    std::thread::spawn(move || {
        rt.block_on(async { axum::serve(listener, router).await.unwrap() });
    });
    url
}

#[test]
fn newer_cli_against_older_server_fails_with_missing_capability() {
    let cli = Cli::new();
    // N-1 que anuncia capacidades, pero no la de auditoría.
    let url = old_server(Some(json!({
        "server_version": "0.0.0-n1",
        "api_version": 1,
        "capabilities": ["feeds"]
    })));
    let feeds = cli.run(&["feed", "list", "--url", &url, "--json"], "opk_x_y");
    assert_eq!(feeds.code, 0, "{}", feeds.stderr);
    assert_eq!(feeds.json(), json!([]));
    let audit = cli.run(&["audit", "list", "--url", &url], "opk_x_y");
    assert_eq!(audit.code, 8, "{}", audit.stderr);
    assert!(audit.stderr.contains("CAPABILITY_MISSING"));
    assert!(audit.stderr.contains("0.0.0-n1"));

    // Servidor de antes de la Fase 6: sin endpoint de capacidades.
    let url = old_server(None);
    let r = cli.run(&["feed", "list", "--url", &url], "opk_x_y");
    assert_eq!(r.code, 8, "{}", r.stderr);
    assert!(r.stderr.contains("CAPABILITY_MISSING"));
}

/// Forma de un valor JSON: claves y tipos, sin datos.
fn shape(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), shape(v))).collect())
        }
        Value::Array(items) => Value::Array(items.first().map(shape).into_iter().collect()),
        Value::String(_) => json!("string"),
        Value::Number(_) => json!("number"),
        Value::Bool(_) => json!("bool"),
        Value::Null => json!("null"),
    }
}

#[test]
fn json_output_matches_schema_snapshot() {
    let s = server();
    let cli = Cli::new();
    context(&cli, &s);
    let a = s.admin.as_str();
    let pkgs = TempDir::new().unwrap();
    let pkg = nupkg(pkgs.path(), "Hemia.Core", "1.0.0", "a");
    let pkg = pkg.to_str().unwrap();
    cli.ok(
        &[
            "nuget",
            "init",
            "--feed",
            "internal",
            "--pattern",
            "Hemia.*",
            "--yes",
        ],
        a,
    );

    let commands: Vec<(&str, Vec<&str>)> = vec![
        ("context list", vec!["context", "list"]),
        ("whoami", vec!["whoami"]),
        ("feed create", vec!["feed", "create", "snap"]),
        ("feed list", vec!["feed", "list"]),
        ("feed show", vec!["feed", "show", "internal"]),
        (
            "feed configure",
            vec!["feed", "configure", "snap", "--max-versions", "5"],
        ),
        (
            "principal create",
            vec!["principal", "create", "svc", "--kind", "service"],
        ),
        ("principal list", vec!["principal", "list"]),
        (
            "token create",
            vec!["token", "create", "--principal", "svc", "--name", "n"],
        ),
        ("token list", vec!["token", "list", "--principal", "svc"]),
        (
            "grant add",
            vec![
                "grant",
                "add",
                "--principal",
                "svc",
                "--feed",
                "snap",
                "--role",
                "reader",
            ],
        ),
        ("grant list", vec!["grant", "list", "--feed", "snap"]),
        (
            "package push",
            vec!["package", "push", "--feed", "internal", pkg],
        ),
        (
            "package list",
            vec!["package", "list", "--feed", "internal"],
        ),
        (
            "package inspect",
            vec!["package", "inspect", "--feed", "internal", "Hemia.Core"],
        ),
        (
            "package inspect version",
            vec![
                "package",
                "inspect",
                "--feed",
                "internal",
                "Hemia.Core",
                "1.0.0",
            ],
        ),
        (
            "package unlist",
            vec![
                "package",
                "unlist",
                "--feed",
                "internal",
                "Hemia.Core",
                "1.0.0",
            ],
        ),
        ("audit list", vec!["audit", "list", "--limit", "1"]),
        ("doctor", vec!["doctor", "--feed", "internal"]),
        (
            "nuget init",
            vec![
                "nuget",
                "init",
                "--feed",
                "internal",
                "--pattern",
                "Hemia.*",
                "--dry-run",
            ],
        ),
    ];
    let mut shapes = serde_json::Map::new();
    for (name, mut args) in commands {
        args.push("--json");
        let r = cli.ok(&args, a);
        shapes.insert(name.to_owned(), shape(&r.json()));
    }
    let error = cli.run(&["feed", "show", "nope", "--json"], a);
    shapes.insert("error".to_owned(), shape(&error.error_json()));
    let actual = Value::Object(shapes);

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/json-schema.json");
    if std::env::var_os("ONEPACK_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
    }
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .expect("falta el snapshot; genera con ONEPACK_UPDATE_SNAPSHOTS=1"),
    )
    .unwrap();
    assert_eq!(
        actual, expected,
        "la salida --json cambió de forma: es un cambio de contrato (ADR-015). \
         Si es intencionado, regenera con ONEPACK_UPDATE_SNAPSHOTS=1 y documenta el cambio."
    );
}
