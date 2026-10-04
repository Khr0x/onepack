# API administrativa `/api/v1`

> Fase 6 ([ADR-011](../roadmap/adr-mvp.md#adr-011), [ADR-015](../roadmap/adr-mvp.md#adr-015)).
> Última actualización: 2026-10-04

Autenticación: `Authorization: Bearer <token>` (solo Bearer; las rutas NuGet usan Basic o `X-NuGet-ApiKey`). Los tipos de petición y respuesta están en `crates/api-client` y los comparten el servidor y el CLI.

## Endpoints

| Método y ruta | Descripción | Requiere |
|---|---|---|
| `GET /capabilities` | Versión del servidor, versión de la API y capacidades. | autenticado |
| `GET /whoami` | Principal, token (id y caducidad) y grants. | autenticado |
| `GET /feeds` | Feeds visibles, con uso y cuotas. | autenticado |
| `POST /feeds` `{"name"}` | Crea un feed. `409 FEED_EXISTS`. | admin |
| `GET /feeds/{feed}` | Detalle. | lectura |
| `PATCH /feeds/{feed}` `{"max_storage_bytes", "max_versions"}` | Reemplaza las cuotas (ausente o `null`: sin límite). | admin |
| `GET /principals` · `POST /principals` `{"name", "kind", "administrator"}` | | admin |
| `POST /principals/{name}/disable` | `409 LAST_ADMIN` para el último administrador activo. | admin |
| `GET /tokens?principal=` · `POST /tokens` `{"principal", "name", "expires_in_days"}` | La respuesta de `POST` es la única que incluye el secreto. | admin |
| `POST /tokens/{id}/revoke` | `404 TOKEN_NOT_FOUND` si no hay un token activo con ese id. | admin |
| `GET /grants?principal=&feed=` | | admin |
| `PUT /feeds/{feed}/grants/{principal}` `{"role", "publish_patterns"}` | Crea o reemplaza. | admin |
| `DELETE /feeds/{feed}/grants/{principal}` | `204`; `404 GRANT_NOT_FOUND`. | admin |
| `GET /feeds/{feed}/packages` | Paquetes con número de versiones y la más alta. | lectura |
| `GET /feeds/{feed}/packages/{id}` | Versiones por precedencia. | lectura |
| `GET /feeds/{feed}/packages/{id}/{version}` | Estado, SHA-256, tamaño; `blocked_reason` solo para Maintainer o admin. | lectura |
| `POST /feeds/{feed}/packages/{id}/{version}/unlist` · `/relist` | Mismas reglas que el protocolo NuGet. | publicación (y patrones) |
| `POST /feeds/{feed}/packages/{id}/{version}/block` · `/unblock` `{"reason"}` | Motivo obligatorio y auditado. | Maintainer |
| `GET /audit?feed=&action=` | Del más reciente al más antiguo; `action` filtra por prefijo. | admin |

Un feed sin permiso de lectura responde igual que uno inexistente (`404`). Los intentos denegados se auditan.

## Paginación

Los listados devuelven `{"items": [...], "next_cursor": "..." | null}`. Para la página siguiente, repite la petición con `?cursor=<next_cursor>`. `limit` va de 1 a 500 (50 por defecto). El cursor es opaco.

## Errores

```json
{
  "error": {
    "code": "AUTH_SCOPE_MISSING",
    "message": "la credencial es válida, pero no tiene el permiso packages:publish en este feed",
    "action": "pide a un administrador un rol suficiente: `onepack grant add --principal <principal> --feed <feed> --role <rol>`",
    "request_id": "3f9c0a1b2c3d4e5f"
  }
}
```

`code` es estable. Además de los de [seguridad](security.md#errores): `AUTH_REQUIRED`, `AUTH_SCOPE_MISSING`, `AUTH_PREFIX_DENIED`, `AUTH_ADMIN_REQUIRED`, `NOT_FOUND`, `FEED_EXISTS`, `PRINCIPAL_EXISTS`, `PRINCIPAL_NOT_FOUND`, `TOKEN_NOT_FOUND`, `GRANT_NOT_FOUND`, `LAST_ADMIN`, `INVALID_REQUEST`, `INVALID_NAME`, `INVALID_CURSOR` e `INTERNAL`.

Todas las respuestas, también las de las rutas NuGet, llevan la cabecera `X-Request-Id`. El mismo id aparece en el span de las trazas del servidor. Siempre lo genera el servidor; uno enviado por el cliente se ignora.

## Compatibilidad

- Los campos nuevos se añaden como opcionales y nadie rechaza campos desconocidos: un CLI N funciona con un servidor N-1 y al revés.
- Lo que un servidor sabe hacer lo anuncia `GET /capabilities` (`feeds`, `feeds.quotas`, `principals`, `tokens`, `grants`, `packages`, `packages.listing`, `packages.availability`, `audit`). `api_version` solo cambia con cambios incompatibles.
