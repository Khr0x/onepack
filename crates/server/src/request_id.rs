//! Identificador por petición (ADR-015): va en la cabecera `X-Request-Id`, en los errores de
//! `/api/v1` y en el span `request` de las trazas, para cruzar lo que ve el cliente con el log.

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

pub const HEADER: &str = "x-request-id";

tokio::task_local! {
    static REQUEST_ID: String;
}

/// Id de la petición en curso, si la hay.
pub fn current() -> Option<String> {
    REQUEST_ID.try_with(Clone::clone).ok()
}

fn generate() -> String {
    let mut bytes = [0u8; 8];
    // Sin entropía del sistema el id sigue sirviendo para correlacionar; no es un secreto.
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        bytes = nanos.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Se genera siempre en el servidor: un id que llega del cliente no se usa.
pub async fn assign(req: Request, next: Next) -> Response {
    let id = generate();
    let mut res = REQUEST_ID.scope(id.clone(), next.run(req)).await;
    res.headers_mut().insert(
        HEADER,
        HeaderValue::from_str(&id).expect("hexadecimal es un valor de cabecera válido"),
    );
    res
}
