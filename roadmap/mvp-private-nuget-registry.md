# Private package registry — NuGet MVP

> Product proposal: scope, architecture, technology stack, security, CLI, operations and evolution.

## Executive summary

The idea makes sense, but I would focus it as a very specific product:

> **A private, self-hosted package registry administered from a CLI, starting with NuGet compatibility and able to expand to other ecosystems later.**

I would not try to create a replacement for `dotnet` or a new dependency resolver. The product would store, protect and distribute packages; developers would keep using their usual tools. NuGet already defines a server protocol that allows that separation. [Reference: NuGet service index](https://learn.microsoft.com/en-us/nuget/api/service-index).

**My initial technical proposal would be: Rust + Tokio + Axum + SQLite + local storage, with server and CLI in the same workspace.** Without a web interface, without Kubernetes and without mandatory external services.

The important thing is that "lightweight" does not mean "incomplete": authentication, consistent publication, permissions and recovery must be part of the MVP.

---

## 1. Product definition

### What it would be

A service that a company installs on its infrastructure to publish and consume its private libraries.

For example, at Hemia you could have:

```text
Private registry
│
├── Feed: internal
│   ├── Hemia.Logging
│   ├── Hemia.Authentication
│   └── Hemia.Observability
│
├── Feed: customer-a
│   ├── CustomerA.Contracts
│   └── CustomerA.Integrations
│
└── Feed: experimental
    └── Hemia.Agents.Preview
```

Each **feed** would be a logical repository with its own packages, configuration and permissions.

A published version would have an unambiguous identity within the feed:

```text
feed + package_id + normalized_version
```

The system should allow a developer to publish a library, a pipeline to restore it and an administrator to answer:

> Who published this version, when, with what credential and what is the exact file we are distributing?

### Who I would target it at

As an initial hypothesis, I would focus it on small and medium-sized teams that need private packages but want to administer the service without a complex platform.

It also fits consultancies that separate libraries by client and organizations that need to keep their artifacts within their infrastructure.

I would not start by trying to cover large multinode installations or companies with advanced identity federation requirements.

### Where the differentiation would be

The category already has alternatives: **BaGet and BaGetter are lightweight NuGet servers**, and BaGetter documents cross-platform execution, ARM64 support and mirror capabilities. Therefore, "lightweight self-hosted NuGet" is not enough as a differentiator. References: [BaGet](https://github.com/loic-sharma/BaGet) and [BaGetter](https://github.com/bagetter/BaGetter).

The proposal I would validate would be:

> **A private registry that is easy to install and fully operated from the terminal, with scoped credentials, immutable versions and predictable recovery.**

The CLI would have to solve real operational problems, not just wrap HTTP calls.

---

## 2. Recommended MVP scope

To keep it buildable, I would assume **one installation for one organization, with several private feeds and a single server node**.

| Area | Would include in the MVP |
|---|---|
| Compatibility | NuGet V3 publication, restoration, search and metadata. |
| Feeds | Create, query and configure several private feeds. |
| Identities | Administrators, users and service accounts for pipelines. |
| Credentials | Tokens with expiration, revocation and scoped permissions. |
| Authorization | Roles per feed and optional publication restrictions by prefix. |
| Packages | Immutable versions; list, query, download, hide and relist. |
| Incidents | Administrative blocking of a version. |
| Storage | SQLite for metadata and local disk for artifacts. |
| Operations | Diagnostics, auditing, integrity checking and backup/restore. |
| Distribution | Server and CLI binaries; optional container image. |

**I would initially leave out:** web interface, nuget.org proxy, virtual feeds, S3, PostgreSQL, high availability, SSO, npm/PyPI support, symbol server and antimalware analysis.

They are not bad features. The problem is that each one opens a different front in compatibility, security or operations.

Especially important: **the MVP would host private packages; it would not yet be a universal dependency proxy**. Projects could use the private feed and nuget.org simultaneously, with an explicit provenance configuration.

---

## 3. What "NuGet compatible" really means

It is not enough to implement one endpoint to upload `.nupkg` files and another to download them.

The client discovers the server resources through a **service index**. Those resources have independent contracts and versions. [Reference: service index](https://learn.microsoft.com/en-us/nuget/api/service-index).

### Resources I would implement

The following routes are my own proposal; the resource names correspond to the protocol:

| Resource | Function | Illustrative route |
|---|---|---|
| Service index | Discover capabilities and endpoints. | `/nuget/{feed}/v3/index.json` |
| `PackageBaseAddress/3.0.0` | Query versions and download `.nupkg` and `.nuspec`. | `/nuget/{feed}/v3/flat/` |
| `RegistrationsBaseUrl/3.6.0` | Expose metadata, dependencies and versions. | `/nuget/{feed}/v3/registration/` |
| `SearchQueryService` | Search packages and browse results. | `/nuget/{feed}/v3/query` |
| `SearchAutocompleteService` | Complete identifiers and query versions. | `/nuget/{feed}/v3/autocomplete` |
| `PackagePublish/2.0.0` | Publish, hide and relist versions. | `/nuget/{feed}/v2/package` |

These capabilities are documented in the official resources for content, registrations, search and autocomplete. [Reference: package content](https://learn.microsoft.com/en-us/nuget/api/package-base-address-resource).

**The `2.0.0` in `PackagePublish` does not imply implementing the entire old NuGet API.** It is the publish resource also used from the V3 API. Its publication uses `PUT`, `multipart/form-data` content and the `X-NuGet-ApiKey` header. [Reference: package publishing](https://learn.microsoft.com/en-us/nuget/api/package-publish-resource).

### The details I would not simplify

**Identity and version normalization.** NuGet has particularities that a generic SemVer library does not automatically cover. For example, it supports a fourth numeric segment and considers certain representations such as `1.0` and `1.0.0` equivalent. These rules must also be applied when detecting duplicates. [Reference: package versioning](https://learn.microsoft.com/en-us/nuget/concepts/package-versioning).

**Dependency metadata.** Framework groups and version ranges must be preserved correctly. The server exposes that information; it does not need to resolve the project's dependency graph. [Reference: package registrations](https://learn.microsoft.com/en-us/nuget/api/registration-base-url-resource).

**Search behavior.** Pagination, prerelease versions and `semVerLevel` must be respected. I would not announce a resource version without implementing its requirements. [Reference: package search](https://learn.microsoft.com/en-us/nuget/api/search-query-service-resource).

**Package states.** Hiding a version is not blocking it: NuGet allows unlisted packages to remain available for download. In particular, the flat container includes listed and unlisted versions. [Reference: package content](https://learn.microsoft.com/en-us/nuget/api/package-base-address-resource).

My criterion would be to publish a **matrix of tested clients**, not promise "full compatibility" without tests.

---

## 4. Stack: what I would choose and why

### Main choice: Rust

For the product direction — self-hosted, few components, native CLI and resource control — I would choose Rust.

Its ownership model allows managing memory without a garbage collector; that is a useful characteristic for this architecture, not an automatic guarantee of higher performance or security for the whole product. [Reference: ownership in Rust](https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html).

| Component | Proposed choice |
|---|---|
| Server language | Rust. |
| Asynchronous runtime | Tokio. |
| HTTP and routing | Axum. |
| Middleware | Tower / tower-http. |
| Persistence | SQLite with WAL. |
| Queries and migrations | SQLx. |
| Artifacts | Local filesystem through a small abstraction. |
| CLI | Rust + clap. |
| CLI HTTP client | reqwest. |
| Serialization and configuration | serde, JSON and TOML. |
| Package inspection | ZIP and XML libraries with explicit limits. |
| Observability | Structured logs, metrics and request IDs. |
| Secure transport | Direct HTTPS or TLS termination at a reverse proxy. |

Axum is designed to work with Tokio and integrates with Tower; SQLx offers support for SQLite and embedded migrations; clap provides the basis for defining commands and arguments. References: [Axum](https://docs.rs/axum/latest/axum/), [SQLx](https://docs.rs/sqlx/latest/sqlx/) and [clap](https://docs.rs/clap/latest/clap/).

I would not introduce Redis, Elasticsearch or an external queue in this version.

### The serious alternative: C# / ASP.NET Core

**C# would be a very defensible choice when the main priority is to reduce the risk of implementing NuGet.**

The official SDK offers `NuGet.Packaging`, `NuGet.Versioning`, `NuGet.Frameworks` and `NuGet.Protocol`, among other libraries. Reusing them avoids redoing part of the ecosystem rules. [Reference: NuGet Client SDK](https://learn.microsoft.com/en-us/nuget/reference/nuget-client-sdk).

The decision, for me, would be:

| Dominant priority | Choice |
|---|---|
| Native product, central CLI and future multiformat expansion | Rust. |
| Reduce NuGet-specific work and leverage .NET experience | C# / ASP.NET Core. |
| Team much more productive in Go | Go would be viable, but would still require NuGet-specific compatibility work. |

**I would keep Rust as the recommendation for your idea**, with one condition: first build a NuGet compatibility test. Do not discover after developing administration, permissions and branding that the adapter behaves differently from real clients.

I would use small .NET programs as a reference in the tests, **without turning .NET into a dependency of the deployed server**.

---

## 5. Architecture: modular monolith

I would not split it into microservices.

I would propose a server process with two HTTP surfaces: one compatible with NuGet and one administrative for the CLI.

```text
dotnet / NuGet clients                  Administrative CLI
           │                                   │
           ▼                                   ▼
   NuGet-compatible API                 Management API /api/v1
           │                                   │
           └────────────────┬──────────────────┘
                            ▼
                 Authentication and permissions
                            │
                            ▼
                  Domain use cases
             ┌──────────────┼──────────────┐
             ▼              ▼              ▼
           Feeds         Packages       Identities
             │              │              │
             └──────────────┼──────────────┘
                            ▼
                   Persistence and auditing
                    │                  │
                    ▼                  ▼
                  SQLite           Blob storage
                                      │
                                      ▼
                                Local disk
```

The server would have bounded internal tasks to clean temporary files, check incomplete operations and manage maintenance. It would not need an independent worker.

### Important internal separation

**Domain:** what it means to publish a version, who can do it and when it is available.

**NuGet adapter:** how to read the package and translate those operations to the protocol.

**Persistence:** how to store metadata and bytes.

**Administrative API:** how to expose operations to the CLI.

This allows adding another format without turning the entire domain into a set of NuGet rules.

A reasonable structure:

```text
private-registry/
├── crates/
│   ├── core/
│   ├── nuget/
│   ├── storage/
│   ├── api-client/
│   ├── server/
│   └── cli/
├── migrations/
├── tests/
│   ├── integration/
│   ├── conformance-dotnet/
│   ├── security/
│   └── recovery/
├── packaging/
│   ├── systemd/
│   └── container/
└── docs/
```

I would not create a dynamic plugin system yet. Modules and compiled adapters would be enough.

---

## 6. Persistence and consistent publication

### SQLite for metadata

For this MVP, SQLite avoids requiring another installation and keeps operation on a single node.

The limitation must be explicit: in WAL there can be concurrent readers, but only one writer at a time; moreover, WAL is not intended for sharing the database over a network filesystem between machines. [Reference: SQLite WAL](https://www.sqlite.org/wal.html).

Therefore:

> **One active server, local disk and short transactions. Not several replicas sharing the same SQLite file.**

I would propose `synchronous=FULL` as the initial durability-oriented configuration, evaluating its cost in the tests. SQLite documents that `NORMAL` and `FULL` have relevant differences on power loss. [Reference: SQLite WAL](https://www.sqlite.org/wal.html).

### Files outside the database

I would store the original `.nupkg` as an immutable object identified by its hash:

```text
/var/lib/pkgd/
├── metadata.sqlite
├── blobs/
│   └── sha256/
│       └── ab/
│           └── cd/
│               └── <hash>
└── staging/
```

The hash would serve integrity and possible deduplication. **It would never be a credential or allow downloading while bypassing feed permissions.**

I would preserve exactly the bytes received. I would not decompress and repackage the artifact to store it.

### Initial data model

| Entity | Main information |
|---|---|
| Feed | Name, format, quotas and configuration. |
| Principal | User or service account. |
| Token | Secret verifier, owner, expiration and revocation. |
| Grant | Actions allowed on a feed and publication restrictions. |
| Package | Original and normalized identifier, associated with the feed. |
| PackageVersion | Version, metadata, publishing author and state. |
| Blob | Hash, size and physical location. |
| AuditEvent | Actor, action, resource, date and result. |

The uniqueness constraint must exist in the database, not only in code:

```text
UNIQUE(feed_id, normalized_package_id, normalized_version)
```

### Publication flow

```text
Authenticate
   ↓
Check permission and quota
   ↓
Receive by streaming into staging
   ↓
Compute hash and inspect package with limits
   ↓
Validate identity, version and metadata
   ↓
Persist the blob durably
   ↓
Confirm metadata + auditing in a transaction
   ↓
Make the version visible and respond
```

**The database and the filesystem do not form a single transaction.** I would design recovery around that reality.

If the process crashes after storing the blob but before committing the database, a recoverable or deletable orphan file may remain. What I would avoid is committing a version that does not yet have a durable artifact.

I would not keep a SQLite transaction open during the entire upload.

### Immutable versions

A published version is not replaced. To correct content, another version is published.

The NuGet endpoint would return `409` for an already existing identity, behavior provided by the protocol. [Reference: package publishing](https://learn.microsoft.com/en-us/nuget/api/package-publish-resource).

The CLI could offer `--skip-existing-identical`, but would only consider a duplicate successful after verifying that the content matches. A conflict with different bytes must still be an error.

---

## 7. Security from the first version

Security would have to cover both **who accesses** and **what happens when it receives a malicious or defective package**.

### Authentication: three distinct contexts

| Context | Proposed mechanism |
|---|---|
| Administrative API | Token via `Authorization: Bearer`. |
| NuGet download and restore | Basic authentication over HTTPS, with token as password. |
| NuGet publish | `X-NuGet-ApiKey`, with publish permissions. |

NuGet clients can look for credentials after a `401` response, through configuration, environment variables or a credential provider. Protecting only the publish endpoint is not enough. [Reference: authenticated feeds](https://learn.microsoft.com/en-us/nuget/consume-packages/consuming-packages-authenticated-feeds).

I would not use a single global API key for the entire installation.

### Tokens and permissions

I would propose high-entropy random tokens, shown only once, with expiration and revocation. The server would store a verifier of the secret, not the recoverable token.

For this MVP I would choose opaque tokens verifiable against the database, instead of long-lived JWTs.

The initial roles could be:

| Role | Permissions |
|---|---|
| Reader | Search, query and download. |
| Publisher | Read and publish new versions. |
| Maintainer | Manage feed visibility and blocks. |
| Administrator | Administer identities, permissions and configuration. |

A CI account should be able to publish only to its feed and, when applicable, only certain prefixes:

```text
Principal: ci-payments
Feed: internal
Actions: read, publish
Allowed prefix: Hemia.Payments.*
```

That prefix would be a registry policy, not a special namespace of the NuGet protocol.

Initial startup would generate a local administrative credential under restrictive permissions. I would not expose a public registry of administrators or use a default password.

### Endpoint protection

Authorization must also run for searches, metadata, `.nuspec`, `HEAD`, conditional responses and downloads. Not only for the main file.

I would apply HTTPS, request limits, quotas and secret redaction in logs. These are controls aligned with OWASP API security recommendations. [Reference: OWASP REST Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html).

I would not allow Nginx to serve the blob directory directly through a public path that bypasses authorization.

### Package validation

I would treat each `.nupkg` as untrusted content. I would set limits for compressed size, ZIP entries, XML metadata, inspection time and resources consumed.

I would not execute scripts, assemblies or tools included in the package. Nor would I automatically download resources indicated by its metadata.

OWASP expressly identifies risks such as ZIP bombs, XML bombs, parser vulnerabilities and file overwrite during uploads. [Reference: OWASP File Upload Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/File_Upload_Cheat_Sheet.html).

Inspection should avoid indiscriminate extraction, reject dangerous paths and work with limited concurrency.

**Checking a hash does not prove that a package is benign.** The MVP should not advertise antimalware analysis or signature trust validation if it only stores the bytes.

### Hiding is not blocking

I would use two separate concepts:

```text
listed: true | false
availability: available | blocked
```

`listed=false` removes it from discovery; `blocked` prevents new downloads from the server. This separation avoids confusing NuGet unlist with an incident response. [Reference: package publishing and listing](https://learn.microsoft.com/en-us/nuget/api/package-publish-resource).

No registry block can delete copies a client has already downloaded. That limitation must be documented.

---

## 8. The CLI would be the product's main interface

I will use `pkgd` and `pkgctl` as illustrative names, not as a validated brand.

**`pkgd` would be the server; `pkgctl`, the administrative client.** They would share contracts, but installing the CLI would not force deploying the server.

### Proposed experience

```bash
# Local server initialization
pkgd init --data-dir /var/lib/pkgd
pkgd serve --config /etc/pkgd/config.toml

# Connection from a workstation
pkgctl context add hemia \
  --url https://packages.hemia.example

pkgctl login --context hemia

# Administration
pkgctl feed create internal --format nuget
pkgctl principal create ci-payments --type service

# Prepare a project, showing changes first
pkgctl nuget init \
  --feed internal \
  --source-name Hemia \
  --prefix "Hemia.*" \
  --dry-run

# Daily operation
pkgctl package list --feed internal
pkgctl package inspect Hemia.Logging@1.2.0 --feed internal
pkgctl package push ./Hemia.Logging.1.2.0.nupkg --feed internal

# Diagnostics
pkgctl doctor --feed internal
```

All are proposed commands for the product, not already implemented tools.

### What it must do well

**Automation.** Stable `--json` output, documented exit codes, `--no-input`, timeouts and pagination. Data would go to stdout; progress and diagnostics, to stderr.

**Interaction.** Autocompletion, local help, confirmations for destructive operations and errors that indicate a concrete action.

**Credentials.** No-echo or stdin input; storage through the system credential manager when available. Do not silently fall back to a plain-text file.

**Resilience.** Correct cancellation, limited retries and no blind automatic repetition of non-idempotent operations.

**Compatibility.** Capability negotiation between CLI and server, without requiring both to always be updated at the same time.

### `doctor` as a differentiating function

It should check server resolution, TLS, authentication, permissions, service index and NuGet configuration.

A useful error would be:

```text
AUTH_SCOPE_MISSING

The credential is valid, but it does not allow publishing to "internal".

Required action: packages:publish
Principal: ci-payments
Request ID: req_...
```

I would not display the secret or dump all environment variables for diagnosis.

### The delicate point: CLI login and NuGet login

Storing a token in the CLI **does not make `dotnet restore` able to read it automatically**.

In the MVP I would propose two paths:

**CI:** credentials injected through mechanisms compatible with NuGet.

**Local development:** a command that runs the standard client with credentials available only in the child process environment:

```bash
pkgctl exec --feed internal -- dotnet restore
```

The resolver would still be `dotnet`; the CLI would only prepare its authentication.

NuGet supports credentials through `NuGetPackageSourceCredentials_{name}`. In addition, password encryption in `NuGet.Config` has platform limitations: it is supported on Windows and tied to user and machine. [Reference: authenticated feeds](https://learn.microsoft.com/en-us/nuget/consume-packages/consuming-packages-authenticated-feeds).

A custom **credential provider** would be a later evolution to improve the experience with IDEs. I would not promise that storing credentials in the CLI also solves that integration. [Reference: NuGet cross-platform authentication plugin](https://learn.microsoft.com/en-us/nuget/reference/extensibility/nuget-cross-platform-authentication-plugin).

---

## 9. Public and private packages: secure configuration

The `pkgctl nuget init` wizard should generate or propose a project configuration without secrets, preserving what exists and showing the changes before applying them.

An example:

```xml
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add
      key="Hemia"
      value="https://packages.hemia.example/nuget/internal/v3/index.json"
      protocolVersion="3" />
    <add
      key="nuget.org"
      value="https://api.nuget.org/v3/index.json"
      protocolVersion="3" />
  </packageSources>

  <packageSourceMapping>
    <clear />
    <packageSource key="Hemia">
      <package pattern="Hemia.*" />
    </packageSource>
    <packageSource key="nuget.org">
      <package pattern="*" />
    </packageSource>
  </packageSourceMapping>
</configuration>
```

The most specific pattern takes precedence: here `Hemia.*` packages are assigned to the private feed. Transitive dependencies must also be covered. I would not rely on feed order to achieve this. [Reference: Package Source Mapping](https://learn.microsoft.com/en-us/nuget/consume-packages/package-source-mapping).

There are two important limits. Packages already present in the global cache can avoid a new query to the source; therefore provenance tests must use clean or isolated caches. [Reference: Package Source Mapping](https://learn.microsoft.com/en-us/nuget/consume-packages/package-source-mapping).

In addition, the mapping **does not guarantee absolute confidentiality of identifiers**: certain metadata queries may be directed to all configured sources. For strict isolation requirements, sources and network egress would have to be restricted, not rely solely on mapping. [Reference: NuGet configuration](https://learn.microsoft.com/en-us/nuget/reference/nuget-config-file).

---

## 10. Deployment, maintenance and recovery

### Initial deployment

I would propose a Linux server on x86-64 and ARM64, with a CLI for Linux, macOS and Windows. Windows server support could be added after validating demand.

The minimum installation would be:

```text
1 server binary
1 configuration file
1 data directory
1 system service
```

Example proposed configuration:

```toml
[server]
listen = "127.0.0.1:8080"
public_url = "https://packages.hemia.example"

[database]
path = "/var/lib/pkgd/metadata.sqlite"

[storage]
driver = "filesystem"
path = "/var/lib/pkgd/blobs"

[security]
anonymous_read = false

[limits]
max_package_size_mib = 100
max_concurrent_uploads = 4
```

Limits would be configurable; not universal NuGet restrictions.

`public_url` would be explicit to correctly generate URLs behind the reverse proxy. I would not derive it from any received `Host` header.

Docker would be another presentation of the same product, not a requirement.

### Backup and restore within the MVP

I would start with **consistent backup in maintenance mode**, pausing mutations and blob cleanup during the operation.

The backup must contain metadata, referenced blobs, necessary configuration and an integrity manifest. A backup of the database without the artifacts does not recover the registry.

SQLite offers specific backup mechanisms; copying only the main file of an active database without coordination is not a sufficient procedure for this design. [Reference: SQLite Online Backup API](https://www.sqlite.org/backup.html).

Updates would include schema version, pre-checks and controlled migrations. I would not assume that switching to the previous binary automatically reverts a data migration.

**The recovery test ends when restoring a project from a rebuilt installation**, not when verifying that a backup file was generated.

---

## 11. Performance: measurable goals

I would not sell "fast because it is written in Rust". I would define a resource budget and an evaluation scenario.

For example, for a test environment of **2 vCPU, 1 GB of RAM and local SSD**, I would propose these initial goals, still pending measurement:

| Indicator | Design goal |
|---|---|
| Server startup | Less than 2 seconds with a small database. |
| Idle memory | Less than 100 MiB of RSS. |
| Authenticated metadata query | p95 below 50 ms on local network and warm cache. |
| Transfers | Bounded memory, not proportional to the full package size. |
| Concurrency | Controlled degradation under saturation, without unlimited task growth. |
| Publication | Available for reading when the operation is confirmed. |

They are goals, not results or guaranteed minimums.

To achieve them I would prioritize streaming, adequate indexes, pagination, bounded caches and concurrency limits. ZIP/XML inspection would have its own budget so as not to block download service.

I would avoid turning each download into several SQLite writes. Important mutations would have durable auditing; access counters could be aggregated without blocking each read.

---

## 12. Construction plan and acceptance criteria

I would develop it through verifiable milestones, not starting with a huge generic platform.

| Milestone | What it must demonstrate |
|---|---|
| Compatibility | A package created with official tools is published and restored through real clients. |
| Persistence | The version preserves exactly its bytes and survives restarts and interruptions. |
| Security | Isolated feeds, revocable tokens and permissions checked on all endpoints. |
| CLI | Configuration, publication, query and diagnosis without a web interface. |
| Operations | Backup, restoration, upgrade and recovery tested. |
| Pilot | A team uses it in its usual builds and publications. |

Test packages should cover different versions, frameworks, dependencies and metadata. Official creation tools must be the reference; Microsoft recommends using them to produce well-formed packages. [Reference: NuGet Client SDK](https://learn.microsoft.com/en-us/nuget/reference/nuget-client-sdk).

### Tests I would consider blocking

| Scenario | Expected result |
|---|---|
| Two simultaneous publications of the same identity | Only one confirmed version, without overwrite. |
| Crash during an upload | No partially available version appears. |
| Disk full | Controlled error, with no metadata pointing to a nonexistent blob. |
| Revoked token | New requests stop being authorized. |
| Cross-feed read without permission | No leak through search, metadata or content. |
| ZIP or XML designed to exhaust resources | Controlled rejection within the limits. |
| Unlisted version | Not discovered in search, but still available according to its state. |
| Backup restore on another server | The project restores its dependencies again. |

I would also test proxies, certificates, inherited NuGet configuration and empty caches. Testing only with my own CLI would be insufficient: it could hide incompatibilities shared between client and server.

---

## 13. Product evolution

After stabilizing the core, I would follow this order:

| Stage | Expansion |
|---|---|
| First evolution | Credential provider, package import and S3-compatible storage. |
| Second evolution | NuGet proxy/cache with explicit provenance rules. |
| Third evolution | npm, preserving its own identity and protocol rules. |
| As demanded | Symbols, PyPI, SSO, web interface and enterprise features. |
| Only with demonstrated need | PostgreSQL, multiple nodes and high availability. |

The proxy deserves to be treated as a security project of its own: it introduces outbound access, upstream credentials, identity collisions and cache policies. I would not implement it as "if it does not exist locally, download from any URL".

I would not assume that all formats share exactly the NuGet rules. I would share storage, permissions and auditing; I would keep validation and normalization in each adapter.

For a possible commercial model, I would leave **the self-hosted core, essential permissions and recovery fully usable**, and evaluate monetizing managed service, support, enterprise integration and advanced operations. I would first validate that teams really choose this experience over existing alternatives.

---

## Final recommendation

I would build a **single-node private NuGet registry**, with:

**Rust server, Rust CLI, SQLite, local disk, mandatory authentication, per-feed permissions, immutable versions and built-in recovery.**

I would not start with S3, proxy, microservices or a web interface. The largest initial investment should go to **NuGet client compatibility, credential management and publication consistency**.

The decisive test of the MVP would be this:

> A team installs the server, configures its project, publishes a library and restores it from CI; then revokes a credential and recovers the service from a backup, without opening a web interface or manually editing the database.

**That would be a small MVP in infrastructure, but complete in its function.**
