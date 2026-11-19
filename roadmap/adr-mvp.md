# Architecture Decision Records — MVP private NuGet registry

> Architecture decisions for the MVP described in [mvp-private-nuget-registry.md](mvp-private-nuget-registry.md) and planned in [roadmap-mvp.md](roadmap-mvp.md).
>
> Last updated: 2026-10-03

---

## How to use this document

- Each ADR records **one** decision, its context, alternatives and consequences.
- An `Accepted` ADR is not substantively edited: if the decision changes, a new ADR is created that **supersedes** it, and the previous one becomes `Superseded by ADR-XXX`.
- Wording fixes, links or the *Review when* section may be edited.
- Each roadmap phase states which ADRs must be `Accepted` before moving to `IN PROGRESS`.

### ADR status

| Status | Meaning |
|---|---|
| 📝 `Proposed` | Drafted, pending review. Not binding yet. |
| 🧪 `Conditional` | Provisionally accepted until a specific validation is passed (stated in the ADR). |
| ✅ `Accepted` | In force. Code must comply with it. |
| ❌ `Rejected` | Evaluated and discarded. Kept as a record. |
| ♻️ `Superseded by ADR-XXX` | Was in force; another decision replaces it. |
| 🗄️ `Obsolete` | No longer applies (e.g. the component disappeared), with no direct replacement. |

### Template

```markdown
<a id="adr-XXX"></a>
## ADR-XXX — Title in decision form

**Status:** 📝 Proposed · **Date:** YYYY-MM-DD · **Phase:** N · **Related:** ADR-YYY

### Context
What problem or force requires a decision.

### Decision
What will be done, in one or a few affirmative sentences.

### Alternatives considered
- Option — why not.

### Consequences
- Positive / negative / obligations it creates.

### Review when
Concrete signal that would justify reopening the decision.
```

---

## Index

