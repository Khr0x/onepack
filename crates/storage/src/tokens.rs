//! Formato y verificación de tokens opacos (ADR-010).
//!
//! `opk_<id>_<secreto>`: `id` son 16 caracteres hex públicos (para buscar y auditar) y el
//! secreto, 64 caracteres hex (256 bits). Solo se guarda `sha256(secreto)`; con esa entropía
//! no hace falta un hash lento.

use sha2::{Digest, Sha256};

pub const TOKEN_PREFIX: &str = "opk_";
const ID_LEN: usize = 16;
const SECRET_LEN: usize = 64;

pub struct GeneratedToken {
    pub id: String,
    /// Token completo para entregar al usuario una sola vez.
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

/// Separa un token en `(id, secreto)` si tiene el formato correcto.
pub fn parse(token: &str) -> Option<(&str, &str)> {
    let rest = token.strip_prefix(TOKEN_PREFIX)?;
    let (id, secret) = rest.split_once('_')?;
    (id.len() == ID_LEN && secret.len() == SECRET_LEN && is_hex(id) && is_hex(secret))
        .then_some((id, secret))
}

pub fn verifier(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

/// Comparación en tiempo constante para cadenas de igual longitud.
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
