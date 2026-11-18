# Package and abuse security

> Phase 5 ([ADR-013](../roadmap/adr-mvp.md#adr-013), [ADR-014](../roadmap/adr-mvp.md#adr-014), [ADR-018](../roadmap/adr-mvp.md#adr-018)).
> Last updated: 2026-10-04

## What the registry does and does not do

- Treats every `.nupkg` as **untrusted content**: it inspects it with a bounded budget, never extracts it to disk, and never executes or downloads anything the package references.
- **Does not analyze malware.** A package that passes inspection may contain malicious code.
- **Does not validate signature trust.** Signed packages are stored and served byte for byte, without checking the certificate.
- **Blocking a version does not delete copies already downloaded** into client caches (`~/.nuget/packages`, HTTP caches, CI agents) or mirrors. After an incident, those caches must be cleaned separately.

## Package inspection

Every upload goes through these checks before anything is stored. All of them are configured in `onepackd serve` (flag or environment variable):

| Limit | Flag | Default |
|---|---|---|
| Compressed size | `--max-package-size-mib` | 100 MiB |
| Number of ZIP entries | `--max-zip-entries` | 20,000 |
| Uncompressed size per entry | `--max-entry-size-mib` | 512 MiB |
| Total uncompressed size | `--max-uncompressed-size-mib` | 2048 MiB |
| `.nuspec` size | `--max-nuspec-size-kib` | 1024 KiB |
| `.nuspec` XML depth | `--max-xml-depth` | 32 |
| Inspection time | `--inspection-timeout-secs` | 30 s |
| Concurrent inspections | `--max-concurrent-inspections` | half the cores |

How they are enforced:

- The number of entries is read from the ZIP end-of-central-directory record (also in ZIP64) **before** loading the central directory, so a ZIP that declares millions of entries does not allocate memory for them.
- Uncompressed sizes are checked against those declared in the central directory, without decompressing. Only the `.nuspec` is decompressed, and its read is cut off at the limit even if the declared size lies.
- The `.nuspec` is parsed **without DTD**: any `<!DOCTYPE>` is rejected, so there are no document-defined entities (billion laughs) or external ones (XXE). Only the five predefined XML entities and numeric references are accepted.
- Paths that would be dangerous when extracted by a client are rejected: absolute (`/x`, `\\x`, `C:/x`), with `.` or `..` segments, with control characters or `:`, with Windows reserved names (`CON`, `PRN`, `AUX`, `NUL`, `COM0-9`, `LPT0-9`, with or without extension), ending in a dot or space, and with incomplete `%XX` escapes. Each path is checked as-is and with the `%XX` escape resolved, because NuGet clients undo it on extraction.
- Duplicate entries (case-insensitive and escape-insensitive), symbolic links, and encrypted entries are also rejected.
- Inspection runs on the blocking thread pool with its own semaphore: it never occupies the threads serving downloads.

## Errors

Error responses start with a stable code (`CODE: message`) on NuGet routes, which clients display verbatim, and use `{"error": {"code", "message"}}` on `/api/v1`.

| Code | Status | When |
|---|---|---|
| `PACKAGE_INVALID` | 400 | Invalid ZIP or `.nuspec`, DTD, unknown entity, encrypted entry, or symbolic link. |
| `PACKAGE_UNSAFE_PATH` | 400 | Unsafe path or duplicate entry. |
| `PACKAGE_LIMIT_EXCEEDED` | 413 | An inspection limit or the maximum time is exceeded. |
| `PACKAGE_TOO_LARGE` | 413 | The compressed size is exceeded. |
| `FEED_QUOTA_VERSIONS` | 403 | The feed reached its version quota. |
| `FEED_QUOTA_STORAGE` | 413 | The feed reached its storage quota. |
| `PACKAGE_BLOCKED` | 410 | Download of a blocked version. |
| `UPLOADS_BUSY` | 503 + `Retry-After` | No upload slots remain. |
| `UPLOAD_TIMEOUT` | 408 | The upload exceeded `--upload-timeout-secs`. |
| `RATE_LIMITED` | 429 + `Retry-After` | Request limit per principal or per IP. |

Publication rejections are audited (`package.publish`, `denied`, with the code).

## Saturation

| Limit | Flag | Default |
|---|---|---|
| Concurrent uploads | `--max-concurrent-uploads` | 8 |
| Maximum upload time | `--upload-timeout-secs` | 600 s |
| Requests per principal | `--principal-rate-limit` / `--principal-rate-burst` | 50/s, burst of 1000 |
| Requests per IP | `--ip-rate-limit` / `--ip-rate-burst` | 100/s, burst of 2000 |

- The upload slot is acquired **before reading the body**: a burst receives `503` without creating staging files or pending tasks.
- The per-IP limit is applied **before authentication**, so it also throttles token brute force. The per-principal limit is applied afterwards.
- The IP is the socket IP. Behind a reverse proxy, all requests share the proxy IP because the server does not trust `X-Forwarded-For` ([ADR-018](../roadmap/adr-mvp.md#adr-018)). In that deployment, enforce the IP limit at the proxy, and raise `--ip-rate-limit` or disable it with `0`.
- A `restore` of a large solution makes several requests per package. The default bursts absorb it; tune them if CI agents share a credential.
- Counters live in memory and are lost on restart. Their size is bounded: with more than 100,000 keys, inactive clients are forgotten.

## Per-feed quotas

```bash
onepackd feed quota internal --max-storage-mib 10240 --max-versions 5000 --data-dir ./data
onepackd feed show internal --data-dir ./data
```

`0` means no limit. Usage is the logical sum of version sizes: two versions with the same content count twice, even if only one copy is stored on disk. A new quota does not affect what is already published even if it exceeds it. If a publication would exceed the quota, nothing is stored and it is audited with the code.

## Version blocking

`availability` is independent of `listed` ([ADR-013](../roadmap/adr-mvp.md#adr-013)):

| | Search | Flat container and registries | Download |
|---|---|---|---|
| `listed=false` | no | yes (`listed: false`) | yes |
| `blocked` | no | yes, advertised as deprecated | `410 PACKAGE_BLOCKED` |

The blocked version is kept in the metadata on purpose: this way `restore` fails with an explicit error, instead of silently resolving another version or saying it does not exist. .NET clients show the status phrase:

```text
Response status code does not indicate success: 410 (PACKAGE_BLOCKED - version blocked by the registry).
```

Blocking and unblocking require the Maintainer role (or administrator) and a reason, which is recorded in the audit (`package.block`, `package.unblock`). The reason is not published in the NuGet metadata; only those who can maintain the feed see it:

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"reason": "CVE-2026-0001"}' \
  https://packages.example.com/api/v1/feeds/internal/packages/hemia.core/1.0.0/block
curl -H "Authorization: Bearer $TOKEN" \
  https://packages.example.com/api/v1/feeds/internal/packages/hemia.core/1.0.0
```

With the CLI:

```bash
onepack package block --feed internal Hemia.Core 1.0.0 --reason "CVE-2026-0001"
onepack package inspect --feed internal Hemia.Core 1.0.0
onepack package unblock --feed internal Hemia.Core 1.0.0 --reason "published patch"
```

Without a running server, or with access only to the data directory:

```bash
onepackd package block --feed internal --id Hemia.Core --version 1.0.0 --reason "CVE-2026-0001" --data-dir ./data
onepackd package unblock --feed internal --id Hemia.Core --version 1.0.0 --reason "published patch" --data-dir ./data
```

## Security suite

`tests/security` generates the malicious artifacts on every run and verifies that they are rejected without persisting anything:

```bash
cargo test -p onepack-server --test security
```

| Scenario | Test |
|---|---|
| ZIP bomb with forged sizes (~4 GiB per entry) | `zip_bomb_with_forged_sizes_is_rejected_without_decompressing` |
| Real ZIP bomb (12 MiB of zeros in less than 200 KiB) | `zip_bomb_with_real_compression_is_rejected` |
| 16 MiB `.nuspec`, with honest and forged size | `oversized_nuspec_is_cut_at_the_limit` |
| Too many entries | `too_many_entries_are_rejected` |
| Billion laughs | `billion_laughs_is_rejected` |
| XXE and undefined entities | `external_entities_are_never_resolved` |
| 10,000 nesting levels | `deeply_nested_xml_is_rejected` |
| Path traversal, absolute paths, reserved names, ADS | `path_traversal_is_rejected_and_nothing_is_written_outside_staging` |
| Duplicate entries | `duplicate_entries_are_rejected` |
| Upload burst | `upload_burst_degrades_with_503_without_queueing` |
| Upload that never finishes | `stalled_upload_times_out` |
| Per-principal and per-IP limit | `requests_are_rate_limited_per_principal`, `requests_are_rate_limited_per_ip_before_authentication` |
| Blocked version | `blocked_version_cannot_be_downloaded_but_stays_visible` and the blocking step of `scripts/e2e-dotnet.sh` with a real `dotnet restore` |
| Quotas | `version_quota_rejects_with_specific_code`, `storage_quota_rejects_with_specific_code` |

## Manual security review (Phase 5)

Review performed on the code of the `feature/phase-5-hardening` branch on 2026-10-04.

| Area | Check | Result |
|---|---|---|
| ZIP reading | Only the `.nuspec` is decompressed, with `take(limit + 1)`; the rest is validated against the central directory (`by_index_raw`). | Correct. |
| Central directory memory | The number of entries is validated before `ZipArchive::new`. The central directory size is bounded by the compressed size. | Correct. |
| Disk writes | The upload goes to `staging/` with a server-generated name and moves to `blobs/sha256/...` by hash. No ZIP entry name reaches the filesystem. | Correct. |
| XML | `DocType` is rejected before processing entities. General entities only resolve the predefined ones. Depth and size are bounded. | Correct. |
| URL segments | Id and version are normalized with the NuGet parsers before being used in queries; invalid values are `404`. | Correct (Phase 3). |
| Time | Inspection checks a deadline on every entry and on every XML event. Upload has its own deadline. | Correct. Residual risk: a single slow blocking disk read cannot be interrupted. |
| Concurrency | Upload slots with `try_acquire` (no queue) and an inspection semaphore. Rejections do not create files. | Correct. |
| Rate limiting | In-memory token bucket with a bounded number of keys. Per IP before authentication. | Correct. Residual risk: behind a proxy, the per-IP limit is global (documented). |
| Blocking | Checked in the download handler for `.nupkg` and `.nuspec`, for any role. The reason is not exposed in NuGet. | Correct. |
| Quotas | Pre-check and inside the single writer's transaction, so two concurrent publications cannot exceed the quota together. | Correct. If the quota is exceeded between the pre-check and the transaction, the blob is left orphaned and cleanup collects it. |
| Secrets in logs | Block reasons and codes are logged; tokens are not (E2E also checks this with the administrative token). | Correct. |
| Out of scope | Antimalware, signature verification, automatic quarantine, IP limits for trusted proxies. | Documented above. |
