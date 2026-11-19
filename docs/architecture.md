# Architecture

onepack is a self-hosted package registry. The server (`onepackd`) owns the
data and protocol endpoints; the CLI (`onepack`) administers the registry and
helps clients use it. NuGet clients connect directly to the server.

The MVP is a single-node modular monolith. PostgreSQL, high availability, SSO,
and a web UI are not part of the current architecture.

## Crates

| Crate | Responsibility |
|---|---|
| `onepack-core` | Domain types, identities, feeds, roles, publish patterns, and use cases. |
| `onepack-nuget` | NuGet package reading, version normalization, identity rules, and V3 contracts. |
| `onepack-storage` | SQLite metadata, content-addressed blobs, migrations, backup, restore, and integrity checks. |
| `onepack-api-client` | Administrative API contracts and the blocking HTTP client used by the CLI. |
| `onepack-server` | HTTP server, authentication, NuGet V3 API, admin API, operational endpoints, and metrics. |
| `onepack-cli` | Command-line administration, contexts, credentials, `onepack exec`, and `nuget init`. |
| `onepack-bench` | Metadata latency benchmark. Not distributed. |

Dependency direction is enforced by [`scripts/check-crate-deps.sh`](../scripts/check-crate-deps.sh)
and [ADR-003](../roadmap/adr-mvp.md#adr-003).

## Request flow

- **Publish:** a NuGet client or `onepack package push` sends a multipart
  request. The server authenticates the credential, checks the feed role and
  publish pattern, validates the package and its limits, writes a content-addressed
  blob, and commits metadata only after the blob is durable.
- **Restore:** NuGet reads the service index, flat-container or registration
  resources, and package content. The server authorizes each request and streams
  blobs without exposing the data directory.
- **Administration:** the CLI calls `/api/v1` for feeds, principals, tokens,
  grants, packages, audit entries, and maintenance. JSON output is a versioned
  contract.
- **Operations:** `/healthz`, `/readyz`, and metrics are served on the
  operational listener, separate from the public API.

## Data

- SQLite in WAL mode with `synchronous=FULL` for metadata
  ([ADR-004](../roadmap/adr-mvp.md#adr-004)).
- Package content stored as immutable blobs addressed by SHA-256
  ([ADR-005](../roadmap/adr-mvp.md#adr-005)).
- Staging directory for uploads; metadata is confirmed only after the blob is
  durable ([ADR-006](../roadmap/adr-mvp.md#adr-006)).
- Forward-only migrations with a prior backup
  ([ADR-020](../roadmap/adr-mvp.md#adr-020)).
- Backup uses maintenance mode for a consistent copy
  ([ADR-017](../roadmap/adr-mvp.md#adr-017)).

## Authentication and isolation

Opaque tokens are stored as SHA-256 verifiers. Roles are scoped per feed:
reader, publisher, and maintainer. Optional publish patterns restrict package
ids. Tokens can expire or be revoked immediately. The admin API and NuGet
endpoints use separate authentication contexts
([ADR-010](../roadmap/adr-mvp.md#adr-010),
[ADR-011](../roadmap/adr-mvp.md#adr-011),
[ADR-012](../roadmap/adr-mvp.md#adr-012)).

## Protocols

| Surface | Purpose |
|---|---|
| NuGet V3 | Service index, flat container, registration, search, and autocomplete. |
| `/api/v1` | Administrative API used by the CLI. |
| Operational listener | Health, readiness, and Prometheus metrics. |

Compatibility with real clients is documented in
[`compatibility.md`](compatibility.md).

## Deployment

A single `onepackd` process runs under systemd or in the published container.
A reverse proxy terminates TLS and forwards requests; the data directory is
never served directly ([ADR-018](../roadmap/adr-mvp.md#adr-018)). Data lives on
a local filesystem, so backups and restores are part of the deployment.

For installation, see [the server runbook](runbooks/install.md). For backup,
restore, and upgrades, see [the operational runbooks](runbooks/).

## Multi-ecosystem direction

The v2 design keeps identity, versions, and protocol behavior in per-ecosystem
adapters while sharing feeds, credentials, storage, and operations. See
[`roadmap/adr-v2.md`](../roadmap/adr-v2.md) and the npm and PyPI specifications.
