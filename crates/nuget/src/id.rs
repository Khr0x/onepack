//! Identificadores de paquete NuGet.

use std::fmt;

/// Longitud máxima de `PackageIdValidator`.
pub const MAX_ID_LEN: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageId(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPackageId(pub String);

impl fmt::Display for InvalidPackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid package id: {:?}", self.0)
    }
}

impl std::error::Error for InvalidPackageId {}

impl PackageId {
    /// Valida contra `^\w+([.-]\w+)*$` con un máximo de 100 caracteres, como `PackageIdValidator`.
    pub fn parse(input: &str) -> Result<Self, InvalidPackageId> {
        let valid = input.chars().count() <= MAX_ID_LEN
            && input
                .split(['.', '-'])
                .all(|part| !part.is_empty() && part.chars().all(is_word_char));
        if valid {
            Ok(Self(input.to_owned()))
        } else {
            Err(InvalidPackageId(input.to_owned()))
        }
    }

    /// Id tal como lo escribió el autor.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Clave de identidad: los ids NuGet no distinguen mayúsculas.
    pub fn identity(&self) -> String {
        self.0.to_lowercase()
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Aproximación de `\w` de .NET (letras, dígitos y `_`).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_ids() {
        for id in ["Newtonsoft.Json", "Hemia.Payments-Core", "a", "My_Lib.2"] {
            assert!(PackageId::parse(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn rejects_invalid_ids() {
        let long = "a".repeat(MAX_ID_LEN + 1);
        for id in [
            "",
            ".a",
            "a.",
            "a..b",
            "a b",
            "a/b",
            "../a",
            "a+b",
            long.as_str(),
        ] {
            assert!(PackageId::parse(id).is_err(), "{id}");
        }
    }

    #[test]
    fn identity_is_case_insensitive() {
        let a = PackageId::parse("Hemia.Logging").unwrap();
        let b = PackageId::parse("hemia.logging").unwrap();
        assert_eq!(a.identity(), b.identity());
        assert_eq!(a.as_str(), "Hemia.Logging");
    }
}
