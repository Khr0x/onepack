# Administrative API `/api/v1`

> Phase 6 ([ADR-011](../roadmap/adr-mvp.md#adr-011), [ADR-015](../roadmap/adr-mvp.md#adr-015)).
> Last updated: 2026-10-07

Authentication: `Authorization: Bearer <token>` (Bearer only; the NuGet routes use Basic or `X-NuGet-ApiKey`). The request and response types live in `crates/api-client` and are shared by the server and the CLI.

## Endpoints

| Method and route | Description | Requires |
|---|---|---|
| `GET /capabilities` | Server version, API version and capabilities. | authenticated |
| `GET /whoami` | Principal, token (id and expiry) and grants. | authenticated |
| `GET /feeds` | Visible feeds, with usage and quotas. | authenticated |
| `POST /feeds` `{"name"}` | Creates a feed. `409 FEED_EXISTS`. | admin |
| `GET /feeds/{feed}` | Details. | read |
| `PATCH /feeds/{feed}` `{"max_storage_bytes", "max_versions"}` | Replaces the quotas (missing or `null`: no limit). | admin |
| `GET /principals` · `POST /principals` `{"name", "kind", "administrator"}` | | admin |
| `POST /principals/{name}/disable` | `409 LAST_ADMIN` for the last active administrator. | admin |
| `GET /tokens?principal=` · `POST /tokens` `{"principal", "name", "expires_in_days"}` | The `POST` response is the only one that includes the secret. | admin |
| `POST /tokens/{id}/revoke` | `404 TOKEN_NOT_FOUND` if there is no active token with that id. | admin |
| `GET /grants?principal=&feed=` | | admin |
| `PUT /feeds/{feed}/grants/{principal}` `{"role", "publish_patterns"}` | Creates or replaces. | admin |
| `DELETE /feeds/{feed}/grants/{principal}` | `204`; `404 GRANT_NOT_FOUND`. | admin |
| `GET /feeds/{feed}/packages` | Packages with their number of versions and the highest one. | read |
| `GET /feeds/{feed}/packages/{id}` | Versions by precedence. | read |
| `GET /feeds/{feed}/packages/{id}/{version}` | State, SHA-256, size; `blocked_reason` only for a Maintainer or an admin. | read |
| `POST /feeds/{feed}/packages/{id}/{version}/unlist` · `/relist` | Same rules as the NuGet protocol. | publish (and patterns) |
| `POST /feeds/{feed}/packages/{id}/{version}/block` · `/unblock` `{"reason"}` | Reason required and audited. | Maintainer |
| `GET /audit?feed=&action=` | Newest first; `action` filters by prefix. | admin |

A feed without read permission responds exactly like a nonexistent one (`404`). Denied attempts are audited.

## Pagination

Lists return `{"items": [...], "next_cursor": "..." | null}`. For the next page, repeat the request with `?cursor=<next_cursor>`. `limit` ranges from 1 to 500 (50 by default). The cursor is opaque.

## Errors

```json
{
  "error": {
    "code": "AUTH_SCOPE_MISSING",
    "message": "the credential is valid, but lacks the packages:publish permission on this feed",
    "action": "ask an administrator for a sufficient role: `onepack grant add --principal <principal> --feed <feed> --role <role>`",
    "request_id": "3f9c0a1b2c3d4e5f"
  }
}
```

`code` is stable. Besides those in [security](security.md#errores): `AUTH_REQUIRED`, `AUTH_SCOPE_MISSING`, `AUTH_PREFIX_DENIED`, `AUTH_ADMIN_REQUIRED`, `NOT_FOUND`, `FEED_EXISTS`, `PRINCIPAL_EXISTS`, `PRINCIPAL_NOT_FOUND`, `TOKEN_NOT_FOUND`, `GRANT_NOT_FOUND`, `LAST_ADMIN`, `INVALID_REQUEST`, `INVALID_NAME`, `INVALID_CURSOR` and `INTERNAL`.

Every response, including those on the NuGet routes, carries the `X-Request-Id` header. The same id appears in the server's trace span. The server always generates it; one sent by the client is ignored.

## Compatibility

- New fields are added as optional and nobody rejects unknown fields: CLI N works with server N-1 and vice versa.
- What a server can do is advertised by `GET /capabilities` (`feeds`, `feeds.quotas`, `principals`, `tokens`, `grants`, `packages`, `packages.listing`, `packages.availability`, `audit`). `api_version` changes only with incompatible changes.
