//! Operación del servidor (Fase 7): salud, preparación, métricas Prometheus y modo
//! mantenimiento (ADR-017).
//!
//! `/healthz` y `/readyz` no requieren credencial: los usan balanceadores, orquestadores y
//! systemd, y no revelan datos. `/metrics` solo se sirve en el listener de operación
//! (`--ops-listen`), pensado para una interfaz interna.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use axum::Json;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::json;

use crate::AppState;
use crate::auth::Surface;
use crate::nuget_api::ApiError;

/// Segundos que se sugiere esperar durante un mantenimiento.
pub const MAINTENANCE_RETRY_SECS: u64 = 30;

const LATENCY_BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Default)]
struct Histogram {
    buckets: [u64; LATENCY_BUCKETS.len()],
    count: u64,
    sum: f64,
}

/// Métricas del proceso. Las etiquetas tienen cardinalidad acotada: superficie, método
/// estándar, código de estado y códigos de error estables; nunca rutas ni ids.
#[derive(Default)]
pub struct Metrics {
    requests: Mutex<BTreeMap<(&'static str, &'static str, u16), u64>>,
    latency: Mutex<BTreeMap<&'static str, Histogram>>,
    errors: Mutex<BTreeMap<&'static str, u64>>,
    uploads: Mutex<BTreeMap<&'static str, u64>>,
    uploads_in_flight: AtomicI64,
}

pub fn metrics() -> &'static Metrics {
    static METRICS: OnceLock<Metrics> = OnceLock::new();
    METRICS.get_or_init(Metrics::default)
}

fn method_label(m: &Method) -> &'static str {
    match *m {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        _ => "OTHER",
    }
}

impl Metrics {
    fn record_request(&self, surface: &'static str, method: &'static str, status: u16, secs: f64) {
        *self
            .requests
            .lock()
            .expect("mutex no envenenado")
            .entry((surface, method, status))
            .or_default() += 1;
        let mut latency = self.latency.lock().expect("mutex no envenenado");
        let h = latency.entry(surface).or_default();
        for (i, le) in LATENCY_BUCKETS.iter().enumerate() {
            if secs <= *le {
                h.buckets[i] += 1;
            }
        }
        h.count += 1;
        h.sum += secs;
    }

    /// Error devuelto, por código estable (rate limit, cuotas, paquetes rechazados…).
    pub fn record_error(&self, code: &'static str) {
        *self
            .errors
            .lock()
            .expect("mutex no envenenado")
            .entry(code)
            .or_default() += 1;
    }

    /// Resultado de una subida: `published`, `conflict` o `rejected`.
    pub fn record_upload(&self, outcome: &'static str) {
        *self
            .uploads
            .lock()
            .expect("mutex no envenenado")
            .entry(outcome)
            .or_default() += 1;
    }

    pub fn upload_started(&self) -> UploadGuard {
        self.uploads_in_flight.fetch_add(1, Ordering::Relaxed);
        UploadGuard
    }

    fn render(&self, free_bytes: Option<u64>, maintenance: bool) -> String {
        let mut out = format!(
            "# HELP onepack_build_info Versión de onepackd.\n# TYPE onepack_build_info gauge\nonepack_build_info{{version=\"{}\"}} 1\n",
            env!("CARGO_PKG_VERSION")
        );
        out.push_str("# HELP onepack_http_requests_total Peticiones HTTP atendidas.\n# TYPE onepack_http_requests_total counter\n");
        for ((surface, method, status), n) in
            self.requests.lock().expect("mutex no envenenado").iter()
        {
            out.push_str(&format!(
                "onepack_http_requests_total{{surface=\"{surface}\",method=\"{method}\",status=\"{status}\"}} {n}\n"
            ));
        }
        out.push_str("# HELP onepack_http_request_duration_seconds Latencia de las peticiones HTTP.\n# TYPE onepack_http_request_duration_seconds histogram\n");
        for (surface, h) in self.latency.lock().expect("mutex no envenenado").iter() {
            for (i, le) in LATENCY_BUCKETS.iter().enumerate() {
                out.push_str(&format!(
                    "onepack_http_request_duration_seconds_bucket{{surface=\"{surface}\",le=\"{le}\"}} {}\n",
                    h.buckets[i]
                ));
            }
            out.push_str(&format!(
                "onepack_http_request_duration_seconds_bucket{{surface=\"{surface}\",le=\"+Inf\"}} {}\n\
                 onepack_http_request_duration_seconds_sum{{surface=\"{surface}\"}} {}\n\
                 onepack_http_request_duration_seconds_count{{surface=\"{surface}\"}} {}\n",
                h.count, h.sum, h.count
            ));
        }
        out.push_str("# HELP onepack_errors_total Respuestas de error por código estable.\n# TYPE onepack_errors_total counter\n");
        for (code, n) in self.errors.lock().expect("mutex no envenenado").iter() {
            out.push_str(&format!("onepack_errors_total{{code=\"{code}\"}} {n}\n"));
        }
        out.push_str("# HELP onepack_uploads_total Subidas de paquetes por resultado.\n# TYPE onepack_uploads_total counter\n");
        for (outcome, n) in self.uploads.lock().expect("mutex no envenenado").iter() {
            out.push_str(&format!(
                "onepack_uploads_total{{outcome=\"{outcome}\"}} {n}\n"
            ));
        }
        out.push_str(&format!(
            "# HELP onepack_uploads_in_flight Subidas en curso.\n# TYPE onepack_uploads_in_flight gauge\nonepack_uploads_in_flight {}\n",
            self.uploads_in_flight.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "# HELP onepack_maintenance Modo mantenimiento activo (1) o no (0).\n# TYPE onepack_maintenance gauge\nonepack_maintenance {}\n",
            u8::from(maintenance)
        ));
        if let Some(free) = free_bytes {
            out.push_str(&format!(
                "# HELP onepack_data_dir_free_bytes Espacio libre en el directorio de datos.\n# TYPE onepack_data_dir_free_bytes gauge\nonepack_data_dir_free_bytes {free}\n"
            ));
        }
        out
    }
}

