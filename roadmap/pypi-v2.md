# PyPI conformance specification — private hosting v2 (OP-027)

> Task OP-027. Depends on OP-020 and OP-021 (verified). Delivers the contracts that OP-033
> (adapter), OP-034 (CLI/guides) and OP-035 (mixed verification) need; implements nothing.
>
> Sources: `roadmap/adr-v2.md` (ADR-021…026), `roadmap/npm-v2.md` (style reference),
> `docs/security.md`, `docs/compatibility.md`, `docs/api.md` and the code in
> `crates/server/src` and `crates/core/src`.
>
> Baseline consulted: `88467a94eaee4fc7345bac54fdf6fa4449a913b8`.
> Date: 2026-10-19. Status: **accepted**. Review on 2026-10-19 by the maintainer
> (instruction in the work session); matrix, contracts and first-cut decisions
> accepted without scope changes. Implementation corresponds to OP-033, OP-034 and OP-035.

## 1. Scope

Enable private publication with **twine** and consumption with **pip** against a `pypi` feed:
simple index (PEP 503 HTML and PEP 691 JSON), upload of wheels and sdists, multiple files
per version, SHA-256 integrity, token authentication, inspection without running code and
stable errors, reusing existing roles, quotas, auditing and storage.

**Out of scope:** PyPI proxy or mirror; running `setup.py`, PEP 517 backends or build hooks on
the server; PyPI JSON API; *yank* endpoints; *search*; metadata-only uploads;
`.zip` as sdist and `.egg`; poetry, uv, PDM or other unlisted clients (not verified or
announced). Documented as a limitation with a review condition in
section 11.

## 2. Initial conformance matrix (AC-01)

Candidate versions; the exact patch and Python version are fixed when preparing OP-033 and a
row is only published as verified if it passes the E2E in section 12.

| Tool | Role in the matrix |
|---|---|
| pip 25.x | consumption with `--no-cache-dir`, private index |
| pip 24.x | compatibility with the previous line |
| twine 6.x | wheel and sdist publication |

When preparing OP-033, patches are fixed and the current major version is added if it already
exists; a tool that has not passed the E2E in CI is not announced. `uv`, poetry and PDM are not verified.

| # | Flow | Example | What it checks |
|---|---|---|---|
| F1 | sdist upload | `twine upload --repository-url ... dist/hemia_core-1.0.0.tar.gz` | Multipart, metadata, identity |
| F2 | wheel upload | `twine upload ... dist/hemia_core-1.0.0-py3-none-any.whl` | ZIP, `METADATA`, tags |
| F3 | Several files | sdist + wheel of the same version | Multiple `version_file` |
| F4 | File added later | new wheel in an already published version | Add without duplicating version |
| F5 | wheel install | `pip install hemia-core --index-url ... --no-cache-dir` | JSON/HTML index + download + hash |
| F6 | sdist install | `pip install --no-binary :all: hemia-core ...` | Local sdist build (not on the server) |
| F7 | Private dependency | `hemia-app` with `Requires-Dist: hemia-lib` | Resolution from the same index |
| F8 | Hashes | `requirements.txt` with `--hash=sha256:...` and `--require-hashes` | Integrity verified by pip |
| F9 | Prerelease | `pip install hemia-core --pre` | PEP 440 ordering |
| F10 | Identical retry | Second `twine upload` of the same file | Idempotent `200 OK` |
| F11 | Different bytes | Same filename, changed content | `409 VERSION_FILE_CONFLICT` + auditing |
| F12 | Adversarial auth | no token, revoked, reader publishing, name outside prefix | 401/403/404 according to section 4 |
| F13 | Isolation | token with a grant only on feed A against feed B | 404 without disclosure |
| F14 | Blocked version | `pip install pkg==blocked_version` | File outside the index and `410` |

## 3. HTTP surface

Base: `{public_url}/pypi/{feed}` (ADR-018). The feed format must be `pypi`; if it is of another
format, the route responds `404` as `FEED_NOT_FOUND` (ADR-024).

