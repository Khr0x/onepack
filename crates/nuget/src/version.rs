//! Versiones NuGet (ADR-008).
//!
//! Reproduce `NuGetVersion.TryParse`, `ToNormalizedString` y `VersionComparer.Default` de
//! `NuGet.Versioning`. El comportamiento se valida contra el corpus generado en
//! `tests/conformance-dotnet/version-corpus`.

use std::cmp::Ordering;
use std::fmt;

/// Versión NuGet: `Major.Minor[.Patch[.Revision]][-Release][+Metadata]`.
#[derive(Debug, Clone)]
pub struct NuGetVersion {
    numbers: [u32; 4],
    release: Vec<String>,
    metadata: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidVersion(pub String);

impl fmt::Display for InvalidVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "versión NuGet inválida: {:?}", self.0)
    }
}

impl std::error::Error for InvalidVersion {}

impl NuGetVersion {
    pub fn parse(input: &str) -> Result<Self, InvalidVersion> {
        Self::try_parse(input).ok_or_else(|| InvalidVersion(input.to_owned()))
    }

    fn try_parse(input: &str) -> Option<Self> {
        let s = input.trim();

        let (head, metadata) = match s.split_once('+') {
            Some((head, meta)) => (head, Some(meta)),
            None => (s, None),
        };
        let (numeric, release) = match head.split_once('-') {
            Some((num, rel)) => (num, Some(rel)),
            None => (head, None),
        };

        let parts: Vec<&str> = numeric.split('.').collect();
        if parts.len() > 4 {
            return None;
        }
        let mut numbers = [0u32; 4];
        for (slot, part) in numbers.iter_mut().zip(&parts) {
            *slot = parse_component(part)?;
        }

        let release = match release {
            Some(r) => {
                let labels: Vec<&str> = r.split('.').collect();
                if !labels.iter().all(|l| is_valid_label(l, false)) {
                    return None;
                }
                labels.into_iter().map(str::to_owned).collect()
            }
            None => Vec::new(),
        };

        if let Some(meta) = metadata
            && !meta.split('.').all(|l| is_valid_label(l, true))
        {
            return None;
        }

        Some(Self {
            numbers,
            release,
            metadata: metadata.map(str::to_owned),
        })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.release.is_empty()
    }

    /// SemVer 2.0.0: varias etiquetas de prerelease o metadatos de build.
    pub fn is_semver2(&self) -> bool {
        self.release.len() > 1 || self.metadata.is_some()
    }

    /// Forma normalizada de NuGet: sin metadatos, revisión solo si no es cero,
    /// etiquetas con su capitalización original.
    pub fn normalized(&self) -> String {
        let [major, minor, patch, revision] = self.numbers;
        let mut out = format!("{major}.{minor}.{patch}");
        if revision != 0 {
            out.push_str(&format!(".{revision}"));
        }
        if self.is_prerelease() {
            out.push('-');
            out.push_str(&self.release.join("."));
        }
        out
    }

    /// Forma normalizada con metadatos de build.
    pub fn full(&self) -> String {
        match &self.metadata {
            Some(meta) => format!("{}+{meta}", self.normalized()),
            None => self.normalized(),
        }
    }

    /// Clave de identidad dentro de un feed (ADR-007): normalizada y en minúsculas.
    /// Dos versiones son la misma identidad si y solo si sus claves coinciden.
    pub fn identity(&self) -> String {
        self.normalized().to_ascii_lowercase()
    }

    /// Orden de precedencia de `VersionComparer.Default` (ignora metadatos).
    pub fn precedence_cmp(&self, other: &Self) -> Ordering {
        self.numbers.cmp(&other.numbers).then_with(|| {
            match (self.release.is_empty(), other.release.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => cmp_release(&self.release, &other.release),
            }
        })
    }
}

impl fmt::Display for NuGetVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.normalized())
    }
}

/// Componente numérico: admite espacios alrededor y ceros a la izquierda, máximo `i32::MAX`.
fn parse_component(part: &str) -> Option<u32> {
    let digits = part.trim();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|&n| n <= i32::MAX as u32)
}

/// Etiqueta de prerelease o metadatos: `[0-9A-Za-z-]+`. En prerelease, una etiqueta
/// solo de dígitos no puede tener ceros a la izquierda.
fn is_valid_label(label: &str, allow_leading_zeros: bool) -> bool {
    if label.is_empty()
        || !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return false;
    }
    let all_digits = label.bytes().all(|b| b.is_ascii_digit());
    allow_leading_zeros || !all_digits || label == "0" || !label.starts_with('0')
}

fn cmp_release(a: &[String], b: &[String]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let ord = cmp_label(x, y);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.len().cmp(&b.len())
}

/// Las etiquetas que `Int32.TryParse` acepta se comparan como números y van antes que las
/// alfanuméricas, que se comparan ordinalmente sin distinguir mayúsculas.
fn cmp_label(a: &str, b: &str) -> Ordering {
    match (as_int32(a), as_int32(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.to_ascii_uppercase().cmp(&b.to_ascii_uppercase()),
    }
}

/// Equivalente a `Int32.TryParse` para etiquetas ya validadas (sin espacios ni `+`).
/// Acepta un `-` inicial: `"-1"` es numérico para NuGet.
fn as_int32(label: &str) -> Option<i32> {
    let digits = label.strip_prefix('-').unwrap_or(label);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    label.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_forms_share_identity() {
        let ids: Vec<String> = ["1", "1.0", "1.0.0", "1.0.0.0", "01.00", "1.0.0+meta"]
            .iter()
            .map(|v| NuGetVersion::parse(v).unwrap().identity())
            .collect();
        assert!(ids.iter().all(|id| id == "1.0.0"));
    }

    #[test]
    fn identity_ignores_label_case_but_normalized_keeps_it() {
        let v = NuGetVersion::parse("1.0.0-Beta").unwrap();
        assert_eq!(v.normalized(), "1.0.0-Beta");
        assert_eq!(v.identity(), "1.0.0-beta");
    }
}
