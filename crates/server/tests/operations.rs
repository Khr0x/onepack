//! Operación (Fase 7): sondas sin credencial, modo mantenimiento y métricas. Las métricas
//! son del proceso y las comparten las pruebas de este archivo: se comprueba qué series
//! existen, no sus valores exactos.

use std::io::{Cursor, Write};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use onepack_core::{FeedName, PrincipalKind, PrincipalName};
use onepack_server::{Limits, app_with_ops};
use onepack_storage::{Store, migrate};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;
use zip::write::SimpleFileOptions;

struct Env {
    router: Router,
    ops: Router,
    store: Arc<Store>,
    _dir: TempDir,
    admin: String,
}

async fn env() -> Env {
    let dir = TempDir::new().unwrap();
    migrate(dir.path()).await.unwrap();
    let store = Arc::new(Store::open(dir.path()).await.unwrap());
    store
        .create_feed(&FeedName::parse("internal").unwrap(), "test")
        .await
        .unwrap();
    let admin = store
        .create_principal(
            &PrincipalName::parse("admin").unwrap(),
            PrincipalKind::User,
            true,
            "test",
        )
        .await
        .unwrap();
    let admin = store
        .create_token(&admin, None, 3600, "test")
        .await
        .unwrap()
        .token;
    let (router, ops) = app_with_ops(
        store.clone(),
        "https://packages.example.test",
        Limits::default(),
    );
    Env {
        router,
        ops,
        store,
        _dir: dir,
        admin,
    }
}

async fn send(router: &Router, req: Request<Body>) -> (StatusCode, Option<String>, String) {
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let retry = res
        .headers()
        .get(header::RETRY_AFTER)
        .map(|v| v.to_str().unwrap().to_owned());
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, retry, String::from_utf8_lossy(&body).into_owned())
}

fn get(path: &str, token: Option<&str>) -> Request<Body> {
    let mut req = Request::get(path);
    if let Some(t) = token {
        req = req.header("X-NuGet-ApiKey", t);
    }
    req.body(Body::empty()).unwrap()
}

fn push(token: &str, id: &str) -> Request<Body> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(format!("{id}.nuspec"), SimpleFileOptions::default())
        .unwrap();
    zip.write_all(
        format!("<package><metadata><id>{id}</id><version>1.0.0</version></metadata></package>")
            .as_bytes(),
    )
    .unwrap();
    let package = zip.finish().unwrap().into_inner();
    let mut body =
        b"--b\r\nContent-Disposition: form-data; name=\"package\"; filename=\"p.nupkg\"\r\n\r\n"
            .to_vec();
    body.extend_from_slice(&package);
    body.extend_from_slice(b"\r\n--b--\r\n");
    Request::put("/nuget/internal/v2/package")
        .header("X-NuGet-ApiKey", token)
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=b")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn probes_need_no_credentials_and_reveal_nothing() {
    let env = env().await;
    for router in [&env.router, &env.ops] {
        let (status, _, body) = send(router, get("/healthz", None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"status":"ok"}"#);
        let (status, _, body) = send(router, get("/readyz", None)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let ready: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(ready["status"], "ready");
        assert_eq!(ready["checks"]["database"], "ok");
        assert_eq!(ready["checks"]["storage"], "ok");
        assert!(!body.contains("internal"), "sin datos del registro");
    }
    // El resto sigue exigiendo credencial, y /metrics no existe en el listener principal.
    let (status, _, _) = send(&env.router, get("/metrics", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = send(
        &env.router,
        get("/healthz/../nuget/internal/v3/index.json", None),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
}

#[tokio::test]
async fn maintenance_rejects_mutations_but_serves_reads() {
    let env = env().await;
    let (status, _, _) = send(&env.router, push(&env.admin, "Before")).await;
    assert_eq!(status, StatusCode::CREATED);

    env.store
        .begin_maintenance("backup", 3600, "test")
        .await
        .unwrap()
        .unwrap();

    let (status, retry, body) = send(&env.router, push(&env.admin, "During")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.starts_with("MAINTENANCE: "), "{body}");
    assert_eq!(retry.as_deref(), Some("30"));

    let admin = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/feeds")
        .header(header::AUTHORIZATION, format!("Bearer {}", env.admin))
        .body(Body::from(r#"{"name":"x"}"#))
        .unwrap();
    let (status, _, body) = send(&env.router, admin).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains(r#""code":"MAINTENANCE""#), "{body}");

    // Las lecturas siguen, y /readyz informa del mantenimiento sin dejar de estar listo.
    let (status, _, _) = send(
        &env.router,
        get(
            "/nuget/internal/v3/flat/before/1.0.0/before.1.0.0.nupkg",
            Some(&env.admin),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, body) = send(&env.router, get("/readyz", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#""reason":"backup""#), "{body}");

    env.store.end_maintenance("test").await.unwrap();
    let (status, _, _) = send(&env.router, push(&env.admin, "After")).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn metrics_count_requests_uploads_and_errors() {
    let env = env().await;
    send(&env.router, push(&env.admin, "Metric.Pkg")).await;
    send(&env.router, push(&env.admin, "Metric.Pkg")).await; // 409
    send(&env.router, get("/nuget/internal/v3/index.json", None)).await; // 401
    send(
        &env.router,
        get("/nuget/internal/v3/index.json", Some(&env.admin)),
    )
    .await;

    let (status, _, text) = send(&env.ops, get("/metrics", None)).await;
    assert_eq!(status, StatusCode::OK);
    for expected in [
        "onepack_build_info{version=",
        r#"onepack_http_requests_total{surface="nuget",method="PUT",status="201"}"#,
        r#"onepack_http_requests_total{surface="nuget",method="GET",status="401"}"#,
        r#"onepack_uploads_total{outcome="published"}"#,
        r#"onepack_uploads_total{outcome="conflict"}"#,
        r#"onepack_errors_total{code="AUTH_REQUIRED"}"#,
        r#"onepack_http_request_duration_seconds_bucket{surface="nuget",le="+Inf"}"#,
        "onepack_uploads_in_flight ",
        "onepack_maintenance ",
        "onepack_data_dir_free_bytes ",
    ] {
        assert!(text.contains(expected), "falta {expected}:\n{text}");
    }
    // Ningún dato de petición en las etiquetas.
    assert!(!text.contains("metric.pkg") && !text.contains("internal"));
}
