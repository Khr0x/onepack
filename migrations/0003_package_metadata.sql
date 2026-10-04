-- Metadatos para registros y búsqueda (Fase 3: ADR-009, ADR-013).
--
-- `metadata` es un documento JSON que genera el adaptador del formato al publicar; el dominio
-- lo guarda sin interpretarlo. `search_text` es el texto, en minúsculas, sobre el que busca
-- `SearchQueryService`. Las versiones publicadas antes de esta migración quedan con NULL y
-- `onepackd` las rellena leyendo el `.nuspec` de su blob.

ALTER TABLE package_version ADD COLUMN metadata TEXT;
ALTER TABLE package_version ADD COLUMN search_text TEXT;

CREATE INDEX package_version_search ON package_version (feed_id, listed);
