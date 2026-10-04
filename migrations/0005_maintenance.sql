-- Modo mantenimiento (Fase 7: ADR-017).
--
-- Una sola fila mientras está activo. `onepackd backup` la crea y la borra al terminar; como
-- lo comparten procesos distintos (CLI y servidor), vive en la base. `expires_at` evita que
-- un backup interrumpido deje el servidor en mantenimiento para siempre.

CREATE TABLE maintenance (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    reason     TEXT    NOT NULL,
    actor      TEXT,
    started_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    expires_at TEXT    NOT NULL
) STRICT;
