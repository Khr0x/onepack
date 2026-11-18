# npm conformance specification — private hosting v2 (OP-023)

> Task OP-023. Depends on OP-020 and OP-021 (verified). Delivers the contracts that OP-024
> (adapter), OP-025 (CLI) and the client documentation need; implements nothing.
>
> Sources: `roadmap/adr-v2.md` (ADR-021…026), `docs/security.md`, `docs/compatibility.md`,
> `docs/api.md` and the code in `crates/server/src` and `crates/core/src`.
>
> Baseline consulted: `88467a94eaee4fc7345bac54fdf6fa4449a913b8`.
> Date: 2026-10-19. Status: **accepted**. Review on 2026-10-19 by the maintainer
> (instruction in the work session); matrix, contracts and first-cut decision
> accepted without scope changes. Implementation corresponds to OP-024 and OP-025.

## 1. Scope

Enable **publish, install and ci** with real npm against a private `npm` feed, with scoped and
unscoped packages, dist-tags, token authentication, inspection limits and stable errors,
reusing the existing roles, quotas, auditing and storage.

**Out of scope:** npmjs proxy or mirror; pnpm, yarn and any client not listed in the
matrix (not announced); `npm login`; `npm unpublish`; `npm deprecate`; `npm search`; running
Node.js on the server; dynamic plugins. All of this is documented as a limitation with its
review condition in section 10.

## 2. Initial conformance matrix (AC-01)

Candidate clients; the exact patch is fixed when preparing OP-024 (AC-01) and a row is only
published as verified if it passes the E2E in section 11.

| Client | Node | Role in the matrix |
|---|---|---|
| npm 10.x | Node 22 LTS | compatibility with the previous LTS |
| npm 11.x | Node 24 LTS | main line |

When preparing OP-024, patch versions are fixed and the current LTS is added if it already
exists; an npm client that has not passed the E2E in CI is not announced. pnpm and yarn remain
explicitly unverified (REQ-03) and `capabilities` announces only the `npm` format, not clients.

| # | Flow | Example | What it checks |
|---|---|---|---|
| F1 | Unscoped publish | `npm publish` in `widgets` | Acceptance, transaction, `latest` |
| F2 | Scoped publish | `npm publish` in `@acme/widgets` | Namespace, grant prefix, scoped tarball |
| F3 | Fresh install | `npm install widgets --registry ...` | Abbreviated packument + tarball |
| F4 | CI with lockfile | `npm ci` with `package-lock.json` | `resolved` + `integrity` (sha512) |
| F5 | Query | `npm view @acme/widgets`, `... versions`, `... dist-tags` | Full packument and version routes |
| F6 | Dist-tags | `npm dist-tag add @acme/widgets@1.2.3 next`, `npm dist-tag rm ...` | Add, move and delete |
| F7 | Adversarial auth | no token, revoked token, reader publishing, foreign prefix | 401/403/404 according to section 4 |
| F8 | Isolation | token with a grant only on feed A against feed B | 404 without disclosure |
| F9 | Blocked version | `npm install` of a blocked version | Tarball `410 PACKAGE_BLOCKED` |
| F10 | Integrity | `npm ci` with correct integrity; tampered lockfile | Client-side failure, not registry failure |
| F11 | Transitive dependency | `app` declares `dependencies: {"lib": "^1.0.0"}`; `npm install app` | `lib` resolution from the same feed |

## 3. HTTP surface

Base: `{public_url}/npm/{feed}` (ADR-018). The feed format must be `npm`; if it is of another
format, the route responds `404` as `FEED_NOT_FOUND` (ADR-024).

