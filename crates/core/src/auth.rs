//! Identidades y autorización (ADR-010, ADR-012).
//!
//! La decisión de autorización es pura y vive aquí; la autenticación (verificar el token) la
//! hace la capa de persistencia y la extracción de credenciales, la capa HTTP.

use std::fmt;

/// Nombre de principal: `[a-z0-9._-]`, de 1 a 64 caracteres, empieza por letra o dígito.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrincipalName(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidName(pub String);

impl fmt::Display for InvalidName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid value: {:?}", self.0)
    }
}

impl std::error::Error for InvalidName {}

impl PrincipalName {
    pub fn parse(input: &str) -> Result<Self, InvalidName> {
        let valid = (1..=64).contains(&input.len())
            && input.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
            })
            && input.as_bytes()[0].is_ascii_alphanumeric();
        if valid {
            Ok(Self(input.to_owned()))
        } else {
            Err(InvalidName(input.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PrincipalName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    /// Persona.
    User,
    /// Cuenta de servicio (p. ej. un pipeline de CI).
    Service,
}

impl PrincipalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Service => "service",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "service" => Some(Self::Service),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub id: i64,
    pub name: PrincipalName,
    pub kind: PrincipalKind,
    /// Administrator: rol global sobre identidades, permisos, configuración y todos los feeds.
    pub is_admin: bool,
}

/// Roles por feed, acumulativos: Reader ⊂ Publisher ⊂ Maintainer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Reader,
    Publisher,
    Maintainer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reader => "reader",
            Self::Publisher => "publisher",
            Self::Maintainer => "maintainer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "reader" => Some(Self::Reader),
            "publisher" => Some(Self::Publisher),
            "maintainer" => Some(Self::Maintainer),
            _ => None,
        }
    }
}

/// Lo que una solicitud necesita hacer sobre un feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Buscar, consultar metadatos y descargar.
    Read,
    /// Publicar nuevas versiones (sujeto a los prefijos del grant).
    Publish,
    /// Gestionar visibilidad y bloqueos.
    Maintain,
}

impl Access {
    fn required_role(self) -> Role {
        match self {
            Self::Read => Role::Reader,
            Self::Publish => Role::Publisher,
            Self::Maintain => Role::Maintainer,
        }
    }

    /// Nombre del permiso para mensajes de error (`packages:publish`).
    pub fn scope(self) -> &'static str {
        match self {
            Self::Read => "packages:read",
            Self::Publish => "packages:publish",
            Self::Maintain => "packages:maintain",
        }
    }
}

/// Patrón de publicación: id exacto o prefijo terminado en `*` (`Hemia.Payments.*`).
/// Sin distinguir mayúsculas, como los ids NuGet. Es una política del registro, no del protocolo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishPattern(String);

