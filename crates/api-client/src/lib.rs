//! Contracts of the administrative API `/api/v1`, shared by `onepackd` and `onepack`
//! (ADR-015).
//!
//! Compatibility: new fields are added as optional (`#[serde(default)]`) and nobody
//! rejects unknown fields, so a CLI N talks to an N-1 server and vice versa. What
//! a server can do is announced in [`Capabilities`].

use serde::{Deserialize, Serialize};

#[cfg(feature = "client")]
pub mod client;

/// Contract version. It changes only with breaking changes.
pub const API_VERSION: u32 = 1;

/// Capabilities a server announces in `GET /api/v1/capabilities`.
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

    /// All the ones implemented by this version.
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

/// Default and maximum page size for cursor-based listings.
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

/// A listing page. `next_cursor` is opaque; `None` indicates the last page.
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

/// Replaces the feed quotas; `None` means no limit.
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

/// Newly issued token: the only time the secret travels over the API.
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
    /// Highest version, listed or not.
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
    /// Only for those who can maintain the feed.
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

/// Result of a state change (unlist, relist, block, unblock).
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

/// Body of every error response from `/api/v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDetail {
    /// Stable code (e.g. `AUTH_SCOPE_MISSING`).
    pub code: String,
    pub message: String,
    /// What the recipient of the error can do.
    #[serde(default)]
    pub action: Option<String>,
    /// Identifier to find the request in the server logs.
    #[serde(default)]
    pub request_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_and_missing_optionals_are_tolerated() {
        // A newer server may add fields; an older one may omit optional ones.
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