| Method | Route | Operation |
|---|---|---|
| GET | `/npm/{feed}/{name}` | Full or abbreviated packument according to `Accept` |
| GET | `/npm/{feed}/{name}/{version}` | Manifest of a specific version |
| GET, HEAD | `/npm/{feed}/{name}/-/{file}.tgz` | Tarball |
| PUT | `/npm/{feed}/{name}` | Publication |
| GET | `/npm/{feed}/-/package/{name}/dist-tags` | List dist-tags |
| PUT | `/npm/{feed}/-/package/{name}/dist-tags/{tag}` | Create or move a tag |
| DELETE | `/npm/{feed}/-/package/{name}/dist-tags/{tag}` | Delete a tag |
| GET | `/npm/{feed}/-/whoami` | Token identity |
| GET | `/npm/{feed}/-/ping` | Probe |
| — | `/npm/{feed}/-/v1/search` | Not registered: `npm search` receives `404` |
| — | Login | Not implemented: the credential is configured in `.npmrc` |

Route details:

- `{name}` accepts `@scope/name` and `name`. The percent-encoded forms `%40` and
  `%2F` are accepted; decoding happens once and double encoding, empty segments, `..`,
  null bytes and control characters are rejected.
- The parser is a per-method catch-all; static `/-/...` routes take priority. The
  concrete implementation (Axum pattern and extractors) is fixed in OP-024 with tests for both
  scoped forms.
- No response compression in the first cut (`identity`), like the NuGet routes.

## 4. Authentication and authorization (AC-02)

