-- Identidades, tokens y permisos (Fase 4: ADR-010, ADR-012).

CREATE TABLE principal (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL UNIQUE,
    kind        TEXT    NOT NULL CHECK (kind IN ('user', 'service')),
    is_admin    INTEGER NOT NULL DEFAULT 0 CHECK (is_admin IN (0, 1)),
    disabled_at TEXT,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- El secreto nunca se guarda: solo su verificador SHA-256 (ADR-010).
CREATE TABLE token (
    id           TEXT    PRIMARY KEY CHECK (length(id) = 16),
    principal_id INTEGER NOT NULL REFERENCES principal (id),
    name         TEXT,
    verifier     TEXT    NOT NULL CHECK (length(verifier) = 64),
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    expires_at   TEXT    NOT NULL,
    revoked_at   TEXT,
    last_used_at TEXT
) STRICT;

CREATE INDEX token_principal ON token (principal_id);

CREATE TABLE feed_grant (
    principal_id INTEGER NOT NULL REFERENCES principal (id),
    feed_id      INTEGER NOT NULL REFERENCES feed (id),
    role         TEXT    NOT NULL CHECK (role IN ('reader', 'publisher', 'maintainer')),
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (principal_id, feed_id)
) STRICT;

-- Sin filas: el grant permite publicar cualquier id del feed.
CREATE TABLE grant_publish_pattern (
    principal_id INTEGER NOT NULL,
    feed_id      INTEGER NOT NULL,
    pattern      TEXT    NOT NULL,
    PRIMARY KEY (principal_id, feed_id, pattern),
    FOREIGN KEY (principal_id, feed_id) REFERENCES feed_grant (principal_id, feed_id) ON DELETE CASCADE
) STRICT;
