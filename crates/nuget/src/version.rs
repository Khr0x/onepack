//! NuGet versions (ADR-008).
//!
//! Reproduces `NuGetVersion.TryParse`, `ToNormalizedString`, and `VersionComparer.Default` from
//! `NuGet.Versioning`. The behavior is validated against the corpus generated in
//! `tests/conformance-dotnet/version-corpus`.

use std::cmp::Ordering;
use std::fmt;

/// NuGet version: `Major.Minor[.Patch[.Revision]][-Release][+Metadata]`.
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
        write!(f, "invalid NuGet version: {:?}", self.0)
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

    /// SemVer 2.0.0: multiple prerelease labels or build metadata.
    pub fn is_semver2(&self) -> bool {
        self.release.len() > 1 || self.metadata.is_some()
    }

    /// NuGet normalized form: no metadata, revision only if it is not zero,
    /// labels with their original capitalization.
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

    /// Normalized form with build metadata.
    pub fn full(&self) -> String {
        match &self.metadata {
            Some(meta) => format!("{}+{meta}", self.normalized()),
            None => self.normalized(),
        }
    }

    /// Identity key within a feed (ADR-007): normalized and lowercase.
    /// Two versions are the same identity if and only if their keys match.
    pub fn identity(&self) -> String {
        self.normalized().to_ascii_lowercase()
    }

    /// Precedence order of `VersionComparer.Default` (ignores metadata).
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

/// Numeric component: allows surrounding spaces and leading zeros, maximum `i32::MAX`.
fn parse_component(part: &str) -> Option<u32> {
    let digits = part.trim();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|&n| n <= i32::MAX as u32)
}

/// Prerelease label or metadata: `[0-9A-Za-z-]+`. In prerelease, a label
/// of digits only cannot have leading zeros.
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

/// Labels that `Int32.TryParse` accepts are compared as numbers and come before
/// alphanumeric ones, which are compared ordinally without case distinction.
fn cmp_label(a: &str, b: &str) -> Ordering {
    match (as_int32(a), as_int32(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.to_ascii_uppercase().cmp(&b.to_ascii_uppercase()),
    }
}

/// Equivalent to `Int32.TryParse` for already validated labels (no spaces or `+`).
/// It accepts a leading `-`: `"-1"` is numeric for NuGet.
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
