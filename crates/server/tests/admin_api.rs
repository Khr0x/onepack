//! API administrativa `/api/v1` (Fase 6, ADR-015): gestión completa, permisos de
//! administrador, errores con acción y `request_id`, y paginación por cursor.

use std::io::{Cursor, Write};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use onepack_api_client as api;
use onepack_core::{FeedName, PrincipalKind, PrincipalName, Role};
use onepack_server::{Limits, app};
use onepack_storage::{Store, migrate};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use zip::write::SimpleFileOptions;

struct Env {
    router: Router,
    _dir: TempDir,
    admin: String,
    reader: String,
}

struct Reply {
    status: StatusCode,
    request_id: Option<String>,
    json: Value,
}

async fn env() -> Env {
    let dir = TempDir::new().unwrap();
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
    let router = app(
        store,
        "https://packages.example.test",
        Limits {
            max_package_bytes: 1 << 20,
            ..Limits::default()
        },
    );
    let reader = tokens.pop().unwrap();
    let admin = tokens.pop().unwrap();
    Env {
        router,
        _dir: dir,
        admin,
        reader,
    }
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
    async fn call(&self, method: Method, path: &str, token: &str, body: Option<Value>) -> Reply {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let request_id = res
            .headers()
            .get("x-request-id")
            .map(|v| v.to_str().unwrap().to_owned());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            request_id,
            json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }

    async fn get(&self, path: &str, token: &str) -> Reply {
        self.call(Method::GET, path, token, None).await
    }

    async fn post(&self, path: &str, token: &str, body: Value) -> Reply {
        self.call(Method::POST, path, token, Some(body)).await
    }

    async fn push(&self, package: &[u8]) -> StatusCode {
        let boundary = "b";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"p.nupkg\"\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(package);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let req = Request::put("/nuget/internal/v2/package")
            .header("X-NuGet-ApiKey", &self.admin)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        self.router.clone().oneshot(req).await.unwrap().status()
    }

    /// Recorre todas las páginas y devuelve los elementos y el número de páginas.
    async fn all_pages(&self, path: &str, token: &str) -> (Vec<Value>, usize) {
        let separator = if path.contains('?') { '&' } else { '?' };
        let mut items = Vec::new();
        let mut pages = 0;
        let mut url = path.to_owned();
        loop {
            let reply = self.get(&url, token).await;
            assert_eq!(reply.status, StatusCode::OK, "{url}: {}", reply.json);
            let page: api::Page<Value> = serde_json::from_value(reply.json).unwrap();
            pages += 1;
            items.extend(page.items);
            match page.next_cursor {
                Some(cursor) => url = format!("{path}{separator}cursor={cursor}"),
                None => return (items, pages),
            }
        }
    }
}

#[tokio::test]
async fn capabilities_and_whoami() {
    let env = env().await;
    let caps: api::Capabilities =
        serde_json::from_value(env.get("/api/v1/capabilities", &env.reader).await.json).unwrap();
    assert_eq!(caps.api_version, api::API_VERSION);
    for c in api::capability::ALL {
        assert!(caps.supports(c), "{c}");
    }
    let me: api::WhoAmI =
        serde_json::from_value(env.get("/api/v1/whoami", &env.reader).await.json).unwrap();
    assert_eq!(me.principal, "reader");
    assert!(me.token_expires_at.is_some());
    assert_eq!(me.grants[0].role, "reader");
}

#[tokio::test]
async fn errors_carry_code_action_and_request_id() {
    let env = env().await;
    let reply = env.get("/api/v1/principals", &env.reader).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let error: api::ErrorBody = serde_json::from_value(reply.json).unwrap();
    assert_eq!(error.error.code, "AUTH_ADMIN_REQUIRED");
    assert!(error.error.action.is_some());
    assert_eq!(error.error.request_id, reply.request_id);
    assert_eq!(reply.request_id.as_deref().map(str::len), Some(16));

    // Sin credencial también, y cada petición tiene su propio id.
    let unauthenticated = env.get("/api/v1/whoami", "opk_bad").await;
    assert_eq!(unauthenticated.status, StatusCode::UNAUTHORIZED);
    assert_eq!(unauthenticated.json["error"]["code"], "AUTH_REQUIRED");
    assert!(unauthenticated.json["error"]["action"].is_string());
    assert_ne!(unauthenticated.request_id, reply.request_id);
}