| Method | Route | Operation |
|---|---|---|
| GET | `/pypi/{feed}/simple/` | Index root: all projects (HTML or JSON) |
| GET | `/pypi/{feed}/simple/{project}/` | Project page (PEP 503 HTML or PEP 691 JSON) |
| GET, HEAD | `/pypi/{feed}/packages/{filename}` | Download of a distribution file |
| POST | `/pypi/{feed}/` and `/pypi/{feed}` | twine upload (multipart) |
| — | `/pypi/{feed}/pypi/{project}/json` | Not implemented (JSON API) |
| — | Yank | Not implemented: no PEP 592 in the first cut |
| — | Search | Not implemented |

Route details:

- The `{project}` in the URL is accepted in any form that normalizes to the same project
  (PEP 503); the response always uses the normalized name. The trailing slash is the canonical
  form and is also accepted without it.
- Both POST variants are registered so twine does not depend on redirects.
- `{filename}` is unique within the feed because it encodes the normalized name and version; even so,
  the handler resolves by project + version + file, not by free text.
- No response compression in the first cut (`identity`), like NuGet and npm.

## 4. Authentication and authorization (AC-02)

The PyPI surface accepts **only** `Authorization: Basic` with the token in the password; the
user is informative (`__token__` by tool convention). pip and twine use this
mechanism; `X-NuGet-ApiKey` and `Bearer` are not accepted on this surface. This implies adding
`Surface::PyPi` to the route classification, to `credential()` and to error rendering, and
the `pypi` label in metrics (ADR-024).

All requests remain authenticated; there is no anonymous read. Client configuration:

```ini
# ~/.config/pip/pip.conf
[global]
index-url = https://__token__:${ONEPACK_TOKEN}@packages.example.com/pypi/internal/simple/
```

```ini
# ~/.pypirc
[distutils]
index-servers = onepack

[onepack]
repository = https://packages.example.com/pypi/internal/
username = __token__
password = ${ONEPACK_TOKEN}
```

| Case | Result |
|---|---|
| Missing, expired, revoked or malformed token | `401 AUTH_REQUIRED` + `WWW-Authenticate: Basic realm="onepack"` |
| Valid token without a grant on the feed | `404 FEED_NOT_FOUND` (does not reveal feed, project or file; ADR-012) |
| `reader` tries to publish | `403 AUTH_SCOPE_MISSING` |
| `publisher` publishes a name outside its prefixes | `403 AUTH_PREFIX_DENIED` |
| `reader`/`publisher`/`maintainer` read | Allowed according to cumulative role |
| `maintainer` or admin | Publication and blocking via the administrative API |

Additional rules:

- Read: `Access::Read` on the feed. Index, project page and download share the decision;
  an error never includes project or file names from other feeds.
- Publication: `Access::Publish` + grant pattern on `package_key` (normalized
  name; ADR-022). Examples: `hemia-*` covers `hemia-core`, `hemia.logging` normalizes to
  `hemia-logging`.
- Revocation takes effect on the next request; pip may store credentials in
  netrc/keyring, so the documentation will remind users to refresh them after revoking.
- TLS: pip and twine send credentials over HTTPS. Deployment requires HTTPS at the reverse
  proxy and an HTTPS `public_url`; the server does not trust `X-Forwarded-For` (ADR-018).

## 5. Identity, versions and conflicts

Adapter rules (ADR-022); the implementation will reproduce PEP 503, PEP 508 and PEP 440 with a
conformance corpus.

| Rule | Detail |
|---|---|
| Name normalization | PEP 503: lowercase and runs of `-`, `_`, `.` → `-` |
| Valid name form | PEP 508: letters, digits and `-_.` inside, starts and ends alphanumeric; ≤ 100 characters per registry policy |
| `package_key` | Normalized name |
| Version | Canonical PEP 440: epoch, release, pre (`a`/`b`/`rc`), post, dev and local (`+local`) |
| Version identity (`version_key`) | Full canonical string, including epoch and local; two texts that compare equal but differ (`1.0` and `1.0.0`) are different identities |
| Ordering | PEP 440; `is_prerelease` for pre and dev |
| Files | They do not create identity: they are added to the version; `{filename}` is their key within the feed |

