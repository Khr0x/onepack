//! Principals, tokens y grants (ADR-010, ADR-012). Toda mutación queda auditada.

use onepack_core::{
    AuthContext, Feed, Grant, Principal, PrincipalKind, PrincipalName, PublishPattern, Role,
};
use sqlx::{Row, SqliteConnection};

use crate::tokens::{self, constant_time_eq};
use crate::{Store, StoreError};

#[derive(Debug)]
pub enum AuthOutcome {
    Valid(AuthContext),
    Invalid(AuthFailure),
}

/// Motivo del rechazo. Nunca contiene el secreto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthFailure {
    Malformed,
    UnknownToken { id: String },
    WrongSecret { id: String },
    Expired { id: String },
    Revoked { id: String },
    PrincipalDisabled { id: String },
}

impl AuthFailure {
    pub fn token_id(&self) -> Option<&str> {
        match self {
            Self::Malformed => None,
            Self::UnknownToken { id }
            | Self::WrongSecret { id }
            | Self::Expired { id }
            | Self::Revoked { id }
            | Self::PrincipalDisabled { id } => Some(id),
        }
    }

    pub fn reason(&self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::UnknownToken { .. } => "unknown_token",
            Self::WrongSecret { .. } => "wrong_secret",
            Self::Expired { .. } => "expired",
            Self::Revoked { .. } => "revoked",
            Self::PrincipalDisabled { .. } => "principal_disabled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct IssuedToken {
    pub id: String,
    /// Token completo: se muestra una sola vez y no se puede recuperar después.
    pub token: String,
    pub expires_at: String,
}

pub(crate) async fn audit(
    conn: &mut SqliteConnection,
    action: &str,
    actor: Option<&str>,
    feed_id: Option<i64>,
    resource: Option<&str>,
    outcome: &str,
    detail: Option<String>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_event (action, actor, feed_id, resource, outcome, detail)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(action)
    .bind(actor)
    .bind(feed_id)
    .bind(resource)
    .bind(outcome)
    .bind(detail)
    .execute(conn)
    .await?;
    Ok(())
}

fn to_principal(r: &sqlx::sqlite::SqliteRow) -> Option<Principal> {
    Some(Principal {
        id: r.get("id"),
        name: PrincipalName::parse(r.get::<&str, _>("name")).ok()?,
        kind: PrincipalKind::parse(r.get::<&str, _>("kind"))?,
        is_admin: r.get("is_admin"),
    })
}

impl Store {
    /// Verifica un token y devuelve la identidad con sus grants. Consulta la base en cada
    /// llamada, así que una revocación surte efecto inmediato.
    pub async fn authenticate(&self, raw: &str) -> Result<AuthOutcome, StoreError> {
        let Some((id, secret)) = tokens::parse(raw) else {
            return Ok(AuthOutcome::Invalid(AuthFailure::Malformed));
        };
        let id = id.to_owned();
        let row = sqlx::query(
            "SELECT t.verifier, t.revoked_at IS NOT NULL AS revoked,
                    t.expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now') AS expired,
                    p.id, p.name, p.kind, p.is_admin, p.disabled_at IS NOT NULL AS disabled
             FROM token t JOIN principal p ON p.id = t.principal_id
             WHERE t.id = ?",
        )
        .bind(&id)
        .fetch_optional(&self.reader)
        .await?;

        let Some(row) = row else {
            return Ok(AuthOutcome::Invalid(AuthFailure::UnknownToken { id }));
        };
        // Primero el secreto: sin él no se revela el estado del token.
        if !constant_time_eq(&tokens::verifier(secret), row.get("verifier")) {
            return Ok(AuthOutcome::Invalid(AuthFailure::WrongSecret { id }));
        }
        if row.get::<bool, _>("revoked") {
            return Ok(AuthOutcome::Invalid(AuthFailure::Revoked { id }));
        }
        if row.get::<bool, _>("expired") {
            return Ok(AuthOutcome::Invalid(AuthFailure::Expired { id }));
        }
        if row.get::<bool, _>("disabled") {
            return Ok(AuthOutcome::Invalid(AuthFailure::PrincipalDisabled { id }));
        }
        let principal = to_principal(&row).ok_or_else(|| {
            StoreError::Database(sqlx::Error::Protocol(
                "invalid principal in the database".into(),
            ))
        })?;
        let grants = self.grants(&principal).await?;
        Ok(AuthOutcome::Valid(AuthContext {
            principal,
            token_id: id,
            grants,
        }))
    }

