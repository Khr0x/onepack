-- Maintenance mode (Phase 7: ADR-017).
--
-- A single row while active. `onepackd backup` creates it and deletes it when done; because
-- different processes (CLI and server) share it, it lives in the database. `expires_at` prevents
-- an interrupted backup from leaving the server in maintenance forever.

CREATE TABLE maintenance (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    reason     TEXT    NOT NULL,
    actor      TEXT,
    started_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    expires_at TEXT    NOT NULL
) STRICT;
