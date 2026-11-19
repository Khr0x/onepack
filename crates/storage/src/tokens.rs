//! Format and verification of opaque tokens (ADR-010).
//!
//! `opk_<id>_<secret>`: `id` is 16 public hex characters (for lookup and auditing) and the
//! secret, 64 hex characters (256 bits). Only `sha256(secret)` is stored; with that entropy
//! a slow hash is not needed.

use sha2::{Digest, Sha256};

pub const TOKEN_PREFIX: &str = "opk_";
const ID_LEN: usize = 16;
const SECRET_LEN: usize = 64;

pub struct GeneratedToken {
    pub id: String,
    /// Full token to hand to the user only once.
    pub token: String,
    pub verifier: String,
}

pub fn generate() -> Result<GeneratedToken, getrandom::Error> {
    let mut id = [0u8; ID_LEN / 2];
    let mut secret = [0u8; SECRET_LEN / 2];
    getrandom::fill(&mut id)?;
    getrandom::fill(&mut secret)?;
    let id = hex(&id);
    let secret = hex(&secret);
    Ok(GeneratedToken {
        token: format!("{TOKEN_PREFIX}{id}_{secret}"),
        verifier: verifier(&secret),
        id,
    })
}

/// Splits a token into `(id, secret)` if it has the correct format.
pub fn parse(token: &str) -> Option<(&str, &str)> {
    let rest = token.strip_prefix(TOKEN_PREFIX)?;
    let (id, secret) = rest.split_once('_')?;
    (id.len() == ID_LEN && secret.len() == SECRET_LEN && is_hex(id) && is_hex(secret))
        .then_some((id, secret))
}

pub fn verifier(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

/// Constant-time comparison for strings of equal length.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_hex(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_parse_and_verify() {
        let t = generate().unwrap();
        assert!(t.token.starts_with("opk_"));
        assert_eq!(t.token.len(), 4 + 16 + 1 + 64);
        let (id, secret) = parse(&t.token).unwrap();
        assert_eq!(id, t.id);
        assert!(constant_time_eq(&verifier(secret), &t.verifier));
        assert!(!t.verifier.contains(secret));
    }

    #[test]
    fn tokens_are_unique() {
        let a = generate().unwrap();
        let b = generate().unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(a.token, b.token);
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        let good = generate().unwrap().token;
        for bad in [
            "",
            "opk_",
            "pkg_0123456789abcdef_".to_owned().as_str(),
            &good[..good.len() - 1],
            &good.to_uppercase(),
            &good.replacen("opk_", "", 1),
        ] {
            assert!(parse(bad).is_none(), "{bad}");
        }
    }
}
