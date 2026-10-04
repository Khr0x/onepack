//! Consultas de administración (Fase 6): listados, detalle de feeds y desactivación de
//! principals. Los listados grandes (paquetes, auditoría) paginan por clave en SQL.

use onepack_core::{Feed, FeedQuota, FeedUsage, Principal};
use sqlx::Row;

use crate::identity::audit;
use crate::{Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedDetails {
    pub created_at: String,
    pub usage: FeedUsage,
    pub quota: FeedQuota,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalRow {
    pub name: String,
    pub kind: String,
    pub is_admin: bool,
    pub disabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisableOutcome {
    Disabled,
    AlreadyDisabled,
    /// Es el último administrador activo: desactivarlo dejaría el registro sin gestión.
    LastAdmin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRow {
    pub id: String,
    pub principal: String,
    pub name: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRow {
    pub principal: String,
    pub feed: String,
    pub role: String,
    pub publish_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRow {
    pub package_key: String,
    pub package_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub id: i64,
    pub occurred_at: String,
    pub actor: Option<String>,
    pub action: String,
    pub feed: Option<String>,
    pub resource: Option<String>,
    pub outcome: String,
    pub detail: Option<String>,
}

/// Filtros del listado de auditoría.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// Eventos con id menor (cursor; el listado va del más reciente al más antiguo).
    pub before_id: Option<i64>,
    pub limit: u32,
    pub feed_id: Option<i64>,
    /// Prefijo de la acción (p. ej. `package.` o `token.create`).
    pub action_prefix: Option<String>,
}

impl Store {
    pub async fn feed_details(&self, feed: &Feed) -> Result<FeedDetails, StoreError> {
        let created_at: String = sqlx::query_scalar("SELECT created_at FROM feed WHERE id = ?")
            .bind(feed.id)
            .fetch_one(&self.reader)
            .await?;
        Ok(FeedDetails {
            created_at,
            usage: self.feed_usage(feed).await?,
            quota: self.feed_quota(feed).await?,
        })
    }

    pub async fn list_principals(&self) -> Result<Vec<PrincipalRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT name, kind, is_admin, disabled_at IS NOT NULL AS disabled, created_at
             FROM principal ORDER BY name",
        )
        .fetch_all(&self.reader)
        .await?;
        Ok(rows
            .iter()
            .map(|r| PrincipalRow {
                name: r.get("name"),
                kind: r.get("kind"),
                is_admin: r.get("is_admin"),
                disabled: r.get("disabled"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Desactiva un principal: sus tokens dejan de autenticar de inmediato. No se puede
    /// desactivar al último administrador activo.
    pub async fn disable_principal(
        &self,
        principal: &Principal,
        actor: &str,
    ) -> Result<DisableOutcome, StoreError> {
        let mut tx = self.writer.begin().await?;
        let disabled: bool =
            sqlx::query_scalar("SELECT disabled_at IS NOT NULL FROM principal WHERE id = ?")
                .bind(principal.id)
                .fetch_one(&mut *tx)
                .await?;
        if disabled {
            return Ok(DisableOutcome::AlreadyDisabled);
        }
        if principal.is_admin {
            let active_admins: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM principal WHERE is_admin = 1 AND disabled_at IS NULL",
            )
            .fetch_one(&mut *tx)
            .await?;
            if active_admins <= 1 {
                return Ok(DisableOutcome::LastAdmin);
            }
        }
        sqlx::query(
            "UPDATE principal SET disabled_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(principal.id)
        .execute(&mut *tx)
        .await?;
        audit(
            &mut tx,
            "principal.disable",
            Some(actor),
            None,
            Some(principal.name.as_str()),
            "success",
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(DisableOutcome::Disabled)
    }

    /// Tokens (activos o no), de un principal o de todos, del más reciente al más antiguo.
    pub async fn list_tokens(
        &self,
        principal: Option<&Principal>,
    ) -> Result<Vec<TokenRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT t.id, p.name AS principal, t.name, t.created_at, t.expires_at,
                    t.last_used_at, t.revoked_at
             FROM token t JOIN principal p ON p.id = t.principal_id
             WHERE (? IS NULL OR t.principal_id = ?)
             ORDER BY t.created_at DESC, t.id",
        )
        .bind(principal.map(|p| p.id))
        .bind(principal.map(|p| p.id))
        .fetch_all(&self.reader)
        .await?;
        Ok(rows
            .iter()
            .map(|r| TokenRow {
                id: r.get("id"),
                principal: r.get("principal"),
                name: r.get("name"),
                created_at: r.get("created_at"),
                expires_at: r.get("expires_at"),
                last_used_at: r.get("last_used_at"),
                revoked_at: r.get("revoked_at"),
            })
            .collect())
    }

    pub async fn token_expires_at(&self, id: &str) -> Result<Option<String>, StoreError> {
        Ok(
            sqlx::query_scalar("SELECT expires_at FROM token WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.reader)
                .await?,
        )
    }

    pub async fn list_grants(
        &self,
        principal_id: Option<i64>,
        feed_id: Option<i64>,
    ) -> Result<Vec<GrantRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT p.name AS principal, f.name AS feed, g.role, gp.pattern
             FROM feed_grant g
             JOIN principal p ON p.id = g.principal_id
             JOIN feed f ON f.id = g.feed_id
             LEFT JOIN grant_publish_pattern gp
                    ON gp.principal_id = g.principal_id AND gp.feed_id = g.feed_id
             WHERE (? IS NULL OR g.principal_id = ?) AND (? IS NULL OR g.feed_id = ?)
             ORDER BY f.name, p.name, gp.pattern",
        )
        .bind(principal_id)
        .bind(principal_id)
        .bind(feed_id)
        .bind(feed_id)
        .fetch_all(&self.reader)
        .await?;
        let mut grants: Vec<GrantRow> = Vec::new();
        for r in &rows {
            let principal: String = r.get("principal");
            let feed: String = r.get("feed");
            if grants
                .last()
                .is_none_or(|g| g.principal != principal || g.feed != feed)
            {
                grants.push(GrantRow {
                    principal,
                    feed,
                    role: r.get("role"),
                    publish_patterns: Vec::new(),
                });
            }
            if let Some(pattern) = r.get::<Option<String>, _>("pattern") {
                grants
                    .last_mut()
                    .expect("insertado arriba")
                    .publish_patterns
                    .push(pattern);
            }
        }
        Ok(grants)
    }

    /// Paquetes de un feed en orden de clave, a partir de `after` (exclusivo).
    pub async fn list_packages(
        &self,
        feed: &Feed,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<PackageRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT normalized_package_id, package_id FROM package
             WHERE feed_id = ? AND (? IS NULL OR normalized_package_id > ?)
             ORDER BY normalized_package_id LIMIT ?",
        )
        .bind(feed.id)
        .bind(after)
        .bind(after)
        .bind(i64::from(limit))
        .fetch_all(&self.reader)
        .await?;
        Ok(rows
            .iter()
            .map(|r| PackageRow {
                package_key: r.get("normalized_package_id"),
                package_id: r.get("package_id"),
            })
            .collect())
    }

    pub async fn list_audit(&self, q: &AuditQuery) -> Result<Vec<AuditRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT a.id, a.occurred_at, a.actor, a.action, f.name AS feed, a.resource,
                    a.outcome, a.detail
             FROM audit_event a LEFT JOIN feed f ON f.id = a.feed_id
             WHERE (? IS NULL OR a.id < ?)
               AND (? IS NULL OR a.feed_id = ?)
               AND (? IS NULL OR substr(a.action, 1, length(?)) = ?)
             ORDER BY a.id DESC LIMIT ?",
        )
        .bind(q.before_id)
        .bind(q.before_id)
        .bind(q.feed_id)
        .bind(q.feed_id)
        .bind(q.action_prefix.as_deref())
        .bind(q.action_prefix.as_deref())
        .bind(q.action_prefix.as_deref())
        .bind(i64::from(q.limit))
        .fetch_all(&self.reader)
        .await?;
        Ok(rows
            .iter()
            .map(|r| AuditRow {
                id: r.get("id"),
                occurred_at: r.get("occurred_at"),
                actor: r.get("actor"),
                action: r.get("action"),
                feed: r.get("feed"),
                resource: r.get("resource"),
                outcome: r.get("outcome"),
                detail: r.get("detail"),
            })
            .collect())
    }
}