#[tokio::test]
async fn admin_operations_require_administrator() {
    let env = env().await;
    let r = &env.reader;
    for (method, path, body) in [
        (Method::POST, "/api/v1/feeds", json!({"name": "x"})),
        (Method::PATCH, "/api/v1/feeds/internal", json!({})),
        (Method::GET, "/api/v1/principals", Value::Null),
        (
            Method::POST,
            "/api/v1/principals",
            json!({"name": "x", "kind": "user"}),
        ),
        (
            Method::POST,
            "/api/v1/principals/admin/disable",
            Value::Null,
        ),
        (Method::GET, "/api/v1/tokens", Value::Null),
        (
            Method::POST,
            "/api/v1/tokens",
            json!({"principal": "reader", "expires_in_days": 1}),
        ),
        (Method::GET, "/api/v1/grants", Value::Null),
        (
            Method::PUT,
            "/api/v1/feeds/internal/grants/reader",
            json!({"role": "maintainer"}),
        ),
        (Method::GET, "/api/v1/audit", Value::Null),
    ] {
        let reply = env.call(method.clone(), path, r, Some(body)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(reply.json["error"]["code"], "AUTH_ADMIN_REQUIRED");
    }
    // Los intentos quedan auditados.
    let audit = env.get("/api/v1/audit?action=principal.", &env.admin).await;
    assert!(
        audit.json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["outcome"] == "denied" && e["actor"].as_str().unwrap().starts_with("reader"))
    );
}

#[tokio::test]
async fn full_identity_lifecycle_through_the_api() {
    let env = env().await;
    let a = &env.admin;

    let feed = env
        .post("/api/v1/feeds", a, json!({"name": "payments"}))
        .await;
    assert_eq!(feed.status, StatusCode::CREATED, "{}", feed.json);
    assert_eq!(feed.json["versions"], 0);
    let dup = env
        .post("/api/v1/feeds", a, json!({"name": "payments"}))
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    assert_eq!(dup.json["error"]["code"], "FEED_EXISTS");
    let bad = env
        .post("/api/v1/feeds", a, json!({"name": "Bad Name"}))
        .await;
    assert_eq!(bad.json["error"]["code"], "INVALID_NAME");

    let quota = env
        .call(
            Method::PATCH,
            "/api/v1/feeds/payments",
            a,
            Some(json!({"max_versions": 10})),
        )
        .await;
    assert_eq!(quota.json["max_versions"], 10);
    assert_eq!(quota.json["max_storage_bytes"], Value::Null);

    let ci = env
        .post(
            "/api/v1/principals",
            a,
            json!({"name": "ci-payments", "kind": "service"}),
        )
        .await;
    assert_eq!(ci.status, StatusCode::CREATED);
    assert_eq!(ci.json["kind"], "service");

    let issued: api::IssuedToken = serde_json::from_value(
        env.post(
            "/api/v1/tokens",
            a,
            json!({"principal": "ci-payments", "name": "pipeline", "expires_in_days": 7}),
        )
        .await
        .json,
    )
    .unwrap();
    assert!(issued.token.starts_with("opk_"));

    let grant = env
        .call(
            Method::PUT,
            "/api/v1/feeds/payments/grants/ci-payments",
            a,
            Some(json!({"role": "publisher", "publish_patterns": ["Hemia.Payments.*"]})),
        )
        .await;
    assert_eq!(grant.status, StatusCode::OK, "{}", grant.json);
    assert_eq!(grant.json["publish_patterns"], json!(["hemia.payments.*"]));

    // El token nuevo funciona y ve solo su feed.
    let me = env.get("/api/v1/whoami", &issued.token).await;
    assert_eq!(me.json["grants"][0]["feed"], "payments");
    let (feeds, _) = env.all_pages("/api/v1/feeds", &issued.token).await;
    assert_eq!(feeds.len(), 1);

    let grants = env.get("/api/v1/grants?feed=payments", a).await;
    assert_eq!(grants.json["items"].as_array().unwrap().len(), 1);
    let tokens = env.get("/api/v1/tokens?principal=ci-payments", a).await;
    let listed = &tokens.json["items"][0];
    assert_eq!(listed["id"], issued.id);
    assert!(
        !tokens.json.to_string().contains(&issued.token),
        "sin secretos"
    );

    // Revocar corta el acceso de inmediato.
    let revoked = env
        .post(
            &format!("/api/v1/tokens/{}/revoke", issued.id),
            a,
            json!({}),
        )
        .await;
    assert!(revoked.json["revoked_at"].is_string());
    assert_eq!(
        env.get("/api/v1/whoami", &issued.token).await.status,
        StatusCode::UNAUTHORIZED
    );
    let again = env
        .post(
            &format!("/api/v1/tokens/{}/revoke", issued.id),
            a,
            json!({}),
        )
        .await;
    assert_eq!(again.json["error"]["code"], "TOKEN_NOT_FOUND");

    let removed = env
        .call(
            Method::DELETE,
            "/api/v1/feeds/payments/grants/ci-payments",
            a,
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let removed = env
        .call(
            Method::DELETE,
            "/api/v1/feeds/payments/grants/ci-payments",
            a,
            None,
        )
        .await;
    assert_eq!(removed.json["error"]["code"], "GRANT_NOT_FOUND");
}

#[tokio::test]
async fn disabling_principals() {
    let env = env().await;
    let disabled = env
        .post("/api/v1/principals/reader/disable", &env.admin, json!({}))
        .await;
    assert_eq!(disabled.json["disabled"], true);
    assert_eq!(
        env.get("/api/v1/whoami", &env.reader).await.status,
        StatusCode::UNAUTHORIZED
    );
    let last = env
        .post("/api/v1/principals/admin/disable", &env.admin, json!({}))
        .await;
    assert_eq!(last.status, StatusCode::CONFLICT);
    assert_eq!(last.json["error"]["code"], "LAST_ADMIN");
    let missing = env
        .post("/api/v1/principals/nobody/disable", &env.admin, json!({}))
        .await;
    assert_eq!(missing.json["error"]["code"], "PRINCIPAL_NOT_FOUND");
}

#[tokio::test]
async fn cursor_pagination() {
    let env = env().await;
    for i in 0..7 {
        let r = env
            .post(
                "/api/v1/feeds",
                &env.admin,
                json!({"name": format!("feed-{i}")}),
            )
            .await;
        assert_eq!(r.status, StatusCode::CREATED);
    }
    let (feeds, pages) = env.all_pages("/api/v1/feeds?limit=3", &env.admin).await;
    assert_eq!(pages, 3);
    let names: Vec<&str> = feeds.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 8);
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "orden estable");

    for bad in [
        "/api/v1/feeds?cursor=%%%",
        "/api/v1/feeds?limit=0",
        "/api/v1/feeds?limit=9999",
    ] {
        let reply = env.get(bad, &env.admin).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{bad}");
    }

    // Auditoría: del más reciente al más antiguo, paginada y filtrable.
    let (events, pages) = env
        .all_pages("/api/v1/audit?action=feed.create&limit=2", &env.admin)
        .await;
    assert_eq!(events.len(), 8);
    assert_eq!(pages, 4);
    let ids: Vec<i64> = events.iter().map(|e| e["id"].as_i64().unwrap()).collect();
    assert!(ids.windows(2).all(|w| w[0] > w[1]));
    assert!(events.iter().all(|e| e["action"] == "feed.create"));
}

