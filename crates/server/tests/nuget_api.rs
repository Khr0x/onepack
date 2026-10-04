use std::io::{Cursor, Write};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use onepack_core::FeedName;
use onepack_server::app;
use onepack_storage::{Store, migrate};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use zip::write::SimpleFileOptions;

const BOUNDARY: &str = "onepack-test-boundary";

async fn server() -> (Router, TempDir) {
    let dir = TempDir::new().unwrap();
    migrate(dir.path()).await.unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    for feed in ["internal", "customer-a"] {
        store
            .create_feed(&FeedName::parse(feed).unwrap())
            .await
            .unwrap();
    }
    let router = app(
        Arc::new(store),
        "https://packages.example.test/",
        1024 * 1024,
    );
    (router, dir)
}

fn staging_files(dir: &TempDir) -> usize {
    std::fs::read_dir(dir.path().join("staging"))
        .unwrap()
        .count()
}

fn nupkg(id: &str, version: &str) -> Vec<u8> {
    let nuspec = format!(
        r#"<?xml version="1.0"?><package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd">
<metadata><id>{id}</id><version>{version}</version><authors>t</authors><description>d</description></metadata></package>"#
    );
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(format!("{id}.nuspec"), SimpleFileOptions::default())
        .unwrap();
    zip.write_all(nuspec.as_bytes()).unwrap();
    zip.start_file("lib/netstandard2.0/a.dll", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(format!("{id}-{version}").as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
}

async fn send(router: &Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    (
        status,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

async fn get(router: &Router, uri: &str) -> (StatusCode, Vec<u8>) {
    send(router, Request::get(uri).body(Body::empty()).unwrap()).await
}

async fn push(router: &Router, feed: &str, package: &[u8]) -> StatusCode {
    push_to(router, &format!("/nuget/{feed}/v2/package"), package).await
}

async fn push_to(router: &Router, uri: &str, package: &[u8]) -> StatusCode {
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"package.nupkg\"\r\n\
         Content-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(package);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    let req = Request::builder()
        .method(Method::PUT)
        .uri(uri)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("X-NuGet-ApiKey", "spike")
        .body(Body::from(body))
        .unwrap();
    send(router, req).await.0
}

fn json_body(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

#[tokio::test]
async fn service_index_uses_public_url_and_announces_only_implemented_resources() {
    let (router, _dir) = server().await;
    let (status, body) = get(&router, "/nuget/internal/v3/index.json").await;
    assert_eq!(status, StatusCode::OK);
    let index = json_body(&body);
    assert_eq!(index["version"], "3.0.0");
    let resources: Vec<(String, String)> = index["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["@type"].as_str().unwrap().into(),
                r["@id"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        resources,
        vec![
            (
                "PackageBaseAddress/3.0.0".into(),
                "https://packages.example.test/nuget/internal/v3/flat/".into()
            ),
            (
                "PackagePublish/2.0.0".into(),
                "https://packages.example.test/nuget/internal/v2/package".into()
            ),
        ]
    );
}

#[tokio::test]
async fn unknown_feed_is_not_found() {
    let (router, _dir) = server().await;
    assert_eq!(
        get(&router, "/nuget/other/v3/index.json").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        push(&router, "other", &nupkg("A", "1.0.0")).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn publish_then_download_returns_identical_bytes() {
    let (router, _dir) = server().await;
    let package = nupkg("Hemia.Logging", "1.2.0-Beta.1+sha.abc");
    assert_eq!(
        push(&router, "internal", &package).await,
        StatusCode::CREATED
    );

    let (status, body) = get(&router, "/nuget/internal/v3/flat/hemia.logging/index.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body), json!({ "versions": ["1.2.0-beta.1"] }));

    let (status, body) = get(
        &router,
        "/nuget/internal/v3/flat/hemia.logging/1.2.0-beta.1/hemia.logging.1.2.0-beta.1.nupkg",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, package);

    let (status, body) = get(
        &router,
        "/nuget/internal/v3/flat/hemia.logging/1.2.0-beta.1/hemia.logging.nuspec",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("<id>Hemia.Logging</id>")
    );
}

#[tokio::test]
async fn versions_are_listed_in_nuget_precedence_order() {
    let (router, _dir) = server().await;
    for v in ["1.10.0", "1.2.0", "1.2.0-rc.1", "1.2.0-beta", "1.9.0"] {
        assert_eq!(
            push(&router, "internal", &nupkg("A", v)).await,
            StatusCode::CREATED
        );
    }
    let (_, body) = get(&router, "/nuget/internal/v3/flat/a/index.json").await;
    assert_eq!(
        json_body(&body),
        json!({ "versions": ["1.2.0-beta", "1.2.0-rc.1", "1.2.0", "1.9.0", "1.10.0"] })
    );
}

#[tokio::test]
async fn equivalent_identity_is_rejected_with_conflict() {
    let (router, _dir) = server().await;
    let original = nupkg("Hemia.Core", "1.0");
    assert_eq!(
        push(&router, "internal", &original).await,
        StatusCode::CREATED
    );

    for duplicate in [
        nupkg("Hemia.Core", "1.0"),
        nupkg("Hemia.Core", "1.0.0"),
        nupkg("Hemia.Core", "1.0.0.0"),
        nupkg("hemia.core", "1.0.0+other"),
    ] {
        assert_eq!(
            push(&router, "internal", &duplicate).await,
            StatusCode::CONFLICT
        );
    }

    let (_, body) = get(
        &router,
        "/nuget/internal/v3/flat/hemia.core/1.0.0/hemia.core.1.0.0.nupkg",
    )
    .await;
    assert_eq!(
        body, original,
        "un duplicado no debe sobrescribir el original"
    );
}

#[tokio::test]
async fn invalid_packages_are_rejected() {
    let (router, _dir) = server().await;
    assert_eq!(
        push(&router, "internal", b"not a zip").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        push(&router, "internal", &nupkg("Bad Id", "1.0.0")).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        push(&router, "internal", &nupkg("A", "1.0.0-01")).await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn oversized_package_is_rejected() {
    let (router, _dir) = server().await;
    let status = push(&router, "internal", &vec![0u8; 2 * 1024 * 1024]).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn invalid_or_unknown_paths_are_not_found() {
    let (router, _dir) = server().await;
    assert_eq!(
        push(&router, "internal", &nupkg("A", "1.0.0")).await,
        StatusCode::CREATED
    );
    for uri in [
        "/nuget/internal/v3/flat/missing/index.json",
        "/nuget/internal/v3/flat/a/2.0.0/a.2.0.0.nupkg",
        "/nuget/internal/v3/flat/a/1.0.0/b.1.0.0.nupkg",
        "/nuget/internal/v3/flat/a/1.0.0/a.1.0.0.zip",
        "/nuget/internal/v3/flat/a/not-a-version/a.nuspec",
        "/nuget/internal/v3/flat/..%2F..%2Fstaging/1.0.0/x.nupkg",
    ] {
        assert_eq!(get(&router, uri).await.0, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn publish_accepts_trailing_slash_like_the_nuget_client() {
    let (router, _dir) = server().await;
    let status = push_to(&router, "/nuget/internal/v2/package/", &nupkg("A", "1.0.0")).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn rejected_uploads_leave_nothing_in_staging() {
    let (router, dir) = server().await;
    assert_eq!(
        push(&router, "internal", b"not a zip").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        push(&router, "internal", &vec![0u8; 2 * 1024 * 1024]).await,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        push(&router, "internal", &nupkg("A", "1.0.0")).await,
        StatusCode::CREATED
    );
    assert_eq!(
        push(&router, "internal", &nupkg("A", "1.0.0")).await,
        StatusCode::CONFLICT
    );
    assert_eq!(staging_files(&dir), 0);
}

#[tokio::test]
async fn feeds_are_isolated() {
    let (router, _dir) = server().await;
    assert_eq!(
        push(&router, "internal", &nupkg("A", "1.0.0")).await,
        StatusCode::CREATED
    );
    assert_eq!(
        get(&router, "/nuget/customer-a/v3/flat/a/index.json")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        push(&router, "customer-a", &nupkg("A", "1.0.0")).await,
        StatusCode::CREATED
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_http_publishes_confirm_exactly_one() {
    let (router, _dir) = server().await;
    let packages: Vec<Vec<u8>> = (0..6)
        .map(|i| {
            // Mismo id y versión, contenido distinto.
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file("Race.nuspec", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(
                b"<package><metadata><id>Race</id><version>1.0.0</version></metadata></package>",
            )
            .unwrap();
            zip.start_file(format!("content-{i}.txt"), SimpleFileOptions::default())
                .unwrap();
            zip.finish().unwrap().into_inner()
        })
        .collect();

    let tasks: Vec<_> = packages
        .iter()
        .cloned()
        .map(|p| {
            let router = router.clone();
            tokio::spawn(async move { (push(&router, "internal", &p).await, p) })
        })
        .collect();
    let mut created = Vec::new();
    for t in tasks {
        let (status, package) = t.await.unwrap();
        match status {
            StatusCode::CREATED => created.push(package),
            StatusCode::CONFLICT => {}
            other => panic!("estado inesperado: {other}"),
        }
    }
    assert_eq!(created.len(), 1);
    let (_, body) = get(
        &router,
        "/nuget/internal/v3/flat/race/1.0.0/race.1.0.0.nupkg",
    )
    .await;
    assert_eq!(body, created[0]);
}
