//! Per-request identifier (ADR-015): it goes in the `X-Request-Id` header, in `/api/v1`
//! errors, and in the `request` span of traces, to correlate what the client sees with the log.

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

pub const HEADER: &str = "x-request-id";

tokio::task_local! {
    static REQUEST_ID: String;
}

/// Id of the current request, if any.
pub fn current() -> Option<String> {
    REQUEST_ID.try_with(Clone::clone).ok()
}

fn generate() -> String {
    let mut bytes = [0u8; 8];
    // Without system entropy the id is still useful for correlation; it is not a secret.
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        bytes = nanos.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// It is always generated on the server: an id that arrives from the client is not used.
pub async fn assign(req: Request, next: Next) -> Response {
    let id = generate();
    let mut res = REQUEST_ID.scope(id.clone(), next.run(req)).await;
    res.headers_mut().insert(
        HEADER,
        HeaderValue::from_str(&id).expect("hexadecimal is a valid header value"),
    );
    res
}