| Publication | Name / version | Result |
|---|---|---|
| `Zope.Interface` `5.0` | `zope-interface` / `5.0` | Accepted with normalized name |
| `hemia.core` `1.0rc1` | `hemia-core` / `1.0rc1` | Accepted, prerelease |
| `hemia-core` `1.0.post1` | `hemia-core` / `1.0.post1` | Different from `1.0`; accepted |
| `hemia-core` `1!2.0` | `hemia-core` / `1!2.0` | Canonical epoch; accepted |
| `hemia-core` `1.0+ubuntu.1` | `hemia-core` / `1.0+ubuntu.1` | Different from `1.0`; accepted |
| `hemia-core` `v1.0` | `hemia-core` / `1.0` | Accepted; the `v` prefix is normalized |
| `hemia-core` `1.0.0` after `1.0` | `hemia-core` / `1.0.0` | Different identity; accepted (they compare equal, they are not the same version) |
| `Hemía` | — | Rejected: non-ASCII name |

## 6. Publication (upload)

twine sends a multipart `POST` to the repository URL:

- `:action = file_upload` and `protocol_version = 1`; any other action is rejected with
  `400 PUBLISH_BODY_INVALID`.
- `name`, `version`, `filetype` (`sdist` or `bdist_wheel`) and `pyversion`, validated against the
  file. Core metadata fields (summary, classifiers, `requires_dist`,
  `requires_python`, etc.) are accepted as input, but **the authoritative source is the
  metadata inside the file** (`METADATA` or `PKG-INFO`); form fields are only
  validated and unknown ones are ignored.
- `content` with the file and its name. Optional `sha256_digest`: if present, it must match
  the computed one; `md5_digest` and others are ignored.

Bounded processing:

1. The multipart is traversed in streaming into `staging/`; the `content` file uses the shared
   compressed limit `max_package_bytes` (100 MiB) and text fields a cap of 1 MiB
   in total.
2. The inspection in section 8 validates the file and extracts the metadata.
3. The blob enters by SHA-256 and a `version_file` row is persisted with `kind = wheel` or
   `sdist` and the original filename (ADR-023). No new tables are needed.

File rules:

- A version is created with its first file; subsequent ones are added to the same version if
  their filename does not exist.
- **Identical retry:** same filename and same SHA-256 → idempotent `200 OK`, without a new
  write and with the same semantics as ADR-007. This is the case of a retried `twine upload`.
- **Conflict:** same filename with different bytes → `409 VERSION_FILE_CONFLICT`, audited
  (`package.publish`, `denied`). twine can avoid the retry with `--skip-existing`, which
  queries the index; the registry does not delegate the decision to the client.
- Quota and auditing are applied as in NuGet/npm; the version's `created_at` does not
  change when files are added.
- No `setup.py`, PEP 517 backend or hook is executed; the registry only reads and validates.

## 7. Read

Project page in HTML (PEP 503):

```html
<a href="{public_url}/pypi/{feed}/packages/{filename}#sha256={hex}"
   data-requires-python="&gt;=3.9">{filename}</a>
```

- One anchor per file, with `#sha256=` mandatory so pip can verify.
- `data-requires-python` is copied escaped from the metadata if it exists; `data-yanked` is not used
  in the first cut.
- The `/simple/` root lists the projects (normalized name) linking to their page.

Project page in JSON (PEP 691, `Accept: application/vnd.pypi.simple.v1+json`):

```json
{
  "meta": { "api-version": "1.0" },
  "name": "hemia-core",
  "files": [
    {
      "filename": "hemia_core-1.0.0-py3-none-any.whl",
      "url": "https://packages.example.com/pypi/internal/packages/hemia_core-1.0.0-py3-none-any.whl",
      "hashes": { "sha256": "…" },
      "requires-python": ">=3.9",
      "yanked": false
    }
  ]
}
```

pip negotiates JSON and falls back to HTML if it does not understand it; both are served from the same state.

- Download: `application/octet-stream`, `HEAD` support, no ranges, no compression.
- Blocked version (ADR-013): its files **are omitted from the index** and direct download
  responds `410 PACKAGE_BLOCKED`. This is deliberately different from NuGet: in pip, a
  yanked version would still be installed with an exact pin, and blocking must prevent that. The version
  remains visible through the administrative API.
