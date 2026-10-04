//! Pruebas de la Fase 3: registros, búsqueda, autocompletado y unlist/relist (ADR-009, ADR-013).

use std::io::{Cursor, Write};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::http::{HeaderValue, Method, Request, StatusCode};
use http_body_util::BodyExt;
use onepack_core::{FeedName, PrincipalKind, PrincipalName};
use onepack_server::app;
use onepack_storage::{Store, migrate};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use zip::write::SimpleFileOptions;

const BOUNDARY: &str = "onepack-catalog";
const BASE: &str = "https://packages.example.test/nuget/internal";

async fn server() -> (Router, TempDir) {
    let dir = TempDir::new().unwrap();
    migrate(dir.path()).await.unwrap();
    let store = Store::open(dir.path()).await.unwrap();
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
    let token = store
        .create_token(&admin, None, 3600, "test")
        .await
        .unwrap()
        .token;
    let api_key = HeaderValue::from_str(&token).unwrap();
    let router = app(Arc::new(store), "https://packages.example.test", 1 << 20).layer(
        axum::middleware::map_request(move |mut req: AxumRequest| {
            let api_key = api_key.clone();
            async move {
                req.headers_mut().insert("X-NuGet-ApiKey", api_key);
                req
            }
        }),
    );
    (router, dir)
}