The npm surface accepts **only** `Authorization: Bearer <token>` (npm's mechanism); it does not
accept `X-NuGet-ApiKey` or `Basic`. The NuGet and administrative surfaces do not change. This
implies adding `Surface::Npm` to the route classification, to `credential()` and to error
rendering, and the `npm` label in metrics (ADR-024).

All requests remain authenticated; there is no anonymous read in this cut. The credential is
configured like this:

```ini
; .npmrc
@acme:registry=https://packages.example.com/npm/internal
//packages.example.com/npm/internal/:_authToken=${ONEPACK_TOKEN}
```

```bash
onepack token create --principal ci-web --name npm-publish
```

| Case | Result |
|---|---|
| Missing, expired, revoked or malformed token | `401 AUTH_REQUIRED` + `WWW-Authenticate: Bearer realm="onepack"` |
| Valid token without a grant on the feed | `404 FEED_NOT_FOUND` (does not reveal that the feed or package exists; ADR-012) |
| `reader` tries to publish or move tags | `403 AUTH_SCOPE_MISSING` |
| `publisher` publishes an id outside its prefixes | `403 AUTH_PREFIX_DENIED` |
| `reader`/`publisher`/`maintainer` read | Allowed according to cumulative role |
| `maintainer` or admin | Publication, tags and blocking via the administrative API |

Additional rules:

- Read: `Access::Read` on the feed. Packument, manifest and tarball share the decision.
- Publication: `Access::Publish` + grant pattern on `package_key` (ADR-022);
  `@acme/*` covers `@acme/widgets`.
- Dist-tags: `Access::Publish` + package pattern; moving or creating a tag is a publication
  operation, and deleting it is too. `latest` cannot be deleted.
- `/-/whoami` and `/-/ping` require a valid token, without requiring a grant: they do not expose
  feed data.
- Revocation takes effect on the next request; npm may reuse cached credentials, so the
  documentation will remind users to refresh `.npmrc` after revoking.
- TLS: npm refuses to send credentials over HTTP except to localhost. Deployment requires HTTPS at
  the reverse proxy and an HTTPS `public_url`; the server does not trust `X-Forwarded-For` (ADR-018).

## 5. Identity, versions and conflicts

Adapter rules (ADR-022); the implementation will reproduce the rules of
`validate-npm-package-name` and SemVer with a conformance corpus.

| Rule | Detail |
|---|---|
| Unscoped name | Lowercase, ≤ 214 characters, no spaces or unsafe characters, not starting with `.` or `_` |
| Scoped name | `@scope/name`; scope and name with the same rules |
| `package_key` | Full name in lowercase, including `@scope/` |
| Version | Strict SemVer 2.0.0; `pre` and `build` according to SemVer |
| Version identity (`version_key`) | Without build metadata; `full_version` preserves the published value |
| Dist-tags | They are not versions and do not create identity; a tag cannot be SemVer or a valid range |
| Ordering | SemVer precedence, in the adapter |

| Publication | Key | Result |
|---|---|---|
| `@acme/widgets` `1.2.3-beta.1+build.5` | `@acme/widgets` / `1.2.3-beta.1` | Accepted; `full_version` preserves `+build.5` |
| `@acme/widgets` `1.2.3` after the previous one | `1.2.3` | Different identity: accepted |
| `Widgets` | — | Rejected: npm requires lowercase |
| `@acme/widgets` `1.2.3` repeated, same bytes | `1.2.3` | `409 VERSION_EXISTS` |
| `@acme/widgets` `1.2.3` repeated, other bytes | `1.2.3` | `409 VERSION_EXISTS` |
| `@acme/widgets` `latest` as version | — | Rejected: not valid SemVer |

A retry of `npm publish` after a network failure returns `409`; npm does not have idempotent
publication and the spec assumes this explicitly (ADR-007).

## 6. Publication

The client sends `PUT /npm/{feed}/{name}` with `Content-Type: application/json`. The body is
npm's CouchDB-style document:

- `versions` with **exactly one** new version; any other case is
  `400 PUBLISH_BODY_INVALID`.
- `_attachments` with **exactly** one attachment. The key can be
  `{name}-{version}.tgz` (with scope) or `{unscoped}-{version}.tgz`; the registry canonicalizes the
  served name as `{unscoped}-{version}.tgz`, which is what it emits in `dist.tarball`.
  `content_type: application/octet-stream` and `data` in base64.
- The document `name` and `versions[version].name` must match the URL; `version` must
  be SemVer and match the attachment name.
- The `package.json` inside the tarball is the **authoritative source** of the manifest; the
  body manifest is only validated and is not persisted. `dist` is generated by the registry.

Bounded processing (the full JSON is not materialized in memory):

1. The body goes to `staging/` with the limit
   `max_package_bytes × 4/3 + 1 MiB` (base64 + metadata).
2. The JSON is traversed in streaming; `_attachments[*].data` is decoded with a base64 reader
   into a staging file, applying the compressed limit while decoding.
3. The remaining fields are collected with size caps; unknown fields are ignored.

Persistence, reusing the current upload → inspection → blob → transaction flow:

- Blob by SHA-256 and a row in `version_file` with `kind = tarball` and the attachment name
  (ADR-023).
- `package_version.metadata` stores an adapter JSON with the normalized manifest, the
  integrity and the tarball name; `search_text` comes from the description and `keywords`.
- `listed = true`, `is_semver2 = false` (NuGet signal; ADR-022) and `is_prerelease` according to SemVer.
- Dist-tags in a new `package_tag` table (PK `feed_id + package_key + tag`, FK to the
  package), in the **same transaction** as the version: a failure does not leave a tag pointing to
  a nonexistent version. The migration is the next one available after the OP-022 migration
  (e.g. `0007`).
- Integrity computed during inspection: SRI `sha512` (`sha512-<base64>`) and legacy hexadecimal
  `sha1`, both persisted for the packument.
- Quota: the sum of version sizes is applied as today (ADR-014).
- `package.publish` audit with actor and token, also on rejections.

Body dist-tags: the tags sent are applied; if the package had no tags and the
body does not include any, `latest` is assigned to the first version. `latest` is not invented
when the body declares other tags (`npm publish --tag next` does not move `latest`). Response `201` with
`{"ok": true}`. Tag `PUT`/`DELETE`:

- `PUT .../dist-tags/{tag}` with a JSON version string; creates or moves; `201`.
- `DELETE .../dist-tags/{tag}`; `latest` → `400 DIST_TAG_PROTECTED`; nonexistent tag → `404`;
  nonexistent version → `404 VERSION_NOT_FOUND`; invalid tag → `400 DIST_TAG_INVALID`.

## 7. Read

Full packument (`GET .../{name}`, `Accept: application/json`):

| Field | Content |
|---|---|
| `_id`, `name` | Name as it appears in the feed |
| `dist-tags` | Current tags |
| `versions` | Manifest per version with `dist.tarball`, `dist.integrity`, `dist.shasum` |
| `time` | `created`, `modified` and one entry per version (RFC 3339 UTC with milliseconds, like the rest of the registry) |
| `description`, `keywords`, `license`, `repository` | From the last published manifest, if they exist |
| `maintainers` | Empty or omitted; the registry does not invent npm identities |

Abbreviated packument (`Accept: application/vnd.npm.install-v1+json`): `name`, `dist-tags` and
`versions` with the fields npm needs to install (`name`, `version`,
`dependencies`, `optionalDependencies`, `peerDependencies`, `peerDependenciesMeta`,
`bundleDependencies`, `bin`, `engines`, `os`, `cpu`, `dist`, `deprecated`). The rest is omitted.
`npm ci` obtains `dist.integrity` from here.

- `dist.tarball` is absolute and stable: `{public_url}/npm/{feed}/{@scope/name}/-/{file}.tgz`;
  a change to `public_url` invalidates lockfiles, and the documentation will warn about it.
- Blocked version (ADR-013): it remains in the full and abbreviated packument; the tarball
  responds `410 PACKAGE_BLOCKED`. The `deprecated` field is not used for blocking in the first
  cut.
- `listed` does not apply to npm: versions are always listed; withdrawal does not exist (section
  10).
- Tarball: `application/octet-stream`, `HEAD` support, no ranges. No compression.
- `/-/whoami` → `{"username": "<principal.name>"}`; `/-/ping` → `{}`.

## 8. tar/gzip validation (AC-03)

Inspection runs like NuGet's: in the blocking pool, with its own semaphore and deadline
(ADR-014).

| Limit | Default |
|---|---|
| Compressed size | shared `max_package_bytes` (100 MiB) |
| Total uncompressed size | 512 MiB |
| tar entries | 50,000 |
| `package.json` | 1 MiB |
| `package.json` JSON depth | 64 |
| Inspection time | 30 s |
| Simultaneous inspections | those of NuGet |

Rules (mirroring NuGet's, adapted to tar):

- Single-member gzip; invalid or truncated gzip is rejected. Caps are applied
  while decompressing, not against declared sizes.
- Only regular entries and directories. Symlinks, hardlinks, devices, FIFOs,
  sparse or encrypted entries, or entries of a type the library does not interpret are rejected; normal
  tar extensions (pax, GNU long name) are processed as metadata, not as entries.
- Each path must be relative and start with `package/`; absolute paths, `..`,
  control characters or `:`, reserved Windows names, trailing dots or
  spaces, duplicate entries (case-insensitive) and any path outside
  `package/` are rejected.
- `package/package.json` is required; a valid JSON with `name` and `version` matching the
  URL and the attachment, with a name and version valid according to section 5.
- `scripts` are not executed or materialized; `bin` is served as metadata.
- A rejection persists nothing and is audited with its code.

Adversarial fixtures planned (OP-024): traversal (`package/../..`), absolute path, symlink,
hardlink, duplicate entry with different capitalization, gzip bomb, 50,001 entries,
missing `package.json`, oversized or with invalid JSON, name/version that do not match
the URL, invalid base64, attachment `length` that lies, tarball larger than the limit and
body with two versions or two attachments.

## 9. Errors

npm surface error format: JSON `{"error": "<CODE>: <message>", "reason":
"<message>", "code": "<CODE>"}`; the stable prefix is the same `CODE` used by the other
surfaces, and `reason` is left for human reading. npm displays the text and its own code
`E<status>`.

| Code | HTTP | When |
|---|---|---|
| `AUTH_REQUIRED` | 401 | Missing, invalid, expired or revoked token |
| `FEED_NOT_FOUND` | 404 | Nonexistent feed, without grant or of another format |
| `AUTH_SCOPE_MISSING` | 403 | Insufficient role for the operation |
| `AUTH_PREFIX_DENIED` | 403 | Unauthorized publication prefix |
| `PACKAGE_NOT_FOUND` | 404 | Nonexistent package or version with feed access |
| `PUBLISH_BODY_INVALID` | 400 | Body that does not comply with section 6 |
| `VERSION_EXISTS` | 409 | Version identity already published |
| `TARBALL_INVALID` | 400 | Invalid gzip/tar or unexpected structure |
| `PACKAGE_UNSAFE_PATH` | 400 | Dangerous or duplicate path or entry |
| `PACKAGE_LIMIT_EXCEEDED` | 413 | An inspection limit or the deadline is exceeded |
| `PACKAGE_TOO_LARGE` | 413 | The compressed size is exceeded |
| `DIST_TAG_INVALID` | 400 | Invalid tag name |
| `DIST_TAG_PROTECTED` | 400 | Attempt to delete `latest` |
| `VERSION_NOT_FOUND` | 404 | Tag pointing to a nonexistent version |
| `FEED_QUOTA_VERSIONS` | 403 | Version quota exceeded |
| `FEED_QUOTA_STORAGE` | 413 | Storage quota exceeded |
| `PACKAGE_BLOCKED` | 410 | Tarball of a blocked version |
| `UPLOADS_BUSY` | 503 | No upload slots |
| `UPLOAD_TIMEOUT` | 408 | Upload exhausted the deadline |
| `RATE_LIMITED` | 429 | Limit per principal or IP |
| `INTERNAL` | 500 | Unclassified error |

## 10. Withdrawal and search (accepted decision)

Decision for the first cut:

- **No `npm unpublish`.** The registry never deletes bytes (ADR-005); the publication route
  rejects any attempt to modify or remove existing versions with `409` or
  `PUBLISH_BODY_INVALID`. Actual withdrawal will be done with the existing administrative block.
- **No `npm deprecate`.** npm implements `deprecate` with a `PUT` that modifies the packument;
  the additive publication contract rejects it. It remains as an evolution with its own metadata
  migration if prioritized.
- **No `npm search`.** `/-/v1/search` is not registered; `npm search` receives `404`. Registry
  search remains the administrative one and the NuGet one.
- **Blocking as withdrawal:** `410` on the tarball, version visible in the packument; Maintainer or admin
  blocks and unblocks with a reason and auditing, like NuGet.

Review condition: if the pilot or users request npm withdrawal or search, an evolution
task is opened before expanding this scope.

## 11. Tests and handoff

- **Identity corpus:** valid and invalid names (scoped, uppercase, 214-character
  boundaries) and SemVer versions, including build metadata; no NuGet equivalents.
- **Adapter unit tests:** body parsing, `package.json` extraction, integrity,
  dist-tags, rendering of the full and abbreviated packument.
- **Server integration:** per-surface auth, 401/403/404, `VERSION_EXISTS`,
  `DIST_TAG_*`, quotas and auditing, version + tag transaction.
- **Security:** the adversarial fixtures from section 8, without persistence.
- **Real E2E:** `scripts/e2e-npm.sh` script (pattern of `scripts/e2e-dotnet.sh`) and CI job with
  the matrix from section 2: F1–F11, including revocation, foreign prefix, isolation between
  feeds and blocking.
- **Docs to update when implementing:** `docs/security.md` (limits, error table and
  npm fixtures), `docs/compatibility.md` (verified npm matrix), `docs/cli.md`
  (`.npmrc` and `onepack token create`). The spec does not modify them yet.
- **Handoff:** OP-024 creates `onepack-npm` (ADR-026), registers the routes approved here and adds
  the `package_tag` migration; OP-025 decides the CLI commands. No npm dependency enters
  `core` or `storage`.

## 12. Verification of this specification

- AC-01: matrix from section 2 with publish/install/ci, scoped/unscoped, dist-tags and the rule
  for fixing versions in OP-024.
- AC-02: auth and error contracts from sections 4 and 9, testable with missing/revoked token,
  insufficient role and foreign namespace without revealing packages.
- AC-03: adversarial plan from section 8 (traversal, links, expansion, collisions) and metadata
  that announces only what is implemented (section 3).
- AC-CIERRE: closed with the recorded acceptance of 2026-10-19, the OP-023 note and the
  backlog synchronized.

Evidence: path of this document, SHA-256 of the accepted content, local commit `fa0d526`
(delivery) and the acceptance recorded in Obsidian on 2026-10-19.
