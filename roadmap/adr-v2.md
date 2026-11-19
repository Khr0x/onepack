# Architecture Decision Records — v2 multi-ecosystem

> Contract design for serving NuGet/.NET, npm/JavaScript and PyPI/Python from the same
> private registry. The scope agreed in Obsidian (`Onepack-Alcance-v2`) is materialized
> here in technical decisions; this document is the repository source.
>
> Task: OP-021. Depends on OP-020 (verified scope). Enables OP-022 and the OP-023
> (npm) and OP-027 (PyPI) specs. Does not implement adapters.
>
> Baseline consulted: `88467a94eaee4fc7345bac54fdf6fa4449a913b8`.
> Date: 2026-10-19. Status: **accepted**. Review on 2026-10-19 by the maintainer
> (instruction in the work session); ADR-021…026 accepted without scope changes.
> Implementation still corresponds to OP-022, OP-024 and OP-033.

## Index

| ADR | Decision | Status |
|---|---|---|
| [021](#adr-021) | Feeds typed by ecosystem; immutable `format` | Accepted |
| [022](#adr-022) | Identity, version and ordering delegated to the adapter | Accepted |
| [023](#adr-023) | Multiple files per version with `version_file` | Accepted |
| [024](#adr-024) | Routes, capabilities and additive contracts per protocol | Accepted |
| [025](#adr-025) | `feed_format` as the authoritative table; the v1 CHECK remains as legacy | Accepted |
| [026](#adr-026) | Crate boundary: adapters outside `core`/`storage` | Accepted |

---

## 1. Scope of this specification

This spec resolves the design of the three ecosystems and leaves the OP-022 cut ready. It does
not include:

- The client matrix or the exact npm (OP-023) and PyPI (OP-027) flows.
- The implementation of adapters, routes or migrations (OP-022, OP-024, OP-033).
- SSO, UI, PostgreSQL and high availability (cross-cutting proposals).

The existing modular separation (OP-037/OP-039) is preserved. This document only describes
the real changes that multi-ecosystem support requires.

## 2. Inventory of consumers and NuGet assumptions

Walkthrough of the baseline to separate the reusable base from the necessary changes. Symbols
are cited by file; line numbers are not fixed.

| Layer / file | Current NuGet assumption | Reusable | Necessary change |
|---|---|---|---|
| `migrations/0001_initial.sql` | `feed.format TEXT NOT NULL CHECK (format IN ('nuget'))`; `package_version.is_semver2`; a single `blob_sha256` per version | Immutable identity, `blob` by SHA-256, auditing | New `feed_format` and `version_file`; document `is_semver2` as a NuGet signal |
| `crates/core/src/lib.rs` | `Feed` without format; `NewVersion`/`PublishedVersion` with `is_prerelease` and `is_semver2`; generic `package_key`/`version_key` | Identity `package_key + version_key` (ADR-007), quotas, `PublishError` | Add `format` to `Feed`; document flag semantics; no NuGet types |
| `crates/core/src/auth.rs` | `PublishPattern` accepts `[A-Za-z0-9._-]` and a trailing `*`; rejects `@` and `/` | Cumulative roles and exact/prefix pattern | Validation by adapter; allow `@` and `/` in npm patterns |
| `crates/storage/src/lib.rs` | `create_feed` inserts the literal `'nuget'`; the quota sums the blob of each version | Publication SQL, staging, quotas, auditing | Insert `feed_format`; quota sums the version files |
| `crates/storage/src/catalog.rs` | `is_prerelease` and `is_semver2` filters in search | Pagination and search by `search_text` | `is_semver2` filter only for NuGet; adapter decides |
| `crates/server/src/lib.rs` | `/nuget/{feed}` routes; `backfill_metadata` reads `.nupkg` with `onepack_nuget` | Layers, auth, limits, metrics, maintenance | Routes per protocol; backfill dispatched by adapter |
| `crates/server/src/nuget_api.rs` | Builds `NewVersion` from `.nuspec`; metadata JSON with `v3::catalog_metadata` | Staging → inspection → `store.publish` flow | Extract the common flow; the adapter produces `NewVersion` and files |
| `crates/server/src/admin_api.rs` | `Feed` DTO without `format`; `capabilities` without ecosystems | Generic administrative endpoints | Add `format`; `formats.*` capabilities |
| `crates/server/src/ops.rs` | `Surface::NuGet` as the only package surface | Metrics and probes | `npm` and `pypi` surfaces |
| `crates/api-client/src/lib.rs` | DTOs and capabilities without format | Additive and tolerant contract (`serde(default)`) | Optional `format`; new capabilities |
| `crates/cli/src/nuget_config.rs`, `nuget.rs` | `nuget init`, `onepack exec`, `NuGet.Config` generation | `onepack exec`, credentials, doctor | Commands per ecosystem in OP-025/OP-034 |
| `docs/api.md`, `docs/compatibility.md`, `docs/security.md` | Document only NuGet | Structure and criteria | Sections per protocol when implementing |
| `crates/nuget/*`, `tests/conformance-dotnet/*` | NuGet adapter and corpus | Adapter model and corpus | Replica for npm and PyPI |

**Inventory conclusion:** `core` and `storage` do not contain NuGet types today. The real leaks
are: the `feed.format` `CHECK`, the `is_semver2` flags (NuGet semantics), the `'nuget'` literal
in `create_feed`, the server routes, metadata backfill, the metrics surface and publication
patterns. None of them forces `onepack-nuget` into `core` or `storage`.

## 3. Decisions

<a id="adr-021"></a>

### ADR-021 — Feeds typed by ecosystem; immutable `format`

**Context.** A feed today is a row with a unique name and `format` limited to `nuget`. The
three protocols have incompatible name, version, ordering and distribution rules; mixing
packages from different ecosystems in the same identity produces collisions and ambiguous
permissions.

**Decision.** Each feed declares exactly one format (`nuget`, `npm`, `pypi`) when created and
cannot change it. The feed name remains unique across the entire registry. To use the same
logical namespace in two ecosystems, different feeds are created (`internal`,
`internal-npm`). A missing format in the API is interpreted as `nuget`, which preserves v1
behavior.

**Alternatives considered.**

- *Mixed feed:* one row hosts several formats. Discarded: it requires composite identity,
  search and permissions per protocol within the same feed, and opens collisions/confusion
  without a confirmed use case.
- *Unique name per format* (`(name, format)`): would allow `internal` in two ecosystems,
  but changes the key of all routes and grants. Postponed until a demonstrated need exists.

**Consequences.** Routes and permissions continue to be resolved per feed; there is no identity
ambiguity between protocols. The cost is that the same logical name requires two feeds and two
grants if two ecosystems are used. It will be reviewed if that real need appears.

<a id="adr-022"></a>

### ADR-022 — Identity, version and ordering delegated to the adapter

**Context.** `NewVersion`/`PublishedVersion` are already generic except for `is_semver2`, whose
meaning belongs to NuGet. Each ecosystem normalizes and orders differently: NuGet (ADR-008),
npm SemVer, Python PEP 440.

**Decision.**

- The domain keeps `package_id`, `package_key`, `version`, `version_key`, `full_version`,
  `is_prerelease`, `metadata` and `search_text` as opaque data.
- Each adapter validates, normalizes and orders; the registry never interprets ecosystem
  rules.
- `is_prerelease` is common: it exists in all three protocols.
- `is_semver2` is documented as a **NuGet-only signal** for `semVerLevel`. The npm and PyPI
  adapters write `false` and their routes do not read it. It is not renamed in v2 to avoid
  breaking the schema or existing filters; it remains explicit debt to review if another
  SemVer ecosystem appears.
- Version ordering (registration, `latest`, pages) is computed by the adapter; `storage`
  returns rows unordered by version.

**Examples and counterexamples.**

| Ecosystem | Published id | `package_key` | Published version | `version_key` | `full_version` | Note |
|---|---|---|---|---|---|---|
| NuGet | `Hemia.Logging` | `hemia.logging` | `1.0.0-Beta.1+sha.abc` | `1.0.0-beta.1` | `1.0.0-Beta.1+sha.abc` | Build metadata outside identity |
| NuGet | `hemia.logging` | `hemia.logging` | `1.0.0` | `1.0.0` | `1.0.0` | Second publication: `409` with different content |
| npm | `@acme/widgets` | `@acme/widgets` | `1.2.3-beta.1+build.5` | `1.2.3-beta.1` | `1.2.3-beta.1+build.5` | Build outside SemVer identity |
| npm | `Widgets` | — | — | — | — | Invalid: npm requires lowercase |
| PyPI | `Hemia_Logging` | `hemia-logging` | `1.0rc1` | `1.0rc1` | `1.0rc1` | PEP 503 and PEP 440 normalization |
| PyPI | `hemia.logging` | `hemia-logging` | `1.0` | `1.0` | `1.0` | Collides with `hemia-logging`; `409`, not two packages |

Identity rules per ecosystem (the adapter implements and tests them with a corpus):

- **NuGet:** case-insensitive; versions `1`, `1.0`, `1.0.0` and `1.0.0+meta` are the same
  identity; prerelease and SemVer 2.0.0 according to ADR-008.
- **npm:** lowercase name, scoped `@scope/name` or unscoped; valid SemVer version;
  build metadata does not enter identity; dist-tags (`latest`, `next`) are not
  versions and do not create identity.
- **PyPI:** name normalized with PEP 503 (`_`, `.`, uppercase → lowercase `-`); canonical
  PEP 440 version; local versions (`+local`) are preserved as-is in the identity
  (not collapsed); prerelease/release/post according to PEP 440.

**Consequences.** Conformance corpora are mandatory per adapter, like the one in
`crates/nuget/tests/version_corpus.rs`. The cost of a new ecosystem is bounded to an adapter
crate plus its corpus.

<a id="adr-023"></a>

### ADR-023 — Multiple files per version with `version_file`

**Context.** `package_version` stores a single `blob_sha256`. NuGet publishes one `.nupkg` per
version, but PyPI supports several files per version (multiple wheels and an sdist) and npm
publishes a tarball plus metadata. Python is mandatory in v2.

**Decision.**

- `version_file(version_id, filename, kind, sha256, size)` is added with primary key
  `(version_id, filename)` and an FK to `blob`.
- `package_version.blob_sha256` is kept as the **primary artifact** to avoid breaking v1
  queries. The migration fills `version_file` with that blob and a name synthesized by
  the NuGet adapter (`{id}.{version}.nupkg` in lowercase, like the flat container).
- `kind` distinguishes the role of the file (`package`, `tarball`, `wheel`, `sdist`, `metadata`).
  The exact list is fixed in OP-023/OP-027; the value is opaque to `storage`.
- A `filename` is immutable: uploading the same name with different bytes responds `409`
  (`VERSION_FILE_CONFLICT`). The comparison is exact on the name; the adapter
  normalizes when its protocol requires it. Uploading another name to an existing version is
  valid only for ecosystems that allow it; the adapter decides (PyPI does; NuGet does not).
- The feed quota sums the size of the files of each version. A blob shared between
  versions is counted once per version, as today.
- In the first cut, files are not deleted from the registry; withdrawing (yank/unlist) is a
  state change, not a deletion.

**Consequences.** PyPI can publish several distributions without duplicating the version row
or inventing identities. NuGet continues to serve the primary artifact without changes.

<a id="adr-024"></a>

### ADR-024 — Routes, capabilities and additive contracts per protocol

**Context.** The `/api/v1` contract is additive and tolerant of unknown fields; the CLI
negotiates capabilities. Current routes are only `/nuget/{feed}`.

**Decision.**

- Routes namespaced by protocol and feed: `/nuget/{feed}/...` (unchanged), `/npm/{feed}/...`
  and `/pypi/{feed}/...`. The exact surface is fixed by OP-023 and OP-027.
- A protocol client that hits a feed of another format receives `404` with
  `FEED_NOT_FOUND`, just like a nonexistent feed: existence is not leaked between protocols.
- New capabilities: `formats` and `formats.nuget`, `formats.npm`, `formats.pypi`. The server
  announces only the compiled adapters; an old CLI ignores the new values.
- `/api/v1/feeds` adds `format` (`#[serde(default)]` → `nuget`); `POST /api/v1/feeds` accepts
  an optional `format`; `PATCH` cannot change it (`FEED_FORMAT_IMMUTABLE`).
- Metrics: the `surface` label becomes `nuget`, `npm`, `pypi`, `admin` or `probe`.
- The CLI gains `feed create --format <nuget|npm|pypi>` and shows the format; ecosystem
  configuration commands arrive in OP-025 and OP-034.
- New errors: `FEED_FORMAT_IMMUTABLE`, `VERSION_FILE_CONFLICT`,
  `FEED_FORMAT_MISMATCH` (internal use if a route and a feed do not match). The current
  stable codes are preserved.

**Compatibility.** A v1 CLI against a v2 server: unknown `format` is ignored;
`create feed` without `format` creates `nuget`. A v2 CLI against a v1 server: the `formats.*`
capabilities are missing and commands for other ecosystems fail before calling. NuGet clients
see no difference.

<a id="adr-025"></a>

### ADR-025 — `feed_format` as the authoritative table; the v1 CHECK remains as legacy

**Context.** SQLite does not allow expanding a `CHECK` without rebuilding the table, and `feed`
has FKs from `package`, `package_version`, `feed_grant` and `audit_event`. Rebuilding the table
in a forward-only migration is riskier than adding a table.

**Decision.** Migration `0006` creates:

```sql
CREATE TABLE feed_format (
    feed_id INTEGER PRIMARY KEY REFERENCES feed (id) ON DELETE CASCADE,
    format  TEXT NOT NULL CHECK (format IN ('nuget', 'npm', 'pypi'))
) STRICT;
```

- Fills all existing rows with `nuget`.
- `create_feed` still inserts `'nuget'` in the legacy column to satisfy its `CHECK` and
  writes the real format to `feed_format`. The `feed.format` column is not read to decide
  behavior; it is documented as v1 legacy and will be removed when a table rebuild
  is justified.
- A feed's format is read with `LEFT JOIN feed_format`, with `nuget` as the default value
  if the row were missing.

**Consequences.** Safe migration, without rebuilding or FK risk, and compatible with
v1 databases. The cost is a legacy column that lies for new feeds; it is recorded as
debt with an exit condition (rebuilding `feed` in a future migration).

<a id="adr-026"></a>

### ADR-026 — Crate boundary: adapters outside `core`/`storage`

**Context.** `onepack-nuget` already concentrates NuGet validation, normalization, inspection and
JSON. The ADR-003 rule forbids `core`, `storage` and the domain from depending on an ecosystem.

**Decision.**

- New crates `onepack-npm` and `onepack-pypi`, mirroring `onepack-nuget`: file
  inspection, normalization, ordering and protocol serialization.
- The server gains route modules per protocol (`npm_api`, `pypi_api`) and shares the common
  upload flow: staging → limit → `tokio::task::spawn_blocking` → adapter inspection →
  `NewVersion` + files → `store.publish`.
- `backfill_metadata` stops calling `onepack_nuget` directly: the feed's adapter
  provides the reader (registry per format in the server).
- `core` and `storage` receive no ecosystem dependencies. If a common interface appears,
  it will be a minimal trait defined in `core` with implementations in the adapter crates;
  an adapter factory is not created before two real consumers exist.

**Consequences.** A new ecosystem is a crate and a route module. The binary grows with
the compiled adapters, as `capabilities` already announces.

## 4. Permissions and publication patterns

Roles remain `reader`, `publisher` and `maintainer`, cumulative, with a global `is_admin`.
The pattern policy belongs to the registry and is applied to `package_key`; its syntax
must support the shape of each ecosystem.

| Ecosystem | Valid pattern | `package_key` | Does it match? |
|---|---|---|---|
| NuGet | `Hemia.Payments.*` | `hemia.payments.core` | Yes |
| NuGet | `Hemia.Payments.*` | `hemia.payments.core.extra` | Yes |
| NuGet | `Hemia.Payments.*` | `hemia.paymentsx` | No (the prefix includes the dot) |
| npm | `@acme/*` | `@acme/widgets` | Yes |
| npm | `@acme/widgets` | `@acme/widgets` | Yes |
| npm | `widgets` | `widgets` | Yes (`@acme/widgets` does not match) |
| PyPI | `hemia-*` | `hemia-logging` | Yes |
| PyPI | `hemia-logging` | `hemia.logging` | Yes (normalized to `hemia-logging`) |

Decision: `PublishPattern` keeps the trailing `*` wildcard and exact or prefix matching,
in lowercase. Character validation is delegated to the adapter at the API edge; the core
stops rejecting `@` and `/`. An empty pattern still means "any id in the feed". Without
patterns, there is no additional restriction.

## 5. API, CLI and error contracts

### Administrative API (`/api/v1`)

| Change | Shape | Compatibility |
|---|---|---|
| `Feed.format` | `"nuget" \| "npm" \| "pypi"` | Additive; missing is interpreted as `nuget` |
| `CreateFeed.format` | optional; defaults to `nuget` | A v1 client still creates NuGet |
| Capabilities | `formats`, `formats.nuget`, `formats.npm`, `formats.pypi` | Additive |
| `PATCH /feeds/{feed}` | does not accept `format` | `409 FEED_FORMAT_IMMUTABLE` |
| `PackageVersion` | unchanged in v2; files are exposed on the protocol route | Additive if `files` is added later |

### CLI

| Command | v1 | v2 |
|---|---|---|
| `onepack feed create` | creates NuGet | optional `--format`; without the flag creates NuGet |
| `onepack feed show/list` | without format | show `format` |
| `onepack doctor` | checks NuGet feeds | checks the feed format and server capabilities |
| `onepack npm init` / `onepack python init` | — | OP-025 / OP-034 |

### New stable errors

| Code | When | Response |
|---|---|---|
| `FEED_FORMAT_IMMUTABLE` | `PATCH` tries to change `format` | `409` |
| `VERSION_FILE_CONFLICT` | same `filename`, different bytes | `409` |
| `FEED_FORMAT_MISMATCH` | protocol route over a feed of another format | `404` visible as `FEED_NOT_FOUND` |

## 6. Migration and compatibility

Migration `0006_multiecosistema.sql` is forward-only (ADR-020) and does everything in one
transaction:

1. Creates `feed_format` and `version_file`.
2. Fills `feed_format` with `nuget` for each existing feed.
3. Fills `version_file` from `package_version.blob_sha256` with `kind = 'package'` and a
   name synthesized by the NuGet adapter.
4. Does not touch `feed`, `package`, `package_version`, grants or auditing.

Before migrating, the current backup procedure is followed
([docs/runbooks/backup-restore.md](../docs/runbooks/backup-restore.md)); the rehearsal of
`onepackd backup` + migration + restore is part of the OP-022 evidence. The existing backup
already covers the new tables and blobs; no different data format or new recovery is
introduced.

Verifiable compatibility:

- v1 feeds and data remain NuGet and respond on the same routes.
- An existing `package_version` keeps its primary blob and gains a row in
  `version_file`; NuGet downloads do not change.
- The v1 CLI talks to the v2 server and vice versa (new fields tolerated; new commands
  blocked by capabilities).
- Permissions, tokens, auditing and quotas are reused without changes in meaning.

## 7. Implementation plan for OP-022 (handoff)

Minimum suggested order, each step with its test:

1. Migration `0006` and migration tests from a v1 database with feeds and versions.
2. `feed_format` in `storage` and `Feed.format` in `core`/`api-client`; `create_feed` with
   format.
3. `version_file` in `storage` and file reading; quota by files.
4. Common publication flow extracted from `nuget_api`; adapter registry in the
   server.
5. Empty npm and PyPI routes that respond `FEED_FORMAT_MISMATCH` when appropriate; metrics
   surface per protocol.
6. Real adapters in OP-024 and OP-033; specs in OP-023 and OP-027.

Minimum tests that OP-022 must deliver:

- Migration: v1 database with feeds, versions and blobs → complete `feed_format` and
  `version_file`; same downloadable content and same quotas.
- Boundaries: `./scripts/check-crate-deps.sh` and the dependency matrix confirm that
  `core`/`storage` do not depend on `onepack-nuget`, `onepack-npm` or `onepack-pypi`.
- Compatibility: v1 CLI against v2 server and v2 CLI against v1 server (create/read feeds,
  missing `format`); NuGet clients without regression.
- Isolation: a token with a grant on one feed cannot read or publish to a feed of another
  format, and a feed of format A does not respond on the route of format B.
- Conformance: identity/version corpus per adapter before exposing its routes.

## 8. Pending decisions for OP-023 and OP-027

- **npm:** client matrix (npm 10/11; pnpm/yarn outside the first cut), publication
  authentication mechanism, dist-tag policy (move `latest`, do not delete), tarball limits
  and SemVer corpus.
- **PyPI:** pip and twine versions, simple index (PEP 503) and JSON (PEP 691), yanked mapping
  (PEP 592) without deleting bytes, PEP 440 corpus, and whether a new file is allowed in an
  already published version (ADR-023 says the adapter decides; the spec must fix it).
- **Version ordering:** executable corpora in CI, like the NuGet one, for npm and PyPI.
- **Search:** `search_text` and metadata per adapter; no cross-format search.

## 9. Verification of this specification

- AC-01: inventory in section 2, with `CHECK format IN ('nuget')` and `is_semver2`
  pointed out.
- AC-02: ADR-021 (feeds) and the identity table with examples and counterexamples for the three
  protocols; decision accepted on 2026-10-19.
- AC-03: sections 4–6 with API/CLI contracts, errors, migration and compatibility; `core`
  and `storage` preserve their boundary without NuGet dependencies.
- AC-CIERRE: closed with the recorded acceptance of 2026-10-19, the OP-021 note and the
  backlog synchronized.

Evidence: path of this document, SHA-256 of the accepted content, local commit `514e439`
(delivery) and the acceptance recorded in Obsidian on 2026-10-19.