impl PublishPattern {
    pub fn parse(input: &str) -> Result<Self, InvalidName> {
        let body = input.strip_suffix('*').unwrap_or(input);
        let valid = !input.is_empty()
            && input.len() <= 101
            && body
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'));
        if valid {
            Ok(Self(input.to_lowercase()))
        } else {
            Err(InvalidName(input.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `package_key` es la clave de identidad del paquete (ya en minúsculas).
    pub fn matches(&self, package_key: &str) -> bool {
        match self.0.strip_suffix('*') {
            Some(prefix) => package_key.starts_with(prefix),
            None => package_key == self.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub feed_id: i64,
    pub role: Role,
    /// Vacío: puede publicar cualquier id del feed.
    pub publish_patterns: Vec<PublishPattern>,
}

/// Identidad autenticada de una solicitud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthContext {
    pub principal: Principal,
    pub token_id: String,
    pub grants: Vec<Grant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    /// Sin acceso de lectura al feed: se responde como si no existiera (ADR-012).
    NotFound,
    /// Puede ver el feed, pero su rol no alcanza para la operación.
    MissingScope(Access),
    /// Puede publicar en el feed, pero no ese id.
    PrefixDenied,
}

impl Denial {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotFound => "FEED_NOT_FOUND",
            Self::MissingScope(_) => "AUTH_SCOPE_MISSING",
            Self::PrefixDenied => "AUTH_PREFIX_DENIED",
        }
    }
}

impl AuthContext {
    fn grant(&self, feed_id: i64) -> Option<&Grant> {
        self.grants.iter().find(|g| g.feed_id == feed_id)
    }

    /// Comprueba el acceso a un feed sin considerar el id del paquete.
    pub fn check_feed(&self, feed_id: i64, access: Access) -> Result<(), Denial> {
        if self.principal.is_admin {
            return Ok(());
        }
        match self.grant(feed_id) {
            None => Err(Denial::NotFound),
            Some(g) if g.role >= access.required_role() => Ok(()),
            Some(_) => Err(Denial::MissingScope(access)),
        }
    }

    /// Comprueba que puede publicar un id concreto en el feed.
    pub fn check_publish(&self, feed_id: i64, package_key: &str) -> Result<(), Denial> {
        self.check_feed(feed_id, Access::Publish)?;
        if self.principal.is_admin {
            return Ok(());
        }
        let patterns = &self
            .grant(feed_id)
            .expect("checked in check_feed")
            .publish_patterns;
        if patterns.is_empty() || patterns.iter().any(|p| p.matches(package_key)) {
            Ok(())
        } else {
            Err(Denial::PrefixDenied)
        }
    }

    /// Actor para auditoría.
    pub fn actor(&self) -> String {
        format!("{} (token {})", self.principal.name, self.token_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(is_admin: bool, grants: Vec<Grant>) -> AuthContext {
        AuthContext {
            principal: Principal {
                id: 1,
                name: PrincipalName::parse("ci-payments").unwrap(),
                kind: PrincipalKind::Service,
                is_admin,
            },
            token_id: "t".into(),
            grants,
        }
    }

    fn grant(feed_id: i64, role: Role, patterns: &[&str]) -> Grant {
        Grant {
            feed_id,
            role,
            publish_patterns: patterns
                .iter()
                .map(|p| PublishPattern::parse(p).unwrap())
                .collect(),
        }
    }

    #[test]
    fn roles_are_cumulative() {
        for (role, read, publish, maintain) in [
            (Role::Reader, true, false, false),
            (Role::Publisher, true, true, false),
            (Role::Maintainer, true, true, true),
        ] {
            let c = ctx(false, vec![grant(1, role, &[])]);
            assert_eq!(c.check_feed(1, Access::Read).is_ok(), read, "{role:?}");
            assert_eq!(
                c.check_feed(1, Access::Publish).is_ok(),
                publish,
                "{role:?}"
            );
            assert_eq!(
                c.check_feed(1, Access::Maintain).is_ok(),
                maintain,
                "{role:?}"
            );
        }
    }

    #[test]
    fn feeds_without_grant_look_missing() {
        let c = ctx(false, vec![grant(1, Role::Maintainer, &[])]);
        for access in [Access::Read, Access::Publish, Access::Maintain] {
            assert_eq!(c.check_feed(2, access), Err(Denial::NotFound));
        }
    }

    #[test]
    fn insufficient_role_reports_missing_scope() {
        let c = ctx(false, vec![grant(1, Role::Reader, &[])]);
        assert_eq!(
            c.check_feed(1, Access::Publish),
            Err(Denial::MissingScope(Access::Publish))
        );
        assert_eq!(
            c.check_publish(1, "a"),
            Err(Denial::MissingScope(Access::Publish))
        );
    }

    #[test]
    fn publish_patterns_restrict_ids() {
        let c = ctx(
            false,
            vec![grant(
                1,
                Role::Publisher,
                &["Hemia.Payments.*", "Hemia.Shared"],
            )],
        );
        assert_eq!(c.check_publish(1, "hemia.payments.core"), Ok(()));
        assert_eq!(c.check_publish(1, "hemia.shared"), Ok(()));
        assert_eq!(
            c.check_publish(1, "hemia.shared.extra"),
            Err(Denial::PrefixDenied)
        );
        assert_eq!(
            c.check_publish(1, "hemia.logging"),
            Err(Denial::PrefixDenied)
        );
        // El grant restringe la publicación, no la lectura.
        assert_eq!(c.check_feed(1, Access::Read), Ok(()));
    }

    #[test]
    fn admin_can_do_everything() {
        let c = ctx(true, vec![]);
        assert_eq!(c.check_feed(42, Access::Maintain), Ok(()));
        assert_eq!(c.check_publish(42, "anything"), Ok(()));
    }

    #[test]
    fn names_and_patterns_validate() {
        assert!(PrincipalName::parse("ci-payments").is_ok());
        assert!(PrincipalName::parse("cristian.mendez").is_ok());
        for bad in ["", "-a", "A", "a b", "a/b", &"a".repeat(65)] {
            assert!(PrincipalName::parse(bad).is_err(), "{bad}");
        }
        assert!(PublishPattern::parse("Hemia.*").is_ok());
        assert!(PublishPattern::parse("*").is_ok());
        for bad in ["", "a*b", "a/b*", "**"] {
            assert!(PublishPattern::parse(bad).is_err(), "{bad}");
        }
    }
}
