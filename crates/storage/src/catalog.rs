//! Catalog queries (search, visibility) and metadata backfill (Phase 3).

use onepack_core::{Feed, PublishedVersion};
use sqlx::Row;

use crate::identity::audit;
use crate::{Store, StoreError, to_published};

/// Version published before the `metadata` column existed (migration 0003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingMetadata {
    pub version_id: i64,
    pub blob_sha256: String,
}

/// Result of changing a version's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionChange {
    NotFound,
    Unchanged,
    Changed,
}

impl Store {
    /// Listed and available versions of a feed, with the prerelease and SemVer 2.0.0 filters. The text
    /// search and ordering are applied by the format adapter.
    pub async fn listed_versions(
        &self,
        feed: &Feed,
        include_prerelease: bool,
        include_semver2: bool,
    ) -> Result<Vec<PublishedVersion>, StoreError> {
        let rows = sqlx::query(
            "SELECT v.package_id, v.normalized_package_id, v.version, v.normalized_version, v.full_version,
                    v.is_prerelease, v.is_semver2, v.blob_sha256, b.size, v.listed, v.availability,
                    v.published_at, v.metadata, v.search_text
             FROM package_version v JOIN blob b ON b.sha256 = v.blob_sha256
             WHERE v.feed_id = ? AND v.listed = 1 AND v.availability = 'available'
               AND (? OR v.is_prerelease = 0)
               AND (? OR v.is_semver2 = 0)
             ORDER BY v.normalized_package_id",
        )
        .bind(feed.id)
        .bind(include_prerelease)
        .bind(include_semver2)
        .fetch_all(&self.reader)
        .await?;
        Ok(rows.iter().map(to_published).collect())
    }

    /// Lists or hides a version (NuGet unlist/relist). Does not affect download (ADR-013).
    pub async fn set_listed(
        &self,
        feed: &Feed,
        package_key: &str,
        version_key: &str,
        listed: bool,
        actor: &str,
    ) -> Result<VersionChange, StoreError> {
        let mut tx = self.writer.begin().await?;
        let row = sqlx::query(
            "SELECT id, listed, package_id, version FROM package_version
             WHERE feed_id = ? AND normalized_package_id = ? AND normalized_version = ?",
        )
        .bind(feed.id)
        .bind(package_key)
        .bind(version_key)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(VersionChange::NotFound);
        };
        if row.get::<bool, _>("listed") == listed {
            return Ok(VersionChange::Unchanged);
        }
        sqlx::query("UPDATE package_version SET listed = ? WHERE id = ?")
            .bind(listed)
            .bind(row.get::<i64, _>("id"))
            .execute(&mut *tx)
            .await?;
        let resource = format!(
            "{}@{}",
            row.get::<&str, _>("package_id"),
            row.get::<&str, _>("version")
        );
        let action = if listed {
            "package.relist"
        } else {
            "package.unlist"
        };
        audit(
            &mut tx,
            action,
            Some(actor),
            Some(feed.id),
            Some(&resource),
            "success",
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(VersionChange::Changed)
    }

    /// Blocks or unblocks the download of a version (ADR-013). The reason is mandatory and
    /// is recorded in the audit.
    pub async fn set_blocked(
        &self,
        feed: &Feed,
        package_key: &str,
        version_key: &str,
        blocked: bool,
        reason: &str,
        actor: &str,
    ) -> Result<VersionChange, StoreError> {
        let mut tx = self.writer.begin().await?;
        let row = sqlx::query(
            "SELECT id, availability, package_id, version FROM package_version
             WHERE feed_id = ? AND normalized_package_id = ? AND normalized_version = ?",
        )
        .bind(feed.id)
        .bind(package_key)
        .bind(version_key)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(VersionChange::NotFound);
        };
        if (row.get::<&str, _>("availability") == "blocked") == blocked {
            return Ok(VersionChange::Unchanged);
        }
        let (availability, action) = if blocked {
            ("blocked", "package.block")
        } else {
            ("available", "package.unblock")
        };
        sqlx::query("UPDATE package_version SET availability = ?, blocked_reason = ? WHERE id = ?")
            .bind(availability)
            .bind(blocked.then_some(reason))
            .bind(row.get::<i64, _>("id"))
            .execute(&mut *tx)
            .await?;
        let resource = format!(
            "{}@{}",
            row.get::<&str, _>("package_id"),
            row.get::<&str, _>("version")
        );
        audit(
            &mut tx,
            action,
            Some(actor),
            Some(feed.id),
            Some(&resource),
            "success",
            Some(serde_json::json!({ "reason": reason }).to_string()),
        )
        .await?;
        tx.commit().await?;
        Ok(VersionChange::Changed)
    }

    /// Reason for a version's current block, if it is blocked.
    pub async fn blocked_reason(
        &self,
        feed: &Feed,
        package_key: &str,
        version_key: &str,
    ) -> Result<Option<String>, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT blocked_reason FROM package_version
             WHERE feed_id = ? AND normalized_package_id = ? AND normalized_version = ?
               AND availability = 'blocked'",
        )
        .bind(feed.id)
        .bind(package_key)
        .bind(version_key)
        .fetch_optional(&self.reader)
        .await?
        .flatten())
    }

    pub async fn versions_missing_metadata(&self) -> Result<Vec<MissingMetadata>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, blob_sha256 FROM package_version WHERE metadata IS NULL ORDER BY id",
        )
        .fetch_all(&self.reader)
        .await?;
        Ok(rows
            .iter()
            .map(|r| MissingMetadata {
                version_id: r.get("id"),
                blob_sha256: r.get("blob_sha256"),
            })
            .collect())
    }

    pub async fn set_metadata(
        &self,
        version_id: i64,
        metadata: &str,
        search_text: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE package_version SET metadata = ?, search_text = ? WHERE id = ? AND metadata IS NULL",
        )
        .bind(metadata)
        .bind(search_text)
        .bind(version_id)
        .execute(&self.writer)
        .await?;
        Ok(())
    }
}
