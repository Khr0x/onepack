//! Contratos de la API administrativa `/api/v1`, compartidos por `onepackd` y `onepack`
//! (ADR-015).
//!
//! Compatibilidad: los campos nuevos se añaden como opcionales (`#[serde(default)]`) y nadie
//! rechaza campos desconocidos, así que un CLI N habla con un servidor N-1 y al revés. Lo
//! que un servidor sabe hacer lo anuncia en [`Capabilities`].

use serde::{Deserialize, Serialize};

#[cfg(feature = "client")]
pub mod client;

/// Versión del contrato. Cambia solo con cambios incompatibles.
pub const API_VERSION: u32 = 1;

/// Capacidades que anuncia un servidor en `GET /api/v1/capabilities`.
pub mod capability {
    pub const FEEDS: &str = "feeds";
    pub const FEED_QUOTAS: &str = "feeds.quotas";
    pub const PRINCIPALS: &str = "principals";
    pub const TOKENS: &str = "tokens";
    pub const GRANTS: &str = "grants";
    pub const PACKAGES: &str = "packages";
    pub const PACKAGE_LISTING: &str = "packages.listing";
    pub const PACKAGE_AVAILABILITY: &str = "packages.availability";
    pub const AUDIT: &str = "audit";

    /// Todas las que implementa esta versión.
    pub const ALL: &[&str] = &[
        FEEDS,
        FEED_QUOTAS,
        PRINCIPALS,
        TOKENS,
        GRANTS,
        PACKAGES,
        PACKAGE_LISTING,
        PACKAGE_AVAILABILITY,
        AUDIT,
    ];
}

/// Tamaño de página por defecto y máximo de los listados con cursor.
pub const DEFAULT_PAGE_SIZE: u32 = 50;
pub const MAX_PAGE_SIZE: u32 = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub server_version: String,
    pub api_version: u32,
    pub capabilities: Vec<String>,
}

impl Capabilities {
    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

/// Página de un listado. `next_cursor` es opaco; `None` indica la última página.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhoAmI {
    pub principal: String,
    pub kind: String,
    pub administrator: bool,
    pub token_id: String,
    #[serde(default)]
    pub token_expires_at: Option<String>,
    pub grants: Vec<GrantSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantSummary {
    pub feed: String,
    pub role: String,
    pub publish_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feed {
    pub name: String,
    pub created_at: String,
    pub versions: u64,
    pub storage_bytes: u64,
    #[serde(default)]
    pub max_storage_bytes: Option<u64>,
    #[serde(default)]
    pub max_versions: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateFeed {
    pub name: String,
}

/// Reemplaza las cuotas del feed; `None` es sin límite.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigureFeed {
    #[serde(default)]
    pub max_storage_bytes: Option<u64>,
    #[serde(default)]
    pub max_versions: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub name: String,
    pub kind: String,
    pub administrator: bool,
    pub disabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePrincipal {
    pub name: String,
    /// `user` o `service`.
    pub kind: String,
    #[serde(default)]
    pub administrator: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub id: String,
    pub principal: String,
    #[serde(default)]
    pub name: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateToken {
    pub principal: String,
    #[serde(default)]
    pub name: Option<String>,
    pub expires_in_days: u32,
}

/// Token recién emitido: el único momento en que el secreto viaja por la API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedToken {
    pub id: String,
    pub principal: String,
    pub token: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub principal: String,
    pub feed: String,
    pub role: String,
    pub publish_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetGrant {
    /// `reader`, `publisher` o `maintainer`.
    pub role: String,
    #[serde(default)]
    pub publish_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSummary {
    pub id: String,
    pub versions: u64,
    /// Versión más alta, listada o no.
    pub latest_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageVersion {
    pub feed: String,
    pub id: String,
    pub version: String,
    pub listed: bool,
    /// `available` o `blocked`.
    pub availability: String,
    /// Solo para quien puede mantener el feed.
    #[serde(default)]
    pub blocked_reason: Option<String>,
    pub sha256: String,
    pub size: u64,
    pub published_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason {
    pub reason: String,
}

/// Resultado de un cambio de estado (unlist, relist, block, unblock).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionState {
    pub feed: String,
    pub id: String,
    pub version: String,
    pub listed: bool,
    pub availability: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub occurred_at: String,
    #[serde(default)]
    pub actor: Option<String>,
    pub action: String,
    #[serde(default)]
    pub feed: Option<String>,
    #[serde(default)]
    pub resource: Option<String>,
    /// `success`, `conflict`, `denied` o `error`.
    pub outcome: String,
    #[serde(default)]
    pub detail: Option<serde_json::Value>,
}

/// Cuerpo de todas las respuestas de error de `/api/v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDetail {
    /// Código estable (p. ej. `AUTH_SCOPE_MISSING`).
    pub code: String,
    pub message: String,
    /// Qué puede hacer quien recibe el error.
    #[serde(default)]
    pub action: Option<String>,
    /// Identificador para buscar la petición en los logs del servidor.
    #[serde(default)]
    pub request_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_and_missing_optionals_are_tolerated() {
        // Un servidor más nuevo puede añadir campos; uno más viejo puede omitir opcionales.
        let feed: Feed = serde_json::from_str(
            r#"{"name":"a","created_at":"t","versions":1,"storage_bytes":2,"future":true}"#,
        )
        .unwrap();
        assert_eq!(feed.max_versions, None);
        let error: ErrorBody =
            serde_json::from_str(r#"{"error":{"code":"X","message":"m"}}"#).unwrap();
        assert_eq!(error.error.action, None);
    }

    #[test]
    fn capabilities() {
        let caps = Capabilities {
            server_version: "0.0.0".into(),
            api_version: API_VERSION,
            capabilities: vec![capability::FEEDS.into()],
        };
        assert!(caps.supports(capability::FEEDS));
        assert!(!caps.supports(capability::AUDIT));
    }
}