- `data-dist-info-metadata` (PEP 658) is not implemented; pip falls back to downloading the wheel to read
  the metadata, which is the correct behavior without that extension.

## 8. ZIP/tar and metadata validation (AC-03)

Inspection runs like NuGet/npm's: in the blocking pool, with its own semaphore and deadline
(ADR-014). The already tested path rules are reused, adapted to the file type.

| Limit | Default |
|---|---|
| Compressed size | shared `max_package_bytes` (100 MiB) |
| Total uncompressed size | 512 MiB |
| ZIP entries (wheel) | 20,000 |
| tar entries (sdist) | 50,000 |
| `METADATA` / `PKG-INFO` | 1 MiB |
| Inspection time | 30 s |
| Simultaneous inspections | those of NuGet |

**Wheel (ZIP):**

- Same rules as the `.nupkg` (Phase 5): number of entries, declared sizes, dangerous
  paths, case-insensitive duplicates, symbolic links and encrypted entries.
- Exactly one `{name}-{version}.dist-info/METADATA` must exist and correspond to the
  filename and the URL; the metadata is parsed with caps and without executing anything.
- The filename complies with PEP 427 (distribution escaped with `_`, PEP 440 version, tags); the
  components are validated with a corpus in OP-033.

**Sdist (`*.tar.gz`):**

- Same rules as the npm tarball: single-member gzip, only files and directories, no
  symlinks, hardlinks, devices, FIFOs, sparse or encrypted entries.
- A single root directory `{name}-{version}/` with `PKG-INFO` inside; any
  absolute path, `..`, control characters or path outside that directory is rejected.

**Metadata:**

- Normalized/canonicalized `name` and `version` must match the filename, the URL and
  the form fields; `Requires-Python` feeds the index and `Summary`/classifiers
  feed `search_text`.
- Dangerous artifacts are rejected without persisting and audited with their code.

Adversarial fixtures planned (OP-033): wheel with traversal, ZIP bomb, duplicate entries,
symlink and encrypted entry; sdist with traversal, symlink, hardlink, gzip bomb and 50,001
entries; filename or metadata that do not match name/version; missing `METADATA`,
oversized or with invalid JSON/RFC 822; `sha256_digest` that lies; file larger than
the limit; action other than `file_upload`.

## 9. Errors

PyPI surface error format: JSON `{"error": "<CODE>: <message>", "reason":
"<message>", "code": "<CODE>"}`; the stable prefix is the same `CODE` as the other
surfaces and twine/pip display the text.

| Code | HTTP | When |
|---|---|---|
| `AUTH_REQUIRED` | 401 | Missing, invalid, expired or revoked token |
| `FEED_NOT_FOUND` | 404 | Nonexistent feed, without grant or of another format |
| `AUTH_SCOPE_MISSING` | 403 | Insufficient role for the operation |
| `AUTH_PREFIX_DENIED` | 403 | Name outside the grant prefixes |
| `PACKAGE_NOT_FOUND` | 404 | Nonexistent project, version or file with feed access |
| `PUBLISH_BODY_INVALID` | 400 | Action, fields or form that do not comply with section 6 |
| `FILENAME_MISMATCH` | 400 | Filename that does not correspond to name/version |
| `WHEEL_INVALID` | 400 | Invalid ZIP, missing or inconsistent `METADATA` |
| `SDIST_INVALID` | 400 | Invalid tar.gz or unexpected structure |
| `PACKAGE_UNSAFE_PATH` | 400 | Dangerous or duplicate path or entry |
| `METADATA_INVALID` | 400 | Unreadable, inconsistent or oversized metadata |
| `VERSION_FILE_CONFLICT` | 409 | Same filename with different bytes |
| `PACKAGE_LIMIT_EXCEEDED` | 413 | An inspection limit or the deadline is exceeded |
| `PACKAGE_TOO_LARGE` | 413 | The compressed size is exceeded |
| `FEED_QUOTA_VERSIONS` | 403 | Version quota exceeded |
| `FEED_QUOTA_STORAGE` | 413 | Storage quota exceeded |
| `PACKAGE_BLOCKED` | 410 | Download of a file from a blocked version |
| `UPLOADS_BUSY` | 503 | No upload slots |
| `UPLOAD_TIMEOUT` | 408 | Upload exhausted the deadline |
| `RATE_LIMITED` | 429 | Limit per principal or IP |
| `INTERNAL` | 500 | Unclassified error |