#[tokio::test]
async fn packages_listing_and_state_changes() {
    let env = env().await;
    for (id, version) in [
        ("Hemia.Core", "1.0.0"),
        ("Hemia.Core", "1.10.0"),
        ("Hemia.Core", "1.2.0"),
        ("Hemia.Logging", "2.0.0"),
        ("Acme.Tools", "0.1.0"),
    ] {
        assert_eq!(env.push(&nupkg(id, version)).await, StatusCode::CREATED);
    }

    let (packages, pages) = env
        .all_pages("/api/v1/feeds/internal/packages?limit=2", &env.reader)
        .await;
    assert_eq!(pages, 2);
    let summary: Vec<(String, u64, String)> = packages
        .iter()
        .map(|p| {
            (
                p["id"].as_str().unwrap().to_owned(),
                p["versions"].as_u64().unwrap(),
                p["latest_version"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("Acme.Tools".to_owned(), 1, "0.1.0".to_owned()),
            ("Hemia.Core".to_owned(), 3, "1.10.0".to_owned()),
            ("Hemia.Logging".to_owned(), 1, "2.0.0".to_owned()),
        ]
    );

    let versions = env
        .get("/api/v1/feeds/internal/packages/hemia.core", &env.reader)
        .await;
    let order: Vec<&str> = versions
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["version"].as_str().unwrap())
        .collect();
    assert_eq!(order, ["1.0.0", "1.2.0", "1.10.0"]);

    // Unlist exige Publisher; el lector recibe el código y la acción sugerida.
    let path = "/api/v1/feeds/internal/packages/hemia.core/1.0.0";
    let denied = env
        .post(&format!("{path}/unlist"), &env.reader, json!({}))
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.json["error"]["code"], "AUTH_SCOPE_MISSING");
    assert!(
        denied.json["error"]["action"]
            .as_str()
            .unwrap()
            .contains("onepack grant add")
    );
    let unlisted = env
        .post(&format!("{path}/unlist"), &env.admin, json!({}))
        .await;
    assert_eq!(unlisted.json["listed"], false);
    assert_eq!(unlisted.json["changed"], true);
    let again = env
        .post(&format!("{path}/unlist"), &env.admin, json!({}))
        .await;
    assert_eq!(again.json["changed"], false);
    let relisted = env
        .post(&format!("{path}/relist"), &env.admin, json!({}))
        .await;
    assert_eq!(relisted.json["listed"], true);

    let blocked = env
        .post(
            &format!("{path}/block"),
            &env.admin,
            json!({"reason": "test"}),
        )
        .await;
    assert_eq!(blocked.json["availability"], "blocked");
    let state = env.get(path, &env.admin).await;
    assert_eq!(state.json["blocked_reason"], "test");

    let missing = env
        .get("/api/v1/feeds/internal/packages/nope", &env.reader)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}
