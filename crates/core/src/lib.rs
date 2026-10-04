//! Dominio: qué significa publicar una versión, quién puede hacerlo y cuándo está disponible.
//! No depende de NuGet, SQLx ni Axum (ADR-003).

pub mod auth;

use std::fmt;

pub use auth::{
    Access, AuthContext, Denial, Grant, Principal, PrincipalKind, PrincipalName, PublishPattern,
    Role,
};

/// Nombre de feed: `[a-z0-9-]`, de 1 a 64 caracteres, empieza por letra o dígito.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedName(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidFeedName(pub String);

impl fmt::Display for InvalidFeedName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "nombre de feed inválido: {:?} (usa a-z, 0-9 y '-', máximo 64, empezando por letra o dígito)",
            self.0
        )
    }
}

impl std::error::Error for InvalidFeedName {}

impl FeedName {
    pub const MAX_LEN: usize = 64;

    pub fn parse(input: &str) -> Result<Self, InvalidFeedName> {
        let valid = (1..=Self::MAX_LEN).contains(&input.len())
            && input
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !input.starts_with('-');
        if valid {
            Ok(Self(input.to_owned()))
        } else {
            Err(InvalidFeedName(input.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FeedName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Feed existente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    pub id: i64,
    pub name: FeedName,
}

/// Versión a publicar, ya validada por el adaptador del formato.
///
/// La identidad dentro del feed es `package_key + version_key` (ADR-007); el adaptador decide
/// cómo se normalizan, el dominio solo exige que no se repitan.
#[derive(Debug, Clone)]
pub struct NewVersion {
    /// Id tal como lo escribió el autor.
    pub package_id: String,
    pub package_key: String,
    /// Versión normalizada, con la capitalización original.
    pub version: String,
    pub version_key: String,
    /// Versión completa, con metadatos de build si los hay.
    pub full_version: String,
    pub is_prerelease: bool,
    pub is_semver2: bool,
    /// Documento JSON de metadatos generado por el adaptador; el dominio no lo interpreta.
    pub metadata: String,
    /// Texto en minúsculas sobre el que se busca.
    pub search_text: String,
}

impl NewVersion {
    /// Recurso legible para auditoría y mensajes: `Id@versión`.
    pub fn resource(&self) -> String {
        format!("{}@{}", self.package_id, self.version)
    }
}

/// Versión publicada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedVersion {
    pub package_id: String,
    pub package_key: String,
    pub version: String,
    pub version_key: String,
    pub full_version: String,
    pub is_prerelease: bool,
    pub is_semver2: bool,
    pub blob_sha256: String,
    pub size: u64,
    pub listed: bool,
    /// `availability: blocked` (ADR-013): sigue en los metadatos, pero no se descarga.
    pub blocked: bool,
    /// RFC 3339 (UTC).
    pub published_at: String,
    pub metadata: Option<String>,
    pub search_text: Option<String>,
}

/// Cuotas de un feed (ADR-014). `None` es sin límite.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeedQuota {
    pub max_storage_bytes: Option<u64>,
    pub max_versions: Option<u64>,
}

/// Uso de un feed: suma lógica de los tamaños de sus versiones.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeedUsage {
    pub versions: u64,
    pub storage_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaKind {
    Versions,
    Storage,
}

impl QuotaKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Versions => "FEED_QUOTA_VERSIONS",
            Self::Storage => "FEED_QUOTA_STORAGE",
        }
    }
}

impl FeedQuota {
    /// Cuota que se superaría al añadir una versión de `size` bytes.
    pub fn exceeded_by(&self, usage: FeedUsage, size: u64) -> Option<QuotaKind> {
        if self
            .max_versions
            .is_some_and(|max| usage.versions + 1 > max)
        {
            Some(QuotaKind::Versions)
        } else if self
            .max_storage_bytes
            .is_some_and(|max| usage.storage_bytes.saturating_add(size) > max)
        {
            Some(QuotaKind::Storage)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub enum PublishError {
    /// La identidad ya existe. `identical` indica si el contenido coincide byte a byte, lo que
    /// permite a un cliente tratar el duplicado como éxito de forma explícita (ADR-007).
    Conflict {
        identical: bool,
    },
    /// Publicarla superaría una cuota del feed; no se persiste nada.
    QuotaExceeded(QuotaKind),
    StorageFull,
    Io(std::io::Error),
    Database(String),
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict { identical: true } => {
                f.write_str("la versión ya existe con el mismo contenido")
            }
            Self::Conflict { identical: false } => {
                f.write_str("la versión ya existe con contenido distinto")
            }
            Self::QuotaExceeded(kind) => write!(f, "{}: cuota del feed superada", kind.code()),
            Self::StorageFull => f.write_str("no queda espacio en el almacenamiento"),
            Self::Io(e) => write!(f, "error de E/S: {e}"),
            Self::Database(e) => write!(f, "error de base de datos: {e}"),
        }
    }
}

impl std::error::Error for PublishError {}

impl From<std::io::Error> for PublishError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::StorageFull {
            Self::StorageFull
        } else {
            Self::Io(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotas() {
        let usage = FeedUsage {
            versions: 2,
            storage_bytes: 100,
        };
        assert_eq!(FeedQuota::default().exceeded_by(usage, u64::MAX), None);
        let versions = FeedQuota {
            max_versions: Some(3),
            ..FeedQuota::default()
        };
        assert_eq!(versions.exceeded_by(usage, 1), None);
        let full = FeedUsage {
            versions: 3,
            ..usage
        };
        assert_eq!(versions.exceeded_by(full, 1), Some(QuotaKind::Versions));
        let storage = FeedQuota {
            max_storage_bytes: Some(150),
            ..FeedQuota::default()
        };
        assert_eq!(storage.exceeded_by(usage, 50), None);
        assert_eq!(storage.exceeded_by(usage, 51), Some(QuotaKind::Storage));
    }

    #[test]
    fn feed_names() {
        for ok in ["internal", "customer-a", "a", "x1", &"a".repeat(64)] {
            assert!(FeedName::parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "-a",
            "Internal",
            "a_b",
            "a b",
            "a/b",
            "..",
            &"a".repeat(65),
        ] {
            assert!(FeedName::parse(bad).is_err(), "{bad}");
        }
    }
}