## 10. Provenance and dependency confusion (REQ-03)

pip does not have scoped registries like npm: it resolves all projects against the configured
indexes. If `--extra-index-url` is combined with PyPI, a collidable private name can be
resolved from the public index (or vice versa). First-cut rules:

- The official guide uses **only** `index-url` pointing to the private feed; for public
  dependencies it is decided explicitly (publish them to the feed or combine them deliberately).
- The registry cannot prevent an insecure client configuration; the documentation
  (`docs/compatibility.md`, `docs/security.md`) will warn about the risk and show the
  recommended configuration.
- Publication patterns by prefix and upload auditing reduce name squatting
  within the feed; they do not replace the user's naming policy.
- `pip` does not query any public index on its own: if the user does not configure
  `extra-index-url`, resolution is private only.

## 11. States, withdrawal and yank (accepted decision)

- **No `yank` (PEP 592) in the first cut.** Neither the endpoint nor
  `data-yanked` is implemented. Effective withdrawal is the existing administrative block (ADR-013), which
  omits files from the index and responds `410`.
- **There is no deletion of files or versions** (ADR-005): immutability is applied per
  filename, not per complete version. Adding another file to an existing version is valid;
  repeating a filename with different bytes is rejected and audited.
- The idempotency decision for the identical retry is ADR-007 applied per file;
  NuGet version immutability is not transferred to the Python version, which can have several
  distributions.
- Review condition: if the pilot requests yank, an evolution task is opened with its
  migration before expanding the scope.

## 12. Tests and handoff

- **Identity corpus:** PEP 508/503 names and PEP 440 versions (epoch, pre, post, dev,
  local, `v` prefix, `1.0`/`1.0.0` comparisons), with wheel filename edge cases.
- **Adapter unit tests:** multipart, `METADATA`/`PKG-INFO` extraction, integrity,
  ordering, HTML and JSON index rendering.
- **Server integration:** per-surface auth, 401/403/404, identical retry,
  `VERSION_FILE_CONFLICT`, quotas and auditing, isolation between feeds.
- **Security:** adversarial fixtures from section 8, without persistence.
- **Real E2E:** `scripts/e2e-pypi.sh` script (pattern of `scripts/e2e-dotnet.sh`) and CI job with
  the matrix from section 2: F1–F14 with real pip and twine, empty cache and private dependency.
- **Docs to update when implementing:** `docs/security.md` (limits, errors and
  Python fixtures), `docs/compatibility.md` (pip/twine matrix and dependency confusion warning),
  `docs/cli.md` or the usage guide (`.pypirc`, `pip.conf`). The spec does not modify them yet.
- **Handoff:** OP-033 creates `onepack-pypi` (ADR-026), registers the approved routes and reuses
  `version_file` without a new migration; OP-034 decides the commands and guides; OP-035 verifies the
  mixed registry. No Python dependency enters `core`, `storage` or the runtime.

## 13. Verification of this specification

- AC-01: upload, index and download contract from sections 3, 6 and 7; wheel,
  sdist, prerelease and private dependency fixtures in sections 2 and 12.
- AC-02: file decision from section 11 (adding another file; different bytes
  rejected and audited) and auth without metadata leakage from section 4.
- AC-03: plan with pip from an empty cache and adversarial fixtures from section 8, including
  the unauthorized accesses from section 4.
- AC-CIERRE: closed with the recorded acceptance of 2026-10-19, the OP-027 note and the
  backlog synchronized.

Evidence: path of this document, SHA-256 of the accepted content, local commit `7d9d1dd`
(delivery) and the acceptance recorded in Obsidian on 2026-10-19.
