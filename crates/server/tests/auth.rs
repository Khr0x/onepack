//! Pruebas bloqueantes de la Fase 4: autenticación obligatoria, aislamiento entre feeds,
//! roles, prefijos y revocación (ADR-010, ADR-011, ADR-012).

use std::io::{Cursor, Write};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use http_body_util::BodyExt;
use onepack_core::{FeedName, PrincipalKind, PrincipalName, PublishPattern, Role};
use onepack_server::{Limits, app};
use onepack_storage::{Store, migrate};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;
use zip::write::SimpleFileOptions;

const BOUNDARY: &str = "onepack-auth";

#[derive(Clone, Copy)]
enum Auth<'a> {
    None,
    ApiKey(&'a str),
    Basic(&'a str),
    Bearer(&'a str),
}

struct Env {
    router: Router,
    store: Arc<Store>,
    dir: TempDir,
    admin: String,
    reader: String,
    ci: String,
}

async fn env() -> Env {
    let dir = TempDir::new().unwrap();
    migrate(dir.path()).await.unwrap();
    let store = Arc::new(Store::open(dir.path()).await.unwrap());
    let internal = store
        .create_feed(&FeedName::parse("internal").unwrap(), "test")
        .await
        .unwrap();
    store
        .create_feed(&FeedName::parse("customer-a").unwrap(), "test")
        .await
        .unwrap();

    let token_for = async |name: &str, admin: bool, role: Option<(Role, &[&str])>| {
        let p = store
            .create_principal(
                &PrincipalName::parse(name).unwrap(),
                PrincipalKind::Service,
                admin,
                "test",
            )
            .await
            .unwrap();
        if let Some((role, patterns)) = role {
            let patterns: Vec<_> = patterns
                .iter()
                .map(|p| PublishPattern::parse(p).unwrap())
                .collect();
            store
                .set_grant(&p, &internal, role, &patterns, "test")
                .await
                .unwrap();
        }
        store
            .create_token(&p, None, 3600, "test")
            .await
            .unwrap()
            .token
    };
    let admin = token_for("admin", true, None).await;
    let reader = token_for("reader", false, Some((Role::Reader, &[]))).await;
    let ci = token_for(
        "ci-payments",
        false,
        Some((Role::Publisher, &["Hemia.Payments.*"])),
    )
    .await;

    let router = app(
        store.clone(),
        "https://packages.example.test",
        Limits {
            max_package_bytes: 1024 * 1024,
            ..Limits::default()
        },
    );
    let env = Env {
        router,
        store,
        dir,
        admin,
        reader,
        ci,
    };
    // Un paquete en cada feed, publicado por el administrador.
    for feed in ["internal", "customer-a"] {
        let admin = env.admin.clone();
        let (status, _) = env
            .push(feed, &nupkg("Hemia.Secret", "1.0.0"), Auth::ApiKey(&admin))
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    env
}

fn nupkg(id: &str, version: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(format!("{id}.nuspec"), SimpleFileOptions::default())
        .unwrap();
    zip.write_all(
        format!(
            "<package><metadata><id>{id}</id><version>{version}</version></metadata></package>"
        )
        .as_bytes(),
    )
    .unwrap();
    zip.finish().unwrap().into_inner()
}

impl Env {
    async fn send(
        &self,
        method: Method,
        uri: &str,
        auth: Auth<'_>,
        body: Option<&[u8]>,
    ) -> (StatusCode, Vec<u8>, Option<String>) {
        let mut req = Request::builder().method(method).uri(uri);
        req = match auth {
            Auth::None => req,
            Auth::ApiKey(t) => req.header("X-NuGet-ApiKey", t),
            Auth::Basic(t) => req.header(
                header::AUTHORIZATION,
                format!("Basic {}", STANDARD.encode(format!("user:{t}"))),
            ),
            Auth::Bearer(t) => req.header(header::AUTHORIZATION, format!("Bearer {t}")),
        };
        let req = match body {
            Some(package) => {
                let mut b = format!(
                    "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"p.nupkg\"\r\n\r\n"
                )
                .into_bytes();
                b.extend_from_slice(package);
                b.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
                req.header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={BOUNDARY}"))
                    .body(Body::from(b))
            }
            None => req.body(Body::empty()),
        }
        .unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let challenge = res
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .map(|v| v.to_str().unwrap().to_owned());
        (
            status,
            res.into_body().collect().await.unwrap().to_bytes().to_vec(),
            challenge,
        )
    }

    async fn get(&self, uri: &str, auth: Auth<'_>) -> (StatusCode, Vec<u8>) {
        let (s, b, _) = self.send(Method::GET, uri, auth, None).await;
        (s, b)
    }

    async fn push(&self, feed: &str, package: &[u8], auth: Auth<'_>) -> (StatusCode, String) {
        let (s, b, _) = self
            .send(
                Method::PUT,
                &format!("/nuget/{feed}/v2/package"),
                auth,
                Some(package),
            )
            .await;
        (s, String::from_utf8_lossy(&b).into_owned())
    }

    async fn audit(&self) -> Vec<(String, Option<String>, String)> {
        let pool = sqlx::SqlitePool::connect(&format!(
            "sqlite://{}",
            self.dir.path().join("metadata.sqlite").display()
        ))
        .await
        .unwrap();
        let rows: Vec<(String, Option<String>, String)> =
            sqlx::query_as("SELECT action, actor, outcome FROM audit_event ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        pool.close().await;
        rows
    }
}

/// Todas las rutas registradas, más rutas inexistentes. El middleware es global, así que una
/// ruta nueva queda protegida aunque no se añada aquí.
const ROUTES: &[(&str, &str)] = &[
    ("GET", "/nuget/internal/v3/index.json"),
    ("HEAD", "/nuget/internal/v3/index.json"),
    ("GET", "/nuget/internal/v3/flat/hemia.secret/index.json"),
    (
        "GET",
        "/nuget/internal/v3/flat/hemia.secret/1.0.0/hemia.secret.1.0.0.nupkg",
    ),
    (
        "HEAD",
        "/nuget/internal/v3/flat/hemia.secret/1.0.0/hemia.secret.1.0.0.nupkg",
    ),
    (
        "GET",
        "/nuget/internal/v3/flat/hemia.secret/1.0.0/hemia.secret.nuspec",
    ),
    ("PUT", "/nuget/internal/v2/package"),
    ("PUT", "/nuget/internal/v2/package/"),
    ("DELETE", "/nuget/internal/v2/package/hemia.secret/1.0.0"),
    ("POST", "/nuget/internal/v2/package/hemia.secret/1.0.0"),
    (
        "GET",
        "/nuget/internal/v3/registration/hemia.secret/index.json",
    ),
    (
        "GET",
        "/nuget/internal/v3/registration/hemia.secret/1.0.0.json",
    ),
    (
        "GET",
        "/nuget/internal/v3/registration/hemia.secret/page/1.0.0/1.0.0.json",
    ),
    ("GET", "/nuget/internal/v3/query?q=hemia"),
    ("GET", "/nuget/internal/v3/autocomplete?q=hemia"),
    ("GET", "/nuget/internal/v3/autocomplete?id=hemia.secret"),
    ("GET", "/api/v1/whoami"),
    ("GET", "/api/v1/feeds/internal/packages/hemia.secret/1.0.0"),
    (
        "POST",
        "/api/v1/feeds/internal/packages/hemia.secret/1.0.0/block",
    ),
    (
        "POST",
        "/api/v1/feeds/internal/packages/hemia.secret/1.0.0/unblock",
    ),
    ("GET", "/api/v1/does-not-exist"),
    ("GET", "/nuget/missing/v3/index.json"),
    ("GET", "/"),
    ("POST", "/anything"),
];

#[tokio::test]
async fn every_route_requires_authentication() {
    let env = env().await;
    for (method, path) in ROUTES {
        let (status, body, challenge) = env
            .send(method.parse().unwrap(), path, Auth::None, None)
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        let expected = if path.starts_with("/api/") {
            "Bearer"
        } else {
            "Basic"
        };
        assert!(
            challenge.is_some_and(|c| c.starts_with(expected)),
            "{method} {path}: falta WWW-Authenticate {expected}"
        );
        assert!(
            !String::from_utf8_lossy(&body).contains("Hemia.Secret"),
            "{method} {path}"
        );
    }
}

#[tokio::test]
async fn invalid_tokens_are_rejected_everywhere() {
    let env = env().await;
    let unknown = format!("opk_{}_{}", "1".repeat(16), "2".repeat(64));
    let wrong_secret = format!("{}_{}", &env.reader[..20], "f".repeat(64));
    for token in ["garbage", unknown.as_str(), wrong_secret.as_str()] {
        assert_eq!(
            env.get("/nuget/internal/v3/index.json", Auth::Basic(token))
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            env.get("/api/v1/whoami", Auth::Bearer(token)).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    let failures = env
        .audit()
        .await
        .iter()
        .filter(|(a, _, _)| a == "auth.failure")
        .count();
    assert_eq!(failures, 6, "cada fallo de autenticación queda auditado");
}

#[tokio::test]
async fn revoked_and_expired_tokens_stop_working_immediately() {
    let env = env().await;
    let index = "/nuget/internal/v3/index.json";
    assert_eq!(
        env.get(index, Auth::Basic(&env.reader)).await.0,
        StatusCode::OK
    );

    let id = &env.reader[4..20];
    assert!(env.store.revoke_token(id, "test").await.unwrap());
    assert_eq!(
        env.get(index, Auth::Basic(&env.reader)).await.0,
        StatusCode::UNAUTHORIZED
    );

    let p = env.store.principal("reader").await.unwrap().unwrap();
    let expired = env
        .store
        .create_token(&p, None, -1, "test")
        .await
        .unwrap()
        .token;
    assert_eq!(
        env.get(index, Auth::Basic(&expired)).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn each_surface_only_accepts_its_mechanisms() {
    let env = env().await;
    let index = "/nuget/internal/v3/index.json";
    assert_eq!(
        env.get(index, Auth::Basic(&env.reader)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        env.get(index, Auth::ApiKey(&env.reader)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        env.get(index, Auth::Bearer(&env.reader)).await.0,
        StatusCode::UNAUTHORIZED
    );

    assert_eq!(
        env.get("/api/v1/whoami", Auth::Bearer(&env.reader)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        env.get("/api/v1/whoami", Auth::Basic(&env.reader)).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        env.get("/api/v1/whoami", Auth::ApiKey(&env.reader)).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn no_cross_feed_leaks() {
    let env = env().await;
    let missing = env
        .get(
            "/nuget/does-not-exist/v3/index.json",
            Auth::Basic(&env.reader),
        )
        .await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);

    for (method, path) in ROUTES
        .iter()
        .filter(|(_, p)| p.starts_with("/nuget/internal/"))
    {
        let path = path.replace("/internal/", "/customer-a/");
        let body = if *method == "PUT" {
            Some(nupkg("Hemia.Payments.X", "1.0.0"))
        } else {
            None
        };
        let (status, response, _) = env
            .send(
                method.parse().unwrap(),
                &path,
                Auth::Basic(&env.reader),
                body.as_deref(),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
        if *method != "HEAD" {
            assert_eq!(
                response, missing.1,
                "{method} {path}: debe ser indistinguible de un feed inexistente"
            );
        }
    }
}

#[tokio::test]
async fn reader_can_read_but_not_publish() {
    let env = env().await;
    let (status, body) = env
        .get(
            "/nuget/internal/v3/flat/hemia.secret/1.0.0/hemia.secret.1.0.0.nupkg",
            Auth::Basic(&env.reader),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, nupkg("Hemia.Secret", "1.0.0"));

    let (status, body) = env
        .push(
            "internal",
            &nupkg("Hemia.New", "1.0.0"),
            Auth::ApiKey(&env.reader),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.starts_with("AUTH_SCOPE_MISSING"), "{body}");
    assert!(body.contains("packages:publish"), "{body}");
}

#[tokio::test]
async fn publishers_are_limited_to_their_patterns() {
    let env = env().await;
    let (status, _) = env
        .push(
            "internal",
            &nupkg("Hemia.Payments.Core", "1.0.0"),
            Auth::ApiKey(&env.ci),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = env
        .push(
            "internal",
            &nupkg("Hemia.Logging", "1.0.0"),
            Auth::ApiKey(&env.ci),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.starts_with("AUTH_PREFIX_DENIED"), "{body}");
    assert_eq!(
        env.get(
            "/nuget/internal/v3/flat/hemia.logging/index.json",
            Auth::Basic(&env.admin)
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "la publicación denegada no deja rastro"
    );
    assert_eq!(
        std::fs::read_dir(env.dir.path().join("staging"))
            .unwrap()
            .count(),
        0
    );

    // Fuera de su feed, el publisher no ve nada.
    let (status, _) = env
        .push(
            "customer-a",
            &nupkg("Hemia.Payments.Core", "1.0.0"),
            Auth::ApiKey(&env.ci),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn whoami_describes_the_credential() {
    let env = env().await;
    let (status, body) = env.get("/api/v1/whoami", Auth::Bearer(&env.ci)).await;
    assert_eq!(status, StatusCode::OK);
    let me: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(me["principal"], "ci-payments");
    assert_eq!(me["kind"], "service");
    assert_eq!(me["administrator"], false);
    assert_eq!(me["token_id"], &env.ci[4..20]);
    assert_eq!(me["grants"][0]["feed"], "internal");
    assert_eq!(me["grants"][0]["role"], "publisher");
    assert_eq!(me["grants"][0]["publish_patterns"][0], "hemia.payments.*");
}

#[tokio::test]
async fn publications_and_denials_are_audited_with_the_actor() {
    let env = env().await;
    env.push(
        "internal",
        &nupkg("Hemia.Payments.Core", "1.0.0"),
        Auth::ApiKey(&env.ci),
    )
    .await;
    env.push(
        "internal",
        &nupkg("Hemia.Logging", "1.0.0"),
        Auth::ApiKey(&env.ci),
    )
    .await;
    env.push(
        "internal",
        &nupkg("Hemia.Other", "1.0.0"),
        Auth::ApiKey(&env.reader),
    )
    .await;

    let audit = env.audit().await;
    let ci_id = &env.ci[4..20];
    let reader_id = &env.reader[4..20];
    let has = |action: &str, actor: &str, outcome: &str| {
        audit
            .iter()
            .any(|(a, who, o)| a == action && o == outcome && who.as_deref() == Some(actor))
    };
    assert!(has(
        "package.publish",
        &format!("ci-payments (token {ci_id})"),
        "success"
    ));
    assert!(has(
        "package.publish",
        &format!("ci-payments (token {ci_id})"),
        "denied"
    ));
    assert!(has(
        "feed.packages:publish",
        &format!("reader (token {reader_id})"),
        "denied"
    ));
}

#[tokio::test]
async fn unlist_requires_publish_rights_on_the_id() {
    let env = env().await;
    let (status, body, _) = env
        .send(
            Method::DELETE,
            "/nuget/internal/v2/package/hemia.secret/1.0.0",
            Auth::ApiKey(&env.reader),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(String::from_utf8_lossy(&body).starts_with("AUTH_SCOPE_MISSING"));

    let (status, body, _) = env
        .send(
            Method::DELETE,
            "/nuget/internal/v2/package/hemia.secret/1.0.0",
            Auth::ApiKey(&env.ci),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "fuera de su patrón");
    assert!(String::from_utf8_lossy(&body).starts_with("AUTH_PREFIX_DENIED"));

    let (status, _) = env
        .push(
            "internal",
            &nupkg("Hemia.Payments.Core", "1.0.0"),
            Auth::ApiKey(&env.ci),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = env
        .send(
            Method::DELETE,
            "/nuget/internal/v2/package/hemia.payments.core/1.0.0",
            Auth::ApiKey(&env.ci),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let audit = env.audit().await;
    assert!(
        audit
            .iter()
            .any(|(a, _, o)| a == "package.unlist" && o == "success")
    );
    assert!(
        audit
            .iter()
            .any(|(a, _, o)| a == "package.unlist" && o == "denied")
    );
}

#[tokio::test]
async fn search_never_returns_packages_from_other_feeds() {
    let env = env().await;
    let (status, _) = env
        .push(
            "customer-a",
            &nupkg("Customer.Only", "1.0.0"),
            Auth::ApiKey(&env.admin),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    for uri in [
        "/nuget/internal/v3/query?q=customer",
        "/nuget/internal/v3/query?q=packageid:customer.only",
        "/nuget/internal/v3/autocomplete?q=customer",
        "/nuget/internal/v3/autocomplete?id=customer.only",
    ] {
        let (status, body) = env.get(uri, Auth::Basic(&env.reader)).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(
            !String::from_utf8_lossy(&body).contains("Customer.Only"),
            "{uri}"
        );
    }
    assert_eq!(
        env.get(
            "/nuget/internal/v3/registration/customer.only/index.json",
            Auth::Basic(&env.reader)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