/// Resta la subida en curso al terminar el handler, también si falla.
pub struct UploadGuard;

impl Drop for UploadGuard {
    fn drop(&mut self) {
        metrics().uploads_in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Cuenta peticiones y latencias. Va por fuera de la autenticación: también cuenta los `401`.
pub async fn track(req: Request, next: Next) -> Response {
    let surface = match Surface::of(req.uri().path()) {
        _ if is_probe(req.uri().path()) => "probe",
        Surface::Admin => "admin",
        Surface::NuGet => "nuget",
    };
    let method = method_label(req.method());
    let started = Instant::now();
    let res = next.run(req).await;
    metrics().record_request(
        surface,
        method,
        res.status().as_u16(),
        started.elapsed().as_secs_f64(),
    );
    res
}

pub fn is_probe(path: &str) -> bool {
    matches!(path, "/healthz" | "/readyz")
}

/// En mantenimiento, las mutaciones reciben `503` con `Retry-After`; las lecturas siguen.
pub async fn maintenance_guard(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let mutation = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if mutation {
        match state.store.maintenance().await {
            Ok(Some(m)) => {
                tracing::info!(reason = %m.reason, "mutación rechazada por mantenimiento");
                return ApiError::Maintenance.render(Surface::of(req.uri().path()));
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!(error = %e, "no se pudo consultar el modo mantenimiento");
                return ApiError::Internal(e.to_string()).render(Surface::of(req.uri().path()));
            }
        }
    }
    next.run(req).await
}

pub async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

/// Preparado si la base responde y el almacenamiento es accesible.
pub async fn readyz(State(state): State<Arc<AppState>>) -> Response {
    let database = match state.store.ping().await {
        Ok(()) => "ok".to_owned(),
        Err(e) => {
            tracing::warn!(error = %e, "readyz: base no disponible");
            "error".to_owned()
        }
    };
    let dir = state.store.data_dir().to_owned();
    let storage = match tokio::task::spawn_blocking(move || {
        let blobs = std::fs::metadata(dir.join("blobs"))?;
        let staging = std::fs::metadata(dir.join("staging"))?;
        if blobs.is_dir() && staging.is_dir() {
            fs4::available_space(&dir)
        } else {
            Err(std::io::Error::other("directorios de datos incompletos"))
        }
    })
    .await
    {
        Ok(Ok(_)) => "ok".to_owned(),
        _ => "error".to_owned(),
    };
    let maintenance = state.store.maintenance().await.ok().flatten();
    let ready = database == "ok" && storage == "ok";
    let body = json!({
        "status": if ready { "ready" } else { "not_ready" },
        "checks": { "database": database, "storage": storage },
        "maintenance": maintenance.map(|m| json!({ "reason": m.reason, "expires_at": m.expires_at })),
    });
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body)).into_response()
}

pub async fn metrics_text(State(state): State<Arc<AppState>>) -> Response {
    let dir = state.store.data_dir().to_owned();
    let free = tokio::task::spawn_blocking(move || fs4::available_space(&dir).ok())
        .await
        .ok()
        .flatten();
    let maintenance = state.store.maintenance().await.ok().flatten().is_some();
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        metrics().render(free, maintenance),
    )
        .into_response()
}

/// Router del listener de operación: métricas y sondas, sin autenticación.
pub fn ops_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/metrics", get(metrics_text))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
}
