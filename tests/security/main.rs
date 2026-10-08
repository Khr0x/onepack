//! Suite de seguridad de la Fase 5 (ADR-013, ADR-014): paquetes maliciosos sintéticos,
//! ráfagas de peticiones, versiones bloqueadas y cuotas.
//!
//! Se ejecuta con `cargo test -p onepack-server --test security`. Los artefactos se generan
//! en cada prueba; ninguno se guarda en el repositorio.

use std::io::{Cursor, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use onepack_core::{FeedName, FeedQuota, PrincipalKind, PrincipalName, Role};
use onepack_nuget::InspectionLimits;
use onepack_server::{Limits, RateLimit, app};
use onepack_storage::{Store, migrate};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tower::ServiceExt;
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

const BOUNDARY: &str = "onepack-security";
const FEED: &str = "internal";

struct Env {
    router: Router,
    store: Arc<Store>,
    dir: TempDir,
    admin: String,
    reader: String,
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
    reason_phrase: Option<String>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

async fn env(limits: Limits) -> Env {
    let dir = TempDir::new().unwrap();
    migrate(dir.path()).await.unwrap();
    let store = Arc::new(Store::open(dir.path()).await.unwrap());
    let feed = store
        .create_feed(&FeedName::parse(FEED).unwrap(), "test")
        .await
        .unwrap();
    let mut tokens = Vec::new();
    for (name, admin) in [("admin", true), ("reader", false)] {
        let p = store
            .create_principal(
                &PrincipalName::parse(name).unwrap(),
                PrincipalKind::Service,
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
    let router = app(store.clone(), "https://packages.example.test", limits);
    Env {
        router,
        store,
        dir,
        reader: tokens.pop().unwrap(),
        admin: tokens.pop().unwrap(),
    }
}

fn limits() -> Limits {
    Limits {
        max_package_bytes: 64 << 20,
        ..Limits::default()
    }
}

fn nuspec(id: &str, version: &str) -> String {
    format!("<package><metadata><id>{id}</id><version>{version}</version></metadata></package>")
}

fn nupkg(id: &str, version: &str) -> Vec<u8> {
    zip_with(&[(&format!("{id}.nuspec"), nuspec(id, version).as_bytes())])
}

fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, content) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn multipart(package: &[u8]) -> Vec<u8> {
    let mut b = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"p.nupkg\"\r\n\r\n"
    )
    .into_bytes();
    b.extend_from_slice(package);
    b.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    b
}

impl Env {
    async fn request(&self, req: Request<Body>) -> Reply {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let reason_phrase = res
            .extensions()
            .get::<hyper::ext::ReasonPhrase>()
            .map(|r| String::from_utf8_lossy(r.as_bytes()).into_owned());
        let status = res.status();
        let headers = res.headers().clone();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
            reason_phrase,
        }
    }

    async fn get(&self, uri: &str, token: &str) -> Reply {
        self.request(
            Request::get(uri)
                .header("X-NuGet-ApiKey", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    async fn push(&self, package: &[u8]) -> Reply {
        self.request(push_request(&self.admin, Body::from(multipart(package))))
            .await
    }

    async fn api(&self, method: Method, uri: &str, token: &str, body: Option<Value>) -> Reply {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json");
        let body = body.map_or(Body::empty(), |b| Body::from(b.to_string()));
        self.request(req.body(body).unwrap()).await
    }

    async fn audit(&self) -> Vec<(String, String, Option<String>)> {
        let pool = sqlx::SqlitePool::connect(&format!(
            "sqlite://{}",
            self.dir.path().join("metadata.sqlite").display()
        ))
        .await
        .unwrap();
        let rows = sqlx::query_as("SELECT action, outcome, detail FROM audit_event ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        pool.close().await;
        rows
    }

    async fn version_count(&self) -> usize {
        let feed = self.store.feed(FEED).await.unwrap().unwrap();
        self.store.feed_usage(&feed).await.unwrap().versions as usize
    }

    fn staging_files(&self) -> usize {
        count_files(&self.dir.path().join("staging"))
    }

    fn blob_files(&self) -> usize {
        count_files(&self.dir.path().join("blobs"))
    }
}

fn push_request(token: &str, body: Body) -> Request<Body> {
    Request::put(format!("/nuget/{FEED}/v2/package"))
        .header("X-NuGet-ApiKey", token)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(body)
        .unwrap()
}

fn count_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let path = e.path();
            if path.is_dir() { count_files(&path) } else { 1 }
        })
        .sum()
}

/// Comprueba un rechazo controlado: código esperado, nada persistido ni en staging y la
/// denegación auditada.
async fn assert_rejected(env: &Env, reply: &Reply, status: StatusCode, code: &str) {
    assert_eq!(reply.status, status, "{}", reply.body);
    assert!(
        reply.body.starts_with(&format!("{code}: ")),
        "{}",
        reply.body
    );
    assert_eq!(env.version_count().await, 0);
    assert_eq!(env.staging_files(), 0, "staging debe quedar vacío");
    assert_eq!(env.blob_files(), 0, "no debe persistirse ningún blob");
    let audit = env.audit().await;
    let last = audit.last().unwrap();
    assert_eq!(
        (last.0.as_str(), last.1.as_str()),
        ("package.publish", "denied")
    );
    assert!(
        last.2.as_deref().unwrap_or_default().contains(code),
        "{last:?}"
    );
}

/// Sustituye el tamaño descomprimido declarado de cada entrada (cabecera local y directorio
/// central) por `size`. Así se fabrica una ZIP bomb sin tener que comprimir gigabytes.
fn forge_uncompressed_size(zip: &mut [u8], size: u32) {
    let mut i = 0;
    while i + 4 <= zip.len() {
        let offset = match &zip[i..i + 4] {
            b"PK\x03\x04" => Some(22),
            b"PK\x01\x02" => Some(24),
            _ => None,
        };
        if let Some(offset) = offset {
            zip[i + offset..i + offset + 4].copy_from_slice(&size.to_le_bytes());
        }
        i += 1;
    }
}

// ---------------------------------------------------------------------------------------------
// ZIP bombs
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn zip_bomb_with_forged_sizes_is_rejected_without_decompressing() {
    let env = env(limits()).await;
    let mut bomb = zip_with(&[
        ("A.nuspec", nuspec("A", "1.0.0").as_bytes()),
        ("lib/payload.bin", b"tiny"),
    ]);
    // ~4 GiB declarados por entrada.
    forge_uncompressed_size(&mut bomb, u32::MAX - 1);
    let started = Instant::now();
    let reply = env.push(&bomb).await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_rejected(
        &env,
        &reply,
        StatusCode::PAYLOAD_TOO_LARGE,
        "PACKAGE_LIMIT_EXCEEDED",
    )
    .await;
}

#[tokio::test]
async fn zip_bomb_with_real_compression_is_rejected() {
    let env = env(Limits {
        inspection: InspectionLimits {
            max_entry_bytes: 4 << 20,
            max_total_bytes: 8 << 20,
            ..InspectionLimits::default()
        },
        ..limits()
    })
    .await;
    let zeros = vec![0u8; 4 << 20];
    let bomb = zip_with(&[
        ("A.nuspec", nuspec("A", "1.0.0").as_bytes()),
        ("lib/a.bin", &zeros),
        ("lib/b.bin", &zeros),
        ("lib/c.bin", &zeros),
    ]);
    assert!(
        bomb.len() < 200 << 10,
        "la bomba comprime: {} bytes",
        bomb.len()
    );
    let reply = env.push(&bomb).await;
    assert_rejected(
        &env,
        &reply,
        StatusCode::PAYLOAD_TOO_LARGE,
        "PACKAGE_LIMIT_EXCEEDED",
    )
    .await;
    assert!(reply.body.contains("uncompressed"), "{}", reply.body);
}

#[tokio::test]
async fn oversized_nuspec_is_cut_at_the_limit() {
    let env = env(limits()).await;
    // 16 MiB de espacios dentro del .nuspec: comprime a unos pocos KiB.
    let huge = format!(
        "<package><metadata><id>A</id><version>1.0.0</version><description>{}</description></metadata></package>",
        " ".repeat(16 << 20)
    );
    let honest = zip_with(&[("A.nuspec", huge.as_bytes())]);
    let reply = env.push(&honest).await;
    assert_rejected(
        &env,
        &reply,
        StatusCode::PAYLOAD_TOO_LARGE,
        "PACKAGE_LIMIT_EXCEEDED",
    )
    .await;

    // Con el tamaño declarado falseado a la baja, la lectura se corta igualmente.
    let mut liar = honest.clone();
    forge_uncompressed_size(&mut liar, 100);
    let reply = env.push(&liar).await;
    assert!(
        reply.status.is_client_error(),
        "{} {}",
        reply.status,
        reply.body
    );
    assert_eq!(env.version_count().await, 0);
    assert_eq!(env.staging_files(), 0);
}

#[tokio::test]
async fn too_many_entries_are_rejected() {
    let env = env(Limits {
        inspection: InspectionLimits {
            max_entries: 50,
            ..InspectionLimits::default()
        },
        ..limits()
    })
    .await;
    let names: Vec<String> = (0..60).map(|i| format!("lib/f{i}.txt")).collect();
    let nuspec = nuspec("A", "1.0.0");
    let mut entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    entries.push(("A.nuspec", nuspec.as_bytes()));
    let reply = env.push(&zip_with(&entries)).await;
    assert_rejected(
        &env,
        &reply,
        StatusCode::PAYLOAD_TOO_LARGE,
        "PACKAGE_LIMIT_EXCEEDED",
    )
    .await;
}

// ---------------------------------------------------------------------------------------------
// XML
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn billion_laughs_is_rejected() {
    let env = env(limits()).await;
    let mut entities = String::from(r#"<!ENTITY lol0 "lol">"#);
    for i in 1..10 {
        let prev = format!("&lol{};", i - 1);
        entities.push_str(&format!(r#"<!ENTITY lol{i} "{}">"#, prev.repeat(10)));
    }
    let bomb = format!(
        r#"<?xml version="1.0"?><!DOCTYPE package [{entities}]>
        <package><metadata><id>A</id><version>1.0.0</version><description>&lol9;</description></metadata></package>"#
    );
    let started = Instant::now();
    let reply = env.push(&zip_with(&[("A.nuspec", bomb.as_bytes())])).await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_rejected(&env, &reply, StatusCode::BAD_REQUEST, "PACKAGE_INVALID").await;
    assert!(reply.body.contains("DTD"), "{}", reply.body);
}

#[tokio::test]
async fn external_entities_are_never_resolved() {
    let env = env(limits()).await;
    let xxe = r#"<?xml version="1.0"?><!DOCTYPE package [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
        <package><metadata><id>A</id><version>1.0.0</version><description>&xxe;</description></metadata></package>"#;
    let reply = env.push(&zip_with(&[("A.nuspec", xxe.as_bytes())])).await;
    assert_rejected(&env, &reply, StatusCode::BAD_REQUEST, "PACKAGE_INVALID").await;
    assert!(!reply.body.contains("root:"));

    // Sin DTD, una entidad desconocida tampoco se resuelve.
    let undefined = "<package><metadata><id>A</id><version>1.0.0</version><description>&xxe;</description></metadata></package>";
    let reply = env
        .push(&zip_with(&[("A.nuspec", undefined.as_bytes())]))
        .await;
    assert_rejected(&env, &reply, StatusCode::BAD_REQUEST, "PACKAGE_INVALID").await;
}

#[tokio::test]
async fn deeply_nested_xml_is_rejected() {
    let env = env(limits()).await;
    let deep = format!(
        "<package><metadata><id>A</id><version>1.0.0</version>{}{}</metadata></package>",
        "<x>".repeat(10_000),
        "</x>".repeat(10_000)
    );
    let reply = env.push(&zip_with(&[("A.nuspec", deep.as_bytes())])).await;
    assert_rejected(
        &env,
        &reply,
        StatusCode::PAYLOAD_TOO_LARGE,
        "PACKAGE_LIMIT_EXCEEDED",
    )
    .await;
}

// ---------------------------------------------------------------------------------------------
// Rutas
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn path_traversal_is_rejected_and_nothing_is_written_outside_staging() {
    let env = env(limits()).await;
    let marker = "onepack-security-evil-marker";
    for name in [
        format!("../{marker}"),
        format!("../../{marker}"),
        format!("lib/../../{marker}"),
        format!("lib\\..\\..\\{marker}"),
        format!("/tmp/{marker}"),
        format!("C:/{marker}"),
        format!("lib/%2E%2E/%2E%2E/{marker}"),
        "lib/CON".to_owned(),
        "lib/aux.dll".to_owned(),
        "lib/file.dll:hidden".to_owned(),
    ] {
        let package = zip_with(&[
            ("A.nuspec", nuspec("A", "1.0.0").as_bytes()),
            (&name, b"evil"),
        ]);
        let reply = env.push(&package).await;
        assert_rejected(&env, &reply, StatusCode::BAD_REQUEST, "PACKAGE_UNSAFE_PATH").await;
    }
    // Nada se extrae: ni en el directorio de datos ni junto a él.
    let parent = env.dir.path().parent().unwrap();
    for dir in [env.dir.path(), parent] {
        assert!(!dir.join(marker).exists());
    }
    assert!(!Path::new("/tmp").join(marker).exists());
}

#[tokio::test]
async fn duplicate_entries_are_rejected() {
    let env = env(limits()).await;
    let package = zip_with(&[
        ("A.nuspec", nuspec("A", "1.0.0").as_bytes()),
        ("lib/a.dll", b"one"),
        ("LIB/A.DLL", b"two"),
    ]);
    let reply = env.push(&package).await;
    assert_rejected(&env, &reply, StatusCode::BAD_REQUEST, "PACKAGE_UNSAFE_PATH").await;
}

// ---------------------------------------------------------------------------------------------
// Ráfagas
// ---------------------------------------------------------------------------------------------

/// Cuerpo que se queda abierto hasta que se suelta el emisor: simula una subida lenta.
fn stalled_body() -> (mpsc::Sender<Bytes>, Body) {
    let (tx, rx) = mpsc::channel::<Bytes>(1);
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|chunk| (Ok::<_, std::io::Error>(chunk), rx))
    });
    (tx, Body::from_stream(stream))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_burst_degrades_with_503_without_queueing() {
    let env = env(Limits {
        max_concurrent_uploads: 2,
        ..limits()
    })
    .await;

    // Dos subidas lentas ocupan todas las plazas.
    let mut slow = Vec::new();
    for _ in 0..2 {
        let (tx, body) = stalled_body();
        let head = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"p.nupkg\"\r\n\r\nPK"
        );
        tx.send(Bytes::from(head)).await.unwrap();
        let router = env.router.clone();
        let req = push_request(&env.admin, body);
        slow.push((tx, tokio::spawn(async move { router.oneshot(req).await })));
    }
    // Cada subida lenta abre su archivo de staging en cuanto tiene plaza.
    let deadline = Instant::now() + Duration::from_secs(20);
    while env.staging_files() < 2 {
        assert!(Instant::now() < deadline, "las subidas lentas no empezaron");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    for _ in 0..20 {
        let reply = env.push(&nupkg("Burst", "1.0.0")).await;
        assert_eq!(
            reply.status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{}",
            reply.body
        );
        assert!(reply.body.starts_with("UPLOADS_BUSY: "), "{}", reply.body);
        assert!(reply.headers.contains_key(header::RETRY_AFTER));
        // Los rechazos no leen el cuerpo ni dejan nada en staging.
        assert_eq!(env.staging_files(), 2);
    }
    assert_eq!(env.version_count().await, 0);

    // Al cortar las subidas lentas se liberan las plazas.
    for (tx, task) in slow {
        drop(tx);
        let res = task.await.unwrap().unwrap();
        assert!(res.status().is_client_error(), "{}", res.status());
    }
    let reply = env.push(&nupkg("After.Burst", "1.0.0")).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(env.staging_files(), 0);
}

#[tokio::test]
async fn stalled_upload_times_out() {
    let env = env(Limits {
        upload_timeout: Duration::from_millis(300),
        ..limits()
    })
    .await;
    let (tx, body) = stalled_body();
    tx.send(Bytes::from(format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"p.nupkg\"\r\n\r\nPK"
    )))
    .await
    .unwrap();
    let reply = env.request(push_request(&env.admin, body)).await;
    drop(tx);
    assert_eq!(reply.status, StatusCode::REQUEST_TIMEOUT, "{}", reply.body);
    assert!(reply.body.starts_with("UPLOAD_TIMEOUT: "));
    assert_eq!(env.staging_files(), 0);
}

#[tokio::test]
async fn requests_are_rate_limited_per_principal() {
    let env = env(Limits {
        principal_rate: RateLimit::new(1, 5),
        ..limits()
    })
    .await;
    let index = format!("/nuget/{FEED}/v3/index.json");
    for _ in 0..5 {
        assert_eq!(env.get(&index, &env.reader).await.status, StatusCode::OK);
    }
    let reply = env.get(&index, &env.reader).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(reply.body.starts_with("RATE_LIMITED: "));
    assert!(reply.headers.contains_key(header::RETRY_AFTER));
    // Otro principal tiene su propio cupo.
    assert_eq!(env.get(&index, &env.admin).await.status, StatusCode::OK);
    // La API administrativa responde el mismo límite en JSON.
    let reply = env
        .api(Method::GET, "/api/v1/whoami", &env.reader, None)
        .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(reply.json()["error"]["code"], "RATE_LIMITED");
}

#[tokio::test]
async fn requests_are_rate_limited_per_ip_before_authentication() {
    let mut env = env(Limits {
        ip_rate: RateLimit::new(1, 3),
        ..limits()
    })
    .await;
    let attacker: SocketAddr = "203.0.113.7:40000".parse().unwrap();
    env.router = env.router.layer(MockConnectInfo(attacker));
    let index = format!("/nuget/{FEED}/v3/index.json");
    // Credenciales inválidas: cuentan igual, así que frenan la fuerza bruta.
    for _ in 0..3 {
        assert_eq!(
            env.get(&index, "opk_0000000000000000_bad").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let reply = env.get(&index, &env.admin).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(reply.headers.contains_key(header::RETRY_AFTER));
}

// ---------------------------------------------------------------------------------------------
// Versiones bloqueadas
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn blocked_version_cannot_be_downloaded_but_stays_visible() {
    let env = env(limits()).await;
    assert_eq!(
        env.push(&nupkg("Hemia.Core", "1.0.0")).await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        env.push(&nupkg("Hemia.Core", "1.1.0")).await.status,
        StatusCode::CREATED
    );
    let api = format!("/api/v1/feeds/{FEED}/packages/hemia.core/1.0.0");
    let nupkg_url = format!("/nuget/{FEED}/v3/flat/hemia.core/1.0.0/hemia.core.1.0.0.nupkg");
    let nuspec_url = format!("/nuget/{FEED}/v3/flat/hemia.core/1.0.0/hemia.core.nuspec");

    // Solo Maintainer o administrador bloquea, y siempre con motivo.
    let reason = json!({ "reason": "CVE-2026-0001: ejecución remota" });
    let reply = env
        .api(
            Method::POST,
            &format!("{api}/block"),
            &env.reader,
            Some(reason.clone()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.json()["error"]["code"], "AUTH_SCOPE_MISSING");
    for body in [None, Some(json!({})), Some(json!({ "reason": "  " }))] {
        let reply = env
            .api(Method::POST, &format!("{api}/block"), &env.admin, body)
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    }
    let reply = env
        .api(
            Method::POST,
            &format!("{api}/block"),
            &env.admin,
            Some(reason.clone()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["availability"], "blocked");
    assert_eq!(reply.json()["changed"], true);

    // La descarga falla con un error explícito, también para el administrador.
    for token in [&env.reader, &env.admin] {
        for url in [&nupkg_url, &nuspec_url] {
            let reply = env.get(url, token).await;
            assert_eq!(reply.status, StatusCode::GONE, "{url}");
            assert!(
                reply.body.starts_with("PACKAGE_BLOCKED: "),
                "{}",
                reply.body
            );
            assert!(
                reply
                    .reason_phrase
                    .as_deref()
                    .is_some_and(|r| r.contains("PACKAGE_BLOCKED")),
                "{:?}",
                reply.reason_phrase
            );
        }
    }
    // La otra versión se descarga con normalidad.
    let other = format!("/nuget/{FEED}/v3/flat/hemia.core/1.1.0/hemia.core.1.1.0.nupkg");
    assert_eq!(env.get(&other, &env.reader).await.status, StatusCode::OK);

    // Sigue en los metadatos (para que restore falle con un mensaje claro) y se anuncia
    // como obsoleta; desaparece de la búsqueda.
    let flat = env
        .get(
            &format!("/nuget/{FEED}/v3/flat/hemia.core/index.json"),
            &env.reader,
        )
        .await;
    assert_eq!(flat.json()["versions"], json!(["1.0.0", "1.1.0"]));
    let leaf = env
        .get(
            &format!("/nuget/{FEED}/v3/registration/hemia.core/index.json"),
            &env.reader,
        )
        .await
        .json();
    let items = &leaf["items"][0]["items"];
    let blocked = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["catalogEntry"]["version"] == "1.0.0")
        .unwrap();
    assert_eq!(
        blocked["catalogEntry"]["deprecation"]["reasons"],
        json!(["Other"])
    );
    assert!(
        !blocked.to_string().contains("CVE-2026-0001"),
        "el motivo no se publica"
    );
    let search = env
        .get(&format!("/nuget/{FEED}/v3/query?q=hemia"), &env.reader)
        .await
        .json();
    assert_eq!(search["data"][0]["version"], "1.1.0");
    assert_eq!(search["data"][0]["versions"].as_array().unwrap().len(), 1);

    // El motivo lo ve quien mantiene el feed; un lector ve solo el estado.
    let state = env.api(Method::GET, &api, &env.admin, None).await.json();
    assert_eq!(state["availability"], "blocked");
    assert_eq!(state["blocked_reason"], "CVE-2026-0001: ejecución remota");
    let state = env.api(Method::GET, &api, &env.reader, None).await.json();
    assert_eq!(state["availability"], "blocked");
    assert_eq!(state["blocked_reason"], Value::Null);

    // Desbloquear también exige motivo y restaura la descarga.
    let reply = env
        .api(
            Method::POST,
            &format!("{api}/unblock"),
            &env.admin,
            Some(json!({ "reason": "falso positivo" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        env.get(&nupkg_url, &env.reader).await.status,
        StatusCode::OK
    );

    let audit = env.audit().await;
    let actions: Vec<_> = audit
        .iter()
        .filter(|(a, o, _)| a.starts_with("package.") && a != "package.publish" && o == "success")
        .map(|(a, _, d)| (a.as_str(), d.clone().unwrap_or_default()))
        .collect();
    assert_eq!(actions.len(), 2, "{actions:?}");
    assert_eq!(actions[0].0, "package.block");
    assert!(actions[0].1.contains("CVE-2026-0001"));
    assert_eq!(actions[1].0, "package.unblock");
    assert!(actions[1].1.contains("falso positivo"));
    assert!(
        audit
            .iter()
            .any(|(a, o, _)| a == "feed.packages:maintain" && o == "denied")
    );
}

// ---------------------------------------------------------------------------------------------
// Cuotas
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn version_quota_rejects_with_specific_code() {
    let env = env(limits()).await;
    let feed = env.store.feed(FEED).await.unwrap().unwrap();
    env.store
        .set_feed_quota(
            &feed,
            FeedQuota {
                max_versions: Some(1),
                ..FeedQuota::default()
            },
            "test",
        )
        .await
        .unwrap();
    assert_eq!(
        env.push(&nupkg("A", "1.0.0")).await.status,
        StatusCode::CREATED
    );
    let blobs = env.blob_files();

    let reply = env.push(&nupkg("A", "2.0.0")).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    assert!(
        reply.body.starts_with("FEED_QUOTA_VERSIONS: "),
        "{}",
        reply.body
    );
    assert_eq!(env.version_count().await, 1);
    assert_eq!(env.blob_files(), blobs, "nada persistido");
    assert_eq!(env.staging_files(), 0);
    let audit = env.audit().await;
    let last = audit.last().unwrap();
    assert_eq!(last.1, "denied");
    assert!(last.2.as_deref().unwrap().contains("FEED_QUOTA_VERSIONS"));
}

#[tokio::test]
async fn storage_quota_rejects_with_specific_code() {
    let env = env(limits()).await;
    let first = nupkg("A", "1.0.0");
    let feed = env.store.feed(FEED).await.unwrap().unwrap();
    env.store
        .set_feed_quota(
            &feed,
            FeedQuota {
                max_storage_bytes: Some(first.len() as u64 + 10),
                ..FeedQuota::default()
            },
            "test",
        )
        .await
        .unwrap();
    assert_eq!(env.push(&first).await.status, StatusCode::CREATED);
    let reply = env.push(&nupkg("A", "2.0.0")).await;
    assert_eq!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.body
    );
    assert!(
        reply.body.starts_with("FEED_QUOTA_STORAGE: "),
        "{}",
        reply.body
    );
    assert_eq!(env.version_count().await, 1);
    assert_eq!(env.blob_files(), 1);

    // Sin cuota se vuelve a publicar.
    env.store
        .set_feed_quota(&feed, FeedQuota::default(), "test")
        .await
        .unwrap();
    assert_eq!(
        env.push(&nupkg("A", "2.0.0")).await.status,
        StatusCode::CREATED
    );
}