    pub async fn record_auth_failure(&self, failure: &AuthFailure) {
        let result = async {
            let mut conn = self.writer.acquire().await?;
            audit(
                &mut conn,
                "auth.failure",
                failure.token_id(),
                None,
                None,
                "denied",
                Some(format!(r#"{{"reason":"{}"}}"#, failure.reason())),
            )
            .await
        }
        .await;
        if let Err(e) = result {
            tracing::error!(error = %e, "could not audit the authentication failure");
        }
    }

    /// Registra el último uso del token. Quien llama decide cada cuánto (ADR-010).
    pub async fn touch_token(&self, id: &str) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE token SET last_used_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(id)
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    pub async fn create_principal(
        &self,
        name: &PrincipalName,
        kind: PrincipalKind,
        is_admin: bool,
        actor: &str,
    ) -> Result<Principal, StoreError> {
        let mut tx = self.writer.begin().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO principal (name, kind, is_admin) VALUES (?, ?, ?) RETURNING id",
        )
        .bind(name.as_str())
        .bind(kind.as_str())
        .bind(is_admin)
        .fetch_one(&mut *tx)
        .await?;
        let detail = format!(r#"{{"kind":"{}","admin":{is_admin}}}"#, kind.as_str());
        audit(
            &mut tx,
            "principal.create",
            Some(actor),
            None,
            Some(name.as_str()),
            "success",
            Some(detail),
        )
        .await?;
        tx.commit().await?;
        Ok(Principal {
            id,
            name: name.clone(),
            kind,
            is_admin,
        })
    }

    pub async fn principal(&self, name: &str) -> Result<Option<Principal>, StoreError> {
        let row = sqlx::query("SELECT id, name, kind, is_admin FROM principal WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.reader)
            .await?;
        Ok(row.as_ref().and_then(to_principal))
    }

    pub async fn has_admin(&self) -> Result<bool, StoreError> {
        Ok(
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM principal WHERE is_admin = 1)")
                .fetch_one(&self.reader)
                .await?,
        )
    }

    /// Emite un token con caducidad obligatoria. `ttl_secs` puede ser negativo solo en pruebas.
    pub async fn create_token(
        &self,
        principal: &Principal,
        name: Option<&str>,
        ttl_secs: i64,
        actor: &str,
    ) -> Result<IssuedToken, StoreError> {
        let generated = tokens::generate().map_err(|e| StoreError::Io(std::io::Error::other(e)))?;
        let mut tx = self.writer.begin().await?;
        let expires_at: String = sqlx::query_scalar(
            "INSERT INTO token (id, principal_id, name, verifier, expires_at)
             VALUES (?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?))
             RETURNING expires_at",
        )
        .bind(&generated.id)
        .bind(principal.id)
        .bind(name)
        .bind(&generated.verifier)
        .bind(format!("{ttl_secs:+} seconds"))
        .fetch_one(&mut *tx)
        .await?;
        let detail = format!(
            r#"{{"token":"{}","expires_at":"{expires_at}"}}"#,
            generated.id
        );
        audit(
            &mut tx,
            "token.create",
            Some(actor),
            None,
            Some(principal.name.as_str()),
            "success",
            Some(detail),
        )
        .await?;
        tx.commit().await?;
        Ok(IssuedToken {
            id: generated.id,
            token: generated.token,
            expires_at,
        })
    }

    /// Revoca un token. Devuelve `false` si no existía o ya estaba revocado.
    pub async fn revoke_token(&self, id: &str, actor: &str) -> Result<bool, StoreError> {
        let mut tx = self.writer.begin().await?;
        let revoked = sqlx::query(
            "UPDATE token SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if revoked {
            audit(
                &mut tx,
                "token.revoke",
                Some(actor),
                None,
                Some(id),
                "success",
                None,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(revoked)
    }

    /// Crea o reemplaza el grant de un principal sobre un feed.
    pub async fn set_grant(
        &self,
        principal: &Principal,
        feed: &Feed,
        role: Role,
        patterns: &[PublishPattern],
        actor: &str,
    ) -> Result<(), StoreError> {
        let mut tx = self.writer.begin().await?;
        sqlx::query(
            "INSERT INTO feed_grant (principal_id, feed_id, role) VALUES (?, ?, ?)
             ON CONFLICT (principal_id, feed_id) DO UPDATE SET role = excluded.role",
        )
        .bind(principal.id)
        .bind(feed.id)
        .bind(role.as_str())
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM grant_publish_pattern WHERE principal_id = ? AND feed_id = ?")
            .bind(principal.id)
            .bind(feed.id)
            .execute(&mut *tx)
            .await?;
        for p in patterns {
            sqlx::query(
                "INSERT INTO grant_publish_pattern (principal_id, feed_id, pattern) VALUES (?, ?, ?)
                 ON CONFLICT DO NOTHING",
            )
            .bind(principal.id)
            .bind(feed.id)
            .bind(p.as_str())
            .execute(&mut *tx)
            .await?;
        }
        let patterns_json: Vec<String> = patterns
            .iter()
            .map(|p| format!("{:?}", p.as_str()))
            .collect();
        let detail = format!(
            r#"{{"role":"{}","patterns":[{}]}}"#,
            role.as_str(),
            patterns_json.join(",")
        );
        audit(
            &mut tx,
            "grant.set",
            Some(actor),
            Some(feed.id),
            Some(principal.name.as_str()),
            "success",
            Some(detail),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn remove_grant(
        &self,
        principal: &Principal,
        feed: &Feed,
        actor: &str,
    ) -> Result<bool, StoreError> {
        let mut tx = self.writer.begin().await?;
        // Los patrones se borran en cascada.
        let removed = sqlx::query("DELETE FROM feed_grant WHERE principal_id = ? AND feed_id = ?")
            .bind(principal.id)
            .bind(feed.id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
        if removed {
            audit(
                &mut tx,
                "grant.remove",
                Some(actor),
                Some(feed.id),
                Some(principal.name.as_str()),
                "success",
                None,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(removed)
    }

    pub async fn grants(&self, principal: &Principal) -> Result<Vec<Grant>, StoreError> {
        let rows = sqlx::query(
            "SELECT g.feed_id, g.role, p.pattern
             FROM feed_grant g
             LEFT JOIN grant_publish_pattern p ON p.principal_id = g.principal_id AND p.feed_id = g.feed_id
             WHERE g.principal_id = ?
             ORDER BY g.feed_id, p.pattern",
        )
        .bind(principal.id)
        .fetch_all(&self.reader)
        .await?;

        let mut grants: Vec<Grant> = Vec::new();
        for r in &rows {
            let feed_id: i64 = r.get("feed_id");
            let Some(role) = Role::parse(r.get::<&str, _>("role")) else {
                continue;
            };
            if grants.last().is_none_or(|g| g.feed_id != feed_id) {
                grants.push(Grant {
                    feed_id,
                    role,
                    publish_patterns: Vec::new(),
                });
            }
            if let Some(p) = r
                .get::<Option<&str>, _>("pattern")
                .and_then(|p| PublishPattern::parse(p).ok())
            {
                grants
                    .last_mut()
                    .expect("insertado arriba")
                    .publish_patterns
                    .push(p);
            }
        }
        Ok(grants)
    }
}
