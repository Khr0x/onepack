-- Per-feed quotas and block reason (Phase 5: ADR-013, ADR-014).
--
-- A NULL quota means unlimited. Usage is computed as a logical sum: each version counts
-- the size of its blob even if another version shares the same content.

ALTER TABLE feed ADD COLUMN max_storage_bytes INTEGER CHECK (max_storage_bytes >= 0);
ALTER TABLE feed ADD COLUMN max_versions INTEGER CHECK (max_versions >= 0);

-- Reason for the last block; the full history remains in `audit_event`.
ALTER TABLE package_version ADD COLUMN blocked_reason TEXT;