/// Paquete con metadatos y dependencias arbitrarios (XML dentro de `<metadata>`).
fn nupkg(id: &str, version: &str, extra: &str) -> Vec<u8> {
    let nuspec = format!(
        r#"<?xml version="1.0"?><package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd">
<metadata><id>{id}</id><version>{version}</version><authors>Tester</authors>
<description>Paquete {id}</description>{extra}</metadata></package>"#
    );
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(format!("{id}.nuspec"), SimpleFileOptions::default())
        .unwrap();
    zip.write_all(nuspec.as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
}

async fn send(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<Vec<u8>>,
) -> (StatusCode, Vec<u8>) {
    let req = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(package) => {
            let mut b = format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"p.nupkg\"\r\n\r\n"
            )
            .into_bytes();
            b.extend_from_slice(&package);
            b.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
            req.header("content-type", format!("multipart/form-data; boundary={BOUNDARY}"))
                .body(Body::from(b))
        }
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    (
        status,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

async fn push(router: &Router, package: Vec<u8>) {
    let (status, body) = send(
        router,
        Method::PUT,
        "/nuget/internal/v2/package",
        Some(package),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
}

async fn get_json(router: &Router, uri: &str) -> Value {
    let (status, body) = send(router, Method::GET, uri, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{uri}: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).unwrap()
}

fn ids(search: &Value) -> Vec<&str> {
    search["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn registration_exposes_metadata_and_dependency_groups() {
    let (router, _dir) = server().await;
    push(&router, nupkg("Hemia.Core", "1.0.0", "")).await;
    push(
        &router,
        nupkg(
            "Hemia.Logging",
            "1.2.0-beta.1+sha.abc",
            r#"<title>Hemia Logging</title><tags>logging tracing</tags>
            <license type="expression">MIT</license><projectUrl>https://example.test</projectUrl>
            <dependencies><group targetFramework="net8.0">
              <dependency id="Hemia.Core" version="[1.0.0, 2.0.0)" />
            </group></dependencies>"#,
        ),
    )
    .await;

    let index = get_json(
        &router,
        "/nuget/internal/v3/registration/hemia.logging/index.json",
    )
    .await;
    assert_eq!(
        index["@id"],
        format!("{BASE}/v3/registration/hemia.logging/index.json")
    );
    assert_eq!(index["count"], 1);
    let page = &index["items"][0];
    assert_eq!(page["lower"], "1.2.0-beta.1");
    assert_eq!(page["upper"], "1.2.0-beta.1");
    let leaf = &page["items"][0];
    let entry = &leaf["catalogEntry"];
    assert_eq!(entry["id"], "Hemia.Logging");
    assert_eq!(entry["version"], "1.2.0-beta.1+sha.abc");
    assert_eq!(entry["title"], "Hemia Logging");
    assert_eq!(entry["tags"], json!(["logging", "tracing"]));
    assert_eq!(entry["authors"], "Tester");
    assert_eq!(entry["licenseExpression"], "MIT");
    assert_eq!(entry["listed"], true);
    assert_eq!(
        entry["dependencyGroups"][0],
        json!({
            "@type": "PackageDependencyGroup",
            "targetFramework": "net8.0",
            "dependencies": [{
                "@type": "PackageDependency",
                "id": "Hemia.Core",
                "range": "[1.0.0, 2.0.0)",
                "registration": format!("{BASE}/v3/registration/hemia.core/index.json"),
            }],
        })
    );
    let content = leaf["packageContent"].as_str().unwrap();
    assert_eq!(
        content,
        format!("{BASE}/v3/flat/hemia.logging/1.2.0-beta.1/hemia.logging.1.2.0-beta.1.nupkg")
    );
    let (status, _) = send(
        &router,
        Method::GET,
        content
            .strip_prefix("https://packages.example.test")
            .unwrap(),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "packageContent apunta a un recurso descargable"
    );

    let leaf_doc = get_json(
        &router,
        "/nuget/internal/v3/registration/hemia.logging/1.2.0-beta.1.json",
    )
    .await;
    assert_eq!(leaf_doc["listed"], true);
    assert_eq!(leaf_doc["registration"], index["@id"]);
}

#[tokio::test]
async fn large_registrations_are_paged() {
    let (router, _dir) = server().await;
    for minor in 0..130 {
        push(&router, nupkg("Many", &format!("1.{minor}.0"), "")).await;
    }
    let index = get_json(&router, "/nuget/internal/v3/registration/many/index.json").await;
    assert_eq!(index["count"], 3);
    let pages = index["items"].as_array().unwrap();
    assert!(
        pages.iter().all(|p| p.get("items").is_none()),
        "con >128 versiones las páginas no van inline"
    );
    assert_eq!(pages[0]["lower"], "1.0.0");
    assert_eq!(pages[0]["upper"], "1.63.0");
    assert_eq!(pages[2]["count"], 2);

    let url = pages[1]["@id"]
        .as_str()
        .unwrap()
        .strip_prefix("https://packages.example.test")
        .unwrap();
    let page = get_json(&router, url).await;
    assert_eq!(page["count"], 64);
    assert_eq!(page["items"].as_array().unwrap().len(), 64);
    assert_eq!(page["items"][0]["catalogEntry"]["version"], "1.64.0");

    let (status, _) = send(
        &router,
        Method::GET,
        "/nuget/internal/v3/registration/many/page/1.0.0/9.9.9.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn search_filters_prerelease_and_semver2() {
    let (router, _dir) = server().await;
    push(&router, nupkg("Stable", "1.0.0", "")).await;
    push(&router, nupkg("Pre", "1.0.0-beta", "")).await;
    push(&router, nupkg("SemVer2", "1.0.0-rc.1", "")).await;
    push(&router, nupkg("Meta", "1.0.0+build", "")).await;
    let q = "/nuget/internal/v3/query";

    assert_eq!(ids(&get_json(&router, q).await), ["Stable"]);
    assert_eq!(
        ids(&get_json(&router, &format!("{q}?prerelease=true")).await),
        ["Pre", "Stable"]
    );
    assert_eq!(
        ids(&get_json(&router, &format!("{q}?semVerLevel=2.0.0")).await),
        ["Meta", "Stable"]
    );
    assert_eq!(
        ids(&get_json(&router, &format!("{q}?prerelease=true&semVerLevel=2.0.0")).await),
        ["Meta", "Pre", "SemVer2", "Stable"]
    );
}

#[tokio::test]
async fn search_pages_and_matches_text() {
    let (router, _dir) = server().await;
    push(
        &router,
        nupkg("Hemia.Logging", "1.0.0", "<tags>observability</tags>"),
    )
    .await;
    push(
        &router,
        nupkg("Hemia.Logging", "1.1.0", "<tags>observability</tags>"),
    )
    .await;
    push(
        &router,
        nupkg(
            "Hemia.Tracing",
            "1.0.0",
            "<title>Distributed tracing</title>",
        ),
    )
    .await;
    push(&router, nupkg("Other.Hemia", "1.0.0", "")).await;
    push(&router, nupkg("Unrelated", "1.0.0", "")).await;
    let q = "/nuget/internal/v3/query";

    let all = get_json(&router, &format!("{q}?q=hemia")).await;
    assert_eq!(all["totalHits"], 3);
    assert_eq!(
        ids(&all),
        ["Hemia.Logging", "Hemia.Tracing", "Other.Hemia"],
        "prefijo de id primero"
    );
    let logging = &all["data"][0];
    assert_eq!(logging["version"], "1.1.0", "la versión es la más reciente");
    assert_eq!(logging["versions"].as_array().unwrap().len(), 2);
    assert_eq!(
        logging["registration"],
        format!("{BASE}/v3/registration/hemia.logging/index.json")
    );

    let page = get_json(&router, &format!("{q}?q=hemia&skip=1&take=1")).await;
    assert_eq!(page["totalHits"], 3);
    assert_eq!(ids(&page), ["Hemia.Tracing"]);

    assert_eq!(
        ids(&get_json(&router, &format!("{q}?q=observability")).await),
        ["Hemia.Logging"]
    );
    assert_eq!(
        ids(&get_json(&router, &format!("{q}?q=distributed%20tracing")).await),
        ["Hemia.Tracing"]
    );
    assert_eq!(
        ids(&get_json(&router, &format!("{q}?q=packageid:hemia.logging")).await),
        ["Hemia.Logging"]
    );
    assert_eq!(
        ids(&get_json(&router, &format!("{q}?q=id:tracing")).await),
        ["Hemia.Tracing"]
    );
    assert_eq!(
        get_json(&router, &format!("{q}?q=nothing-matches")).await["totalHits"],
        0
    );
}

#[tokio::test]
async fn search_filters_by_package_type() {
    let (router, _dir) = server().await;
    push(&router, nupkg("Lib", "1.0.0", "")).await;
    push(
        &router,
        nupkg(
            "Tool",
            "1.0.0",
            r#"<packageTypes><packageType name="DotnetTool" /></packageTypes>"#,
        ),
    )
    .await;
    let q = "/nuget/internal/v3/query";
    assert_eq!(
        ids(&get_json(&router, &format!("{q}?packageType=DotnetTool")).await),
        ["Tool"]
    );
    assert_eq!(
        ids(&get_json(&router, &format!("{q}?packageType=dependency")).await),
        ["Lib"]
    );
    let auto = get_json(
        &router,
        "/nuget/internal/v3/autocomplete?packageType=DotnetTool",
    )
    .await;
    assert_eq!(auto["data"], json!(["Tool"]));
}

#[tokio::test]
async fn autocomplete_ids_and_versions() {
    let (router, _dir) = server().await;
    for (id, v) in [
        ("Hemia.Logging", "1.0.0"),
        ("Hemia.Logging", "1.10.0"),
        ("Hemia.Logging", "1.2.0"),
        ("Hemia.Logging", "2.0.0-beta"),
        ("My.Hemia", "1.0.0"),
    ] {
        push(&router, nupkg(id, v, "")).await;
    }
    let a = "/nuget/internal/v3/autocomplete";
    let ids = get_json(&router, &format!("{a}?q=hemia")).await;
    assert_eq!(ids["totalHits"], 2);
    assert_eq!(ids["data"], json!(["Hemia.Logging", "My.Hemia"]));

    let versions = get_json(&router, &format!("{a}?id=hemia.logging")).await;
    assert_eq!(versions["data"], json!(["1.0.0", "1.2.0", "1.10.0"]));
    let versions = get_json(&router, &format!("{a}?id=Hemia.Logging&prerelease=true")).await;
    assert_eq!(
        versions["data"],
        json!(["1.0.0", "1.2.0", "1.10.0", "2.0.0-beta"])
    );
    assert_eq!(
        get_json(&router, &format!("{a}?id=missing")).await["data"],
        json!([])
    );
}

#[tokio::test]
async fn unlisted_versions_are_hidden_from_discovery_but_still_restorable() {
    let (router, _dir) = server().await;
    push(&router, nupkg("Hemia.Core", "1.0.0", "")).await;
    let original = nupkg("Hemia.Core", "2.0.0", "");
    push(&router, original.clone()).await;

    let (status, _) = send(
        &router,
        Method::DELETE,
        "/nuget/internal/v2/package/Hemia.Core/2.0.0",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(
        &router,
        Method::DELETE,
        "/nuget/internal/v2/package/Hemia.Core/2.0.0",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "unlist es idempotente");

    let search = get_json(&router, "/nuget/internal/v3/query?q=hemia.core").await;
    assert_eq!(search["data"][0]["version"], "1.0.0");
    assert_eq!(search["data"][0]["versions"].as_array().unwrap().len(), 1);
    let auto = get_json(&router, "/nuget/internal/v3/autocomplete?id=hemia.core").await;
    assert_eq!(auto["data"], json!(["1.0.0"]));

    let index = get_json(
        &router,
        "/nuget/internal/v3/registration/hemia.core/index.json",
    )
    .await;
    let listed: Vec<(&str, bool)> = index["items"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["catalogEntry"]["version"].as_str().unwrap(),
                l["catalogEntry"]["listed"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(listed, [("1.0.0", true), ("2.0.0", false)]);

    let flat = get_json(&router, "/nuget/internal/v3/flat/hemia.core/index.json").await;
    assert_eq!(flat["versions"], json!(["1.0.0", "2.0.0"]));
    let (status, body) = send(
        &router,
        Method::GET,
        "/nuget/internal/v3/flat/hemia.core/2.0.0/hemia.core.2.0.0.nupkg",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body, original,
        "una versión no listada sigue siendo descargable"
    );

    let (status, _) = send(
        &router,
        Method::POST,
        "/nuget/internal/v2/package/hemia.core/2.0.0",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let search = get_json(&router, "/nuget/internal/v3/query?q=hemia.core").await;
    assert_eq!(
        search["data"][0]["version"], "2.0.0",
        "relist la devuelve a la búsqueda"
    );

    let (status, _) = send(
        &router,
        Method::DELETE,
        "/nuget/internal/v2/package/Hemia.Core/9.9.9",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
