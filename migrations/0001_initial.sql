-- Esquema inicial (Fase 2). Migraciones solo hacia adelante (ADR-020).
-- Fechas en UTC con formato RFC 3339 y milisegundos.

CREATE TABLE feed (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL UNIQUE,
    format     TEXT    NOT NULL CHECK (format IN ('nuget')),
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Artefacto inmutable direccionado por contenido (ADR-005).
CREATE TABLE blob (
    sha256     TEXT    PRIMARY KEY CHECK (length(sha256) = 64),
    size       INTEGER NOT NULL CHECK (size >= 0),
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE package (
    feed_id               INTEGER NOT NULL REFERENCES feed (id),
    normalized_package_id TEXT    NOT NULL,
    -- Capitalización de la primera publicación.
    package_id            TEXT    NOT NULL,
    created_at            TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (feed_id, normalized_package_id)
) STRICT;

CREATE TABLE package_version (
    id                    INTEGER PRIMARY KEY,
    feed_id               INTEGER NOT NULL,
    normalized_package_id TEXT    NOT NULL,
    normalized_version    TEXT    NOT NULL,
    -- Id y versión tal como los publicó el autor (versión normalizada, sin metadatos).
    package_id            TEXT    NOT NULL,
    version               TEXT    NOT NULL,
    full_version          TEXT    NOT NULL,
    is_prerelease         INTEGER NOT NULL CHECK (is_prerelease IN (0, 1)),
    is_semver2            INTEGER NOT NULL CHECK (is_semver2 IN (0, 1)),
    blob_sha256           TEXT    NOT NULL REFERENCES blob (sha256),
    -- Descubrimiento y disponibilidad son independientes (ADR-013).
    listed                INTEGER NOT NULL DEFAULT 1 CHECK (listed IN (0, 1)),
    availability          TEXT    NOT NULL DEFAULT 'available' CHECK (availability IN ('available', 'blocked')),
    published_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (feed_id, normalized_package_id) REFERENCES package (feed_id, normalized_package_id),
    -- Identidad inmutable (ADR-007).
    UNIQUE (feed_id, normalized_package_id, normalized_version)
) STRICT;

CREATE INDEX package_version_blob ON package_version (blob_sha256);

CREATE TABLE audit_event (
    id          INTEGER PRIMARY KEY,
    occurred_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Principal que actuó. NULL hasta que exista autenticación (Fase 4).
    actor       TEXT,
    action      TEXT    NOT NULL,
    feed_id     INTEGER REFERENCES feed (id),
    resource    TEXT,
    outcome     TEXT    NOT NULL CHECK (outcome IN ('success', 'conflict', 'denied', 'error')),
    detail      TEXT
) STRICT;

CREATE INDEX audit_event_occurred_at ON audit_event (occurred_at);