| ADR | Decision | Status | Phase |
|---|---|---|---|
| [001](#adr-001) | MVP: private NuGet registry, one organization, one node | ✅ Accepted | 0 |
| [002](#adr-002) | Rust + Tokio + Axum as the server and CLI stack | ✅ Accepted | 1 |
| [003](#adr-003) | Modular monolith in a crate workspace | ✅ Accepted | 0 |
| [004](#adr-004) | SQLite in WAL with `synchronous=FULL` for metadata | ✅ Accepted | 2 |
| [005](#adr-005) | Blobs addressed by SHA-256 on local filesystem, original bytes | ✅ Accepted | 2 |
| [006](#adr-006) | Durable blob before confirming metadata | ✅ Accepted | 2 |
| [007](#adr-007) | Immutable versions and `409` on duplicates | ✅ Accepted | 2 |
| [008](#adr-008) | Own NuGet normalization validated against `NuGet.Versioning` | ✅ Accepted | 1 |
| [009](#adr-009) | NuGet V3 resources announced in the MVP | ✅ Accepted | 1, 3 |
| [010](#adr-010) | Opaque tokens with a hash verifier, not JWT | ✅ Accepted | 4 |
| [011](#adr-011) | Three separate authentication contexts | ✅ Accepted | 4 |
| [012](#adr-012) | Per-feed roles and optional prefix restriction | ✅ Accepted | 4 |
| [013](#adr-013) | `listed` and `availability` as independent states | ✅ Accepted | 3, 5 |
| [014](#adr-014) | Packages as untrusted content with an inspection budget | ✅ Accepted | 5 |
| [015](#adr-015) | `onepackd` and `onepack` separated; CLI with a stable contract | ✅ Accepted | 6 |
| [016](#adr-016) | NuGet credentials via `onepack exec`; credential provider postponed | ✅ Accepted | 6 |
| [017](#adr-017) | Consistent backup in maintenance mode | ✅ Accepted | 7 |
| [018](#adr-018) | Explicit `public_url` and blobs only through the server | ✅ Accepted | 3, 7 |
| [019](#adr-019) | Conformance proven with real .NET clients, without .NET at runtime | ✅ Accepted | 0, 1 |
| [020](#adr-020) | Forward-only migrations with a prior backup | ✅ Accepted | 2, 7 |

---

<a id="adr-001"></a>
## ADR-001 — MVP: private NuGet registry, one organization, one node

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-03 · **Phase:** 0 · **Related:** ADR-002, ADR-004

### Context
Lightweight alternatives exist (BaGet, BaGetter). "Lightweight self-hosted NuGet" does not differentiate. Each additional feature (proxy, S3, SSO, UI, multinode) opens its own front in compatibility, security and operations.

### Decision
The MVP is a **private** NuGet package registry, for **one organization**, with **several feeds**, on a **single node**, operated **from the CLI**. The differentiation is: simple installation, complete operation from the terminal, scoped credentials, immutable versions and predictable recovery.

Out of the MVP: web UI, nuget.org proxy, virtual feeds, S3, PostgreSQL, HA, SSO, npm/PyPI, symbols, antimalware.

### Alternatives considered
- **Multiformat registry from the start** — multiplies compatibility work before validating demand.
- **Universal dependency proxy** — introduces outbound access, upstream credentials and identity collisions; it is a security project of its own.
- **Multi-tenant SaaS** — requires isolation and operations that do not contribute to the initial validation.

### Consequences
- Projects use the private feed and nuget.org simultaneously with `packageSourceMapping`.
- The domain must remain decoupled from NuGet so as not to close the door to other formats (see ADR-003).
- Any inclusion of excluded features requires a new ADR.

### Review when
The pilot (Phase 8) shows that an excluded feature is a condition for adopting the product.

---

<a id="adr-002"></a>
## ADR-002 — Rust + Tokio + Axum as the server and CLI stack

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 1 · **Related:** ADR-008, ADR-019

### Context
The product prioritizes native binaries, few components, resource control and a central CLI. C#/ASP.NET Core would reduce NuGet risk by reusing `NuGet.Packaging`, `NuGet.Versioning` and `NuGet.Protocol`.

### Decision
Server and CLI in **Rust**: Tokio, Axum, Tower/tower-http, SQLx (SQLite), clap, reqwest, serde (JSON/TOML).

**Condition:** the decision is ratified only if the Phase 1 spike demonstrates publish/restore with real `dotnet` and zero normalization divergences against `NuGet.Versioning`. Otherwise, an ADR is created to replace it in favor of C#.

**Validation:** condition met on 2026-10-04: 0 divergences against `NuGet.Versioning` and a green E2E with `dotnet` ([CI](https://github.com/Khr0x/onepack/actions/runs/37176042099), [report](../docs/spikes/phase-1-nuget-compatibility.md)).

### Alternatives considered
- **C# / ASP.NET Core** — lower compatibility risk; heavier runtime and a less native CLI. Discarded after the Phase 1 spike.
- **Go** — viable, with no clear advantage over Rust and the same NuGet compatibility work.

### Consequences
- Version normalization and `.nuspec` reading must be implemented in Rust (see ADR-008).
- .NET is used only in tests (see ADR-019).
- No Redis, Elasticsearch or external queues.

### Review when
The Phase 1 gate fails, or the accumulated cost of NuGet compatibility sustainably exceeds that of the rest of the product.

---

<a id="adr-003"></a>
## ADR-003 — Modular monolith in a crate workspace

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-03 · **Phase:** 0 · **Related:** ADR-001

### Context
A single node and a small team do not justify microservices, but the domain must not become "NuGet rules" if formats are added.

### Decision
A server process with two HTTP surfaces (NuGet and `/api/v1`), organized into crates:

| Crate | Responsibility | Cannot depend on |
|---|---|---|
| `core` | Domain: feeds, versions, permissions, use cases | `nuget`, `sqlx`, `axum` |
| `nuget` | Adapter: package reading, normalization, protocol routes | `server`, `cli` |
| `storage` | SQLite persistence and `BlobStore` | `nuget`, `axum` |
| `api-client` | Shared admin API contracts | `server` |
| `server` | `onepackd` binary, composition and HTTP | — |
| `cli` | `onepack` binary | `server`, `storage` |

Internal maintenance tasks in the same process. No dynamic plugins.

### Alternatives considered
- **Microservices** — operational cost with no benefit at this scale.
- **Single crate** — faster at the start, but boundaries erode without the compiler preventing it.

### Consequences
- Dependency rules are verified in CI with `scripts/check-crate-deps.sh` (over `cargo tree`, including transitive dependencies). Packages are named `onepack-<crate>`.
- Adding a format = a new adapter crate, no changes in `core`.

### Review when
A real requirement appears to scale a surface independently.

---

<a id="adr-004"></a>
## ADR-004 — SQLite in WAL with `synchronous=FULL` for metadata

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 2 · **Related:** ADR-001, ADR-017

### Context
The MVP must not require installing another database. SQLite in WAL supports concurrent readers and **a single writer**, and is not suitable for sharing over a network filesystem.

### Decision
- SQLite as the only metadata store, in WAL mode.
- `synchronous=FULL` by default (durability against power loss), evaluating its cost in benchmarks.
- Writes serialized by a single write point; short transactions; `busy_timeout` configured.
- Integrity constraints in the database (`UNIQUE`, `FOREIGN KEY`), not only in code.
- One active server, local disk. **No** replicas sharing the file.

### Alternatives considered
- **PostgreSQL** — adds a component to install and operate; reserved for when a multinode need is demonstrated.
- **`synchronous=NORMAL`** — better performance, but can lose committed transactions on power loss.

### Consequences
- Known write ceiling; acceptable for the expected publication volume.
- Downloads must not generate synchronous writes (counters aggregated in memory and flushed in batches).
- Backup must use the SQLite backup API (see ADR-017).

### Review when
Benchmarks show relevant write contention or more than one node is required.

---

<a id="adr-005"></a>
## ADR-005 — Blobs addressed by SHA-256 on local filesystem, original bytes

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 2 · **Related:** ADR-006, ADR-018

### Context
`.nupkg` files can be large; storing them in SQLite penalizes the database and backups. It must be possible to answer "what is the exact file we distribute?".

### Decision
- The `.nupkg` is stored **exactly** as received, at `blobs/sha256/<2>/<2>/<hash>`.
- The hash serves integrity and deduplication; **never** as a credential or as a public path.
- Access through a minimal `BlobStore` abstraction (put from staging, streaming get, exists, delete).
- The `.nuspec` is served by extracting it from the blob (or cached), not by repackaging.

### Alternatives considered
- **Blobs in SQLite** — huge database, slow backups, worse streaming.
- **Path by id/version** — breaks deduplication and complicates immutability.
- **Decompress and repackage** — alters the bytes and breaks author signatures.

### Consequences
- The same blob can be referenced by several versions/feeds; deletion requires reference counting and a grace period.
- Enables a future S3 driver without changing the domain.

### Review when
S3-compatible storage is implemented (first evolution).

---

<a id="adr-006"></a>
## ADR-006 — Durable blob before confirming metadata

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 2 · **Related:** ADR-004, ADR-005, ADR-007

### Context
SQLite and the filesystem do not share a transaction. One must choose which inconsistency is tolerated on a crash.

### Decision
Fixed publish order:

1. Authenticate, authorize, check quota.
2. Receive by streaming to `staging/` with a size limit.
3. Compute hash and inspect with limits (ADR-014).
4. Validate identity, version and metadata.
5. `fsync` the file, atomic `rename` to its final path, `fsync` the directory.
6. Single transaction: version metadata + blob reference + audit event.
7. Respond `201`; the version is visible from the commit.

An **orphan blob** is tolerated (deletable); never a **version without a blob**. No SQLite transaction remains open during the upload.

### Alternatives considered
- **Metadata first, blob later** — can expose versions without an artifact.
- **"Pending" state in the DB during the upload** — more writes and visible intermediate states.

### Consequences
- Maintenance task that cleans staging and unreferenced blobs after a grace period, paused during backups.
- `onepackd check` verifies the DB → blob invariant.

### Review when
A remote `BlobStore` with different consistency semantics is introduced.

---

<a id="adr-007"></a>
## ADR-007 — Immutable versions and `409` on duplicates

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 2 · **Related:** ADR-006, ADR-008, ADR-013

### Context
Replacing the content of a published version breaks reproducibility and traceability, and the NuGet protocol provides `409` for existing identities.

### Decision
- Identity: `feed + normalized_package_id + normalized_version`, with `UNIQUE` in the database.
- A published version **is not replaced or deleted** from the API in the MVP. To correct, publish another version; to withdraw, unlist or block (ADR-013).
- Duplicate → `409`.
- `onepack package push --skip-existing-identical` treats the duplicate as success **only** if the hash matches; with different bytes it remains an error.

### Alternatives considered
- **Allow overwrite with admin permission** — breaks client caches and auditing.
- **Hard delete** — postponed; if it arrives, it will be an audited administrative operation, not part of the normal flow.

### Consequences
- The race between two simultaneous publications is resolved in the database: one wins, the other receives `409`.
- Publication errors cost a version; this must be documented.

### Review when
A legal deletion requirement exists (e.g. content leaked by mistake) — it will require an audited purge ADR.

---

<a id="adr-008"></a>
## ADR-008 — Own NuGet normalization validated against `NuGet.Versioning`

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 1 · **Related:** ADR-002, ADR-019

### Context
NuGet versions are not pure SemVer: fourth segment, equivalences (`1.0` ≡ `1.0.0`), build metadata ignored in identity, case-insensitive ids. A mistake here produces duplicates or collisions.

### Decision
- Implement id and version normalization, and version comparison, in the `nuget` crate.
- Validate it with a **corpus of cases generated by `NuGet.Versioning`** (test .NET program) and run in CI.
- Do not use a generic SemVer library as the source of truth.

### Alternatives considered
- **Generic SemVer crate** — does not cover NuGet rules.
- **Call .NET at runtime** — breaks ADR-002 and ADR-019.

### Consequences
- Any divergence detected in the corpus is a blocking bug.
- The corpus grows with each rare case found in the pilot.

### Review when
NuGet changes its versioning rules.

---

<a id="adr-009"></a>
## ADR-009 — NuGet V3 resources announced in the MVP

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 1, 3 · **Related:** ADR-013, ADR-018

### Context
The client discovers capabilities in the service index. Announcing a resource without fulfilling its contract causes failures that are hard to diagnose.

### Decision
The service index announces **only** these resources, and only when they are complete:

| Resource | Path |
|---|---|
| `PackageBaseAddress/3.0.0` | `/nuget/{feed}/v3/flat/` |
| `RegistrationsBaseUrl/3.6.0` | `/nuget/{feed}/v3/registration/` |
| `SearchQueryService` | `/nuget/{feed}/v3/query` |
| `SearchAutocompleteService` | `/nuget/{feed}/v3/autocomplete` |
| `PackagePublish/2.0.0` | `/nuget/{feed}/v2/package` |

The V2 API (OData) is not implemented. `PackagePublish/2.0.0` is the V3 publish resource, not the V2 API.

**Clarification (Phase 3, 2026-10-04).** Search and autocomplete are also announced under `/3.0.0-beta`, `/3.0.0-rc` and `/3.5.0` (same endpoint), because each client version looks for a different type; `/3.5.0` requires the `packageType` filter, which is implemented. `RegistrationsBaseUrl/3.6.0` is served without gzip: clients accept uncompressed responses.

### Alternatives considered
- **Implement V2 OData** — expands the surface for old clients with no demand.
- **Announce everything from the spike** — confuses clients while resources are missing.

### Consequences
- A matrix of tested clients is published instead of promising "full compatibility".
- Search must respect pagination, prerelease and `semVerLevel`.

### Review when
A relevant pilot client requires an unimplemented resource (e.g. `ReadmeUriTemplate`, `RepositorySignatures`).

---

<a id="adr-010"></a>
## ADR-010 — Opaque tokens with a hash verifier, not JWT

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 4 · **Related:** ADR-011, ADR-012

### Context
Expiration, immediate revocation and scoped permissions are needed. Long-lived JWTs are not revoked without a blocklist, which nullifies their advantage.

### Decision
- Token = identifiable prefix (e.g. `pkg_`) + short public id + random secret ≥ 256 bits.
- The id and a **verifier** are stored (SHA-256 of the secret; sufficient due to its high entropy), never the recoverable token. Constant-time comparison.
- It is shown only once when created.
- Mandatory expiration (configurable), immediate revocation, `last_used_at` updated in aggregate.

### Alternatives considered
- **JWT** — complex revocation, risk of long-lived tokens.
- **Argon2/bcrypt for the verifier** — unnecessary with high-entropy secrets and costly per request.
- **Global API key** — no traceability or least privilege.

### Consequences
- Each authenticated request queries the database (mitigable with a short cache and invalidation on revocation).
- The prefix enables detection by secret scanners.

### Review when
SSO/OIDC or CI identity federation is integrated (e.g. GitHub Actions OIDC).

---

<a id="adr-011"></a>
## ADR-011 — Three separate authentication contexts

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 4 · **Related:** ADR-010, ADR-016

### Context
NuGet clients authenticate reads and publishes by different mechanisms, and the administrative CLI has its own channel.

### Decision

| Context | Mechanism |
|---|---|
| Administrative API `/api/v1` | `Authorization: Bearer <token>` |
| NuGet read (index, flat, registration, search, autocomplete) | Basic over HTTPS; user ignored or informative, password = token. `401` + `WWW-Authenticate: Basic` |
| NuGet publish | `X-NuGet-ApiKey: <token>` |

All resolve to the same principal and permission model. Authorization is applied on **all** routes, including `HEAD` and conditional responses.

**Clarification (Phase 4, 2026-10-04).** The context is determined by the surface, not by the operation: NuGet routes accept both Basic and `X-NuGet-ApiKey`, because `dotnet nuget push` queries the service index with Basic (after the `401`) and publishes with the API key. The administrative API accepts **only** Bearer: thus a browser that has cached Basic credentials cannot use them against it.

### Alternatives considered
- **A single mechanism** — not compatible with what NuGet clients send.
- **Protect only publishing** — exposes private packages.

### Consequences
- Automated test that walks all registered routes and verifies they require authentication.
- `anonymous_read=false` by default.

### Review when
The credential provider (ADR-016) or anonymous read per feed is implemented.

---

<a id="adr-012"></a>
## ADR-012 — Per-feed roles and optional prefix restriction

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 4 · **Related:** ADR-010, ADR-011

### Context
A CI account must be able to publish only what belongs to it. NuGet has no namespaces in the protocol.

### Decision
- Grants per `(principal, feed, role)` with cumulative roles: **Reader** ⊂ **Publisher** ⊂ **Maintainer**; **Administrator** is global (identities, permissions, configuration).
- Optional publish restriction by id prefix patterns (`Hemia.Payments.*`), as **registry policy**.
- Feeds without read permission respond `404` to avoid revealing their existence.
- Bootstrap: `onepackd init` generates the initial admin credential in a `0600` file; no default password or public admin registry.

### Alternatives considered
- **Per-package ACL** — granularity with no proven demand.
- **Global roles without feeds** — breaks per-client isolation.

### Consequences
- Authorization errors with stable codes (`AUTH_SCOPE_MISSING`, `AUTH_PREFIX_DENIED`).
- Grant changes are audited.

### Review when
Group/team or SSO requirements appear.

---

<a id="adr-013"></a>
## ADR-013 — `listed` and `availability` as independent states

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 3, 5 · **Related:** ADR-007

### Context
In NuGet, unlist removes a package from search but it remains downloadable. That is not useful for responding to incidents.

### Decision
Each version has two orthogonal attributes:

```text
listed:       true | false          (discovery; Publisher+ via NuGet protocol)
availability: available | blocked   (download; Maintainer+ via /api/v1, with reason)
```

- `listed=false`: out of search and autocomplete; present in the flat container and registrations with `listed=false`; downloadable.
- `blocked`: the server rejects download of the `.nupkg` and `.nuspec`; visible to Maintainer/Admin.

### Alternatives considered
- **A single state** — mixes a curation operation with a security one.

### Consequences
- It is documented that blocking does not delete copies already downloaded in client caches.
- Blocks and unblocks are audited with a reason.

**Clarification (Phase 5, 2026-10-04).** A blocked version remains in the flat container and registrations (announced as deprecated, without publishing the reason) and leaves search. Thus `restore` fails with an explicit error instead of silently resolving another version. Download responds `410` with the code `PACKAGE_BLOCKED` in the body and in the status phrase, which is what .NET clients display. Unblocking also requires a reason.

### Review when
Automatic quarantine is required (e.g. after a scanner).

---

<a id="adr-014"></a>
## ADR-014 — Packages as untrusted content with an inspection budget

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 5 · **Related:** ADR-006

### Context
A `.nupkg` is a ZIP with XML: exposed to ZIP bombs, XML bombs, path traversal and parser failures.

### Decision
- Configurable limits: compressed size, number of entries, uncompressed size per entry and total, `.nuspec` size, XML depth, inspection time.
- Read only what is needed from the ZIP into bounded memory; **do not** extract to disk.
- XML parser without DTD or external entities.
- Reject dangerous paths.
- Inspection with limited concurrency, isolated from download service.
- Never execute or download anything referenced by the package.
- Do not advertise antimalware analysis or signature validation.

### Alternatives considered
- **Trust authenticated publishers** — a compromised token or a defective package would bring down the service.
- **Built-in antimalware scanning** — outside the MVP; would require external dependencies.

### Consequences
- Very large legitimate packages may require adjusting limits; limits belong to the registry, not NuGet.
- `tests/security` suite with synthetic malicious artifacts.

**Clarification (Phase 5, 2026-10-04).** The limits, their default values and the error codes are in [docs/security.md](../docs/security.md). The number of entries is read from the ZIP end record before loading the central directory; uncompressed sizes are validated against the declared ones and only the `.nuspec` is decompressed, with reading cut off at the limit. Paths are also validated with the `%XX` escape resolved. Upload concurrency does not queue (`503` with `Retry-After`) and the per-IP limit is applied before authenticating, with the socket IP (ADR-018).

### Review when
Signature verification or scanner integration is added.

---

<a id="adr-015"></a>
## ADR-015 — `onepackd` and `onepack` separated; CLI with a stable contract

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 6 · **Related:** ADR-003, ADR-016

### Context
The CLI is the product's main interface, used by people and by pipelines.

### Decision
- `onepackd` (server) and `onepack` (client) are distinct binaries that share contracts via `api-client`. Installing `onepack` does not imply the server.
- CLI contract: `--json` with a stable schema, documented exit codes, `--no-input`, timeouts, data to stdout and diagnostics to stderr.
- Credentials: no-echo input or `--token-stdin`; OS keychain; **no silent fallback to plain text** (if there is no keychain, explicit error and `--token-env` option).
- Retries only on idempotent operations.
- Capability negotiation with the server (`/api/v1/capabilities`), N/N-1 compatibility.
- Errors with code, suggested action and `request_id`, never with secrets.

### Alternatives considered
- **A single binary with server and client subcommands** — forces distributing the server to every workstation.
- **Script over `curl`** — no credential handling or stable contract.

**Clarification (Phase 6, 2026-10-04).** Exit codes and the output contract are in [docs/cli.md](../docs/cli.md); the shape of `--json` is fixed by a snapshot in the tests. `ONEPACK_KEYRING=off` explicitly disables the keychain (CI, containers): `login` fails with `KEYCHAIN_UNAVAILABLE` instead of storing the token elsewhere. A server without `/api/v1/capabilities` (before Phase 6) is treated as one without administrative capabilities. The `request_id` is always generated by the server and travels in `X-Request-Id`.

### Consequences
- Incompatible changes in `--json` or exit codes require a major CLI version.
- `onepack doctor` is treated as product functionality, not as a debugging utility.

### Review when
A web UI or third-party SDK is added over `/api/v1`.

---

<a id="adr-016"></a>
## ADR-016 — NuGet credentials via `onepack exec`; credential provider postponed

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 6 · **Related:** ADR-011, ADR-015

### Context
Storing a token in the CLI does not make `dotnet restore` read it. Password encryption in `NuGet.Config` only works on Windows.

### Decision
- **CI:** credentials injected by standard NuGet mechanisms (`NuGetPackageSourceCredentials_{source}` or `dotnet nuget add source` with pipeline secrets).
- **Local development:** `onepack exec --feed X -- <command>` defines the credential variables only in the child process environment.
- `onepack nuget init` generates `NuGet.Config` **without secrets**, with `packageSourceMapping`, preserving what exists and showing the diff.
- The cross-platform credential provider is postponed to the first evolution.

### Alternatives considered
- **Write the token in `NuGet.Config`** — plain text outside Windows; risk of accidental commit.
- **Credential provider in the MVP** — better IDE experience, but an additional plugin protocol and more surface.

**Clarification (Phase 6, 2026-10-04).** The source key is `onepack_<feed>` (with `-` replaced by `_`), so the `NuGetPackageSourceCredentials_onepack_<feed>` variable is valid in any shell. If the `NuGet.Config` had no `packageSourceMapping`, `nuget init` maps its other sources (and `nuget.org` if the file inherits sources) to `*`, so as not to break existing restores.

### Consequences
- The IDE experience requires documented manual configuration during the MVP.
- It is documented that `packageSourceMapping` does not guarantee absolute id confidentiality, and that provenance tests use clean caches.

### Review when
The pilot identifies IDE use as the main friction.

---

<a id="adr-017"></a>
## ADR-017 — Consistent backup in maintenance mode

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 7 · **Related:** ADR-004, ADR-005, ADR-006

### Context
A database backup without blobs does not recover the registry. Copying the active SQLite file without coordination is not reliable.

### Decision
- `onepackd backup` activates maintenance mode: rejects mutations (`503` with `Retry-After`) and pauses blob cleanup; reads continue.
- Copies the database with the **SQLite Online Backup API**, the referenced blobs and the necessary configuration (without TLS secrets unless explicitly indicated).
- Generates a manifest with hashes and schema version.
- `onepackd restore` requires an empty directory and verifies the manifest.
- **Success criterion:** restore on another server and have a project run `dotnet restore` successfully.

### Alternatives considered
- **Hot backup without pause** — risk of blobs collected between the DB copy and the blob copy.
- **Delegate to filesystem snapshots** — valid as a complement, not as the only portable mechanism.

**Clarification (Phase 7, 2026-10-04).** The database is copied with `VACUUM INTO` and not with the Online Backup API: sqlx does not expose that API and calling it directly would require `unsafe` code, which the workspace forbids. `VACUUM INTO` also produces a consistent copy (a read transaction) and compacts. The blobs that are copied are those referenced by that copy, verified against their hash. Maintenance mode lives in the database (shared by the backup process and the server) and expires on its own, so an interrupted backup does not leave the registry locked. Service configuration (flags and environment variables) is not in the data directory and is not included in the backup; it is documented that it must be stored separately. `restore` applies migrations if the backup is from an older version. Detail in [docs/runbooks/backup-restore.md](../docs/runbooks/backup-restore.md).

### Consequences
- Short window without publications during the backup; documented.
- Incremental and hot backup are left for evolution.

### Review when
Data size makes the maintenance window unacceptable.

---

<a id="adr-018"></a>
## ADR-018 — Explicit `public_url` and blobs only through the server

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 3, 7 · **Related:** ADR-005, ADR-009, ADR-011

### Context
Behind a reverse proxy, deriving URLs from the `Host` header allows URL poisoning. Serving blobs directly from the proxy would bypass authorization.

### Decision
- `server.public_url` is mandatory and is the only base for absolute URLs of the service index and resources.
- TLS directly or terminated at the reverse proxy; the server listens on `127.0.0.1` by default.
- Blobs are served **only** by the server, after authorization. The Nginx/Caddy examples do not expose the data directory.

### Alternatives considered
- **Derive from `Host`/`X-Forwarded-*`** — manipulable.
- **`X-Accel-Redirect`/`X-Sendfile`** — valid future optimization if prior authorization is maintained; not in the MVP.

### Consequences
- A domain change requires changing configuration and restarting.

### Review when
Large downloads saturate the process and byte serving needs to be delegated.

---

<a id="adr-019"></a>
## ADR-019 — Conformance proven with real .NET clients, without .NET at runtime

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-03 · **Phase:** 0, 1 · **Related:** ADR-002, ADR-008, ADR-009

### Context
Testing only with one's own CLI can hide incompatibilities shared between client and server.

### Decision
- `tests/conformance-dotnet` uses `dotnet` SDK, `nuget.exe` and small .NET programs (e.g. with `NuGet.Versioning`) as a **test reference**.
- Test packages are created with official tools (`dotnet pack`).
- Restore tests use isolated caches (temporary `NUGET_PACKAGES`).
- .NET **is not** a dependency of the deployed server.

### Alternatives considered
- **Static response fixtures** — do not detect client behavior regressions.

### Consequences
- CI needs the .NET SDK and, for `nuget.exe`, a Windows runner.
- A matrix of tested clients with exact versions is maintained.

### Review when
Another format (npm, PyPI) is added, which will require its own conformance suite.

---

<a id="adr-020"></a>
## ADR-020 — Forward-only migrations with a prior backup

**Status:** ✅ Accepted · **Date:** 2026-10-03 · **Accepted:** 2026-10-04 · **Phase:** 2, 7 · **Related:** ADR-004, ADR-017

### Context
Going back to the previous binary does not revert a data migration. "Down" migrations are rarely tested.

### Decision
- SQLx migrations embedded in `onepackd`, numbered and **forward only**.
- On startup: check the schema version; if there are pending migrations, require `onepackd migrate` (or an explicit flag), which generates a prior backup.
- A binary refuses to start with a schema newer than the one it knows.
- Rollback consists of restoring the prior backup with the previous binary.

### Alternatives considered
- **Silent automatic migrations on startup** — surprises during upgrades.
- **Reversible migrations** — maintenance cost with no real guarantee.

### Consequences
- Release notes indicate whether a release includes a migration.
- N-1 → N upgrade test in CI.

### Review when
PostgreSQL or multinode deployments are adopted.
