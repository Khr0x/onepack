-- Cuotas por feed y motivo de bloqueo (Fase 5: ADR-013, ADR-014).
--
-- Una cuota NULL significa sin límite. El uso se calcula como suma lógica: cada versión cuenta
-- el tamaño de su blob aunque otra versión comparta el mismo contenido.

ALTER TABLE feed ADD COLUMN max_storage_bytes INTEGER CHECK (max_storage_bytes >= 0);
ALTER TABLE feed ADD COLUMN max_versions INTEGER CHECK (max_versions >= 0);

-- Motivo del último bloqueo; el historial completo queda en `audit_event`.
ALTER TABLE package_version ADD COLUMN blocked_reason TEXT;
