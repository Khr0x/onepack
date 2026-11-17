# Pilot friction log

> Phase 8. See [the pilot plan](README.md) for what to log and how to classify it.

## Pilot details

| Field | Value |
|---|---|
| Team | _pending_ |
| Person operating the server | _pending_ |
| Period | _pending_ (start – end) |
| Server environment | _pending_ (systemd / container, OS, architecture, reverse proxy) |
| onepack version | _pending_ |
| CI used | _pending_ |
| Projects and libraries | _pending_ |

## Frictions

Classes: **B** = MVP blocker, **P** = post-MVP. Status: open, fixed (with a link to the change), dismissed (with the reason).

| ID | Date | Who / where | What they were trying to do | What happened | Time lost | Class | Status |
|---|---|---|---|---|---|---|---|
| F-001 | 2026-10-04 | Automated acceptance test | Restore from CI with a revoked token | `dotnet restore` only said `NU1301: unable to load the service index`, without mentioning the credential. The server did log `reason="revoked"`. | — | B | Fixed: `onepack exec` checks the credential before running the command and fails with exit code 3 and a clear action ([cli](../cli.md#nugetconfig-and-nuget-credentials)). |
| F-002 | 2026-10-17 | OP-036 candidate validation, SHA `88467a9` | Recheck dependency policy before promoting the signed candidate | Fresh `cargo deny check` exits 1 for unmaintained dependencies: `zvariant` (RUSTSEC-2026-0370), `zbus_names` (RUSTSEC-2026-0371), and `zvariant_derive` (RUSTSEC-2026-0372), through the Linux Secret Service keyring backend. These are maintenance advisories, not evidence of an exploit. | — | B | Fixed in `dbaf3be`, included in candidate `f84978b`: [new dry run 40900732673](https://github.com/Khr0x/onepack/actions/runs/40900732673), independent signature/checksums and downloaded Linux binary tests passed on 2026-10-29. These advisories are absent from the current check. OP-045 still awaits native macOS/Windows keychain acceptance; F-003/F-004 are fixed locally in `bc78dc4`, and F-005/OP-048 blocks the current gate. |
| F-003 | 2026-10-29 | OP-036 candidate validation, SHA `f84978b` | Recheck dependency policy before accepting the new signed artifact | Current cargo-deny fails on `rustls-webpki 0.103.15` (RUSTSEC-2026-0382), `webpki-roots 1.0.9` (0381) and `webpki-root-certs 1.0.9` (0383), concerning distrusted emSign intermediates. Passing functional TLS checks does not resolve these advisories; no exploitation of onepack was demonstrated. | — | B | Fixed locally in `bc78dc4`: `rustls-webpki` 0.103.17, `webpki-roots`/`webpki-root-certs` 1.0.12; these advisories are absent from the current check. OP-046 remains implemented pending a new candidate and signed artifact; do not reuse `f84978b`. |
| F-004 | 2026-10-29 | OP-036 candidate validation, SHA `f84978b` | Recheck dependency policy before accepting the new signed artifact | Current cargo-deny fails on `tokio 1.53.2` (RUSTSEC-2026-0386, unsound / AsyncFd I/O safety). An affected dependency and a failing policy gate are confirmed; exploitation of onepack was not tested or demonstrated. | — | B | Fixed locally in `bc78dc4`: Tokio 1.54.1; the advisory is absent from the current check. OP-047 remains implemented pending a new candidate and signed artifact. |
| F-005 | 2026-11-16 | OP-046/OP-047 validation, SHA `bc78dc4` | Recheck dependency policy after updating TLS and Tokio | Current cargo-deny fails on bundled `libsqlite3-sys 0.37.0` (RUSTSEC-2026-0459 / CVE-2026-119375). It enters through `sqlx-sqlite 0.9.0`; `sqlx 0.9.0` is the latest release and requires `libsqlite3-sys <0.38.0`, while the fix is >=0.39.0. This is a local file memory-exposure advisory; exploitation of onepack was not demonstrated. | — | B | Open: OP-048 in Obsidian. Resolve without ignores or policy changes, then validate a new candidate; `bc78dc4` is not promotable. |

### Current candidate validation — 2026-10-29

Candidate `f84978b` passed all nine jobs in [Release dry run 40900732673](https://github.com/Khr0x/onepack/actions/runs/40900732673).
The downloaded artifact's digest, seven archive checksums and Sigstore signature
were independently verified. Downloaded static ARM64 binaries passed acceptance
(including recovery of five byte-identical packages), Secret Service login/logout,
headless credentials and TLS checks in a fresh Ubuntu 24.04.5 / .NET 8.0.427
container. Current cargo-deny still exits 1 for F-003/F-004, so this is not an
accepted candidate. Evidence is recorded in Obsidian as
`OP-036-Resultados-2026-10-29`; temporary test resources were removed.

### Dependency gate update — 2026-11-16

Local commit `bc78dc4` updates the TLS chain (`rustls-webpki` 0.103.17,
`webpki-roots`/`webpki-root-certs` 1.0.12) and Tokio 1.54.1. `cargo fmt`,
Clippy, 151 tests and crate boundaries pass. A fresh `cargo deny check`
(advisory DB `02e2517`) no longer reports F-003/F-004, but still exits 1 for
F-005: bundled `libsqlite3-sys 0.37.0` via `sqlx 0.9.0`, which cannot use the
patched `libsqlite3-sys >=0.39.0`. No candidate is promoted and nothing was
pushed. Evidence is recorded in Obsidian as `OP-046-047-Resultados-2026-11-16`.

### F-002 follow-up — 2026-10-18

OP-045 has a local fix: the CLI now selects `keyring-core` stores directly and uses
the maintained DBus Secret Service backend on Unix. The three reported crates are
absent from the lockfile, and current `cargo deny check` passes without policy
exceptions. Clippy and all 151 workspace tests pass on both macOS ARM64 and Linux
ARM64. Static Linux ARM64 release binaries pass Secret Service login/logout,
credential compatibility with the previous CLI, headless and TLS checks, and the
MVP acceptance test (including recovery of five byte-identical packages).

F-002 remains open for candidate promotion: this is an uncommitted local change,
with no push or new signed artifact. OP-036 still needs CI and the full platform
matrix on the integrated candidate, followed by independent signature/checksum
verification and acceptance of its downloaded binaries. OP-045 evidence and file
hashes are recorded in Obsidian as `OP-045-Resultados-2026-10-18`.

<!-- Row template:
| F-00N | YYYY-MM-DD | person, machine or pipeline | | | | B/P | open |
-->

## Incident drill

See [the runbook](../runbooks/incident-drill.md). One row per incident.

| Incident | Date | Time to resolve | Was the runbook enough? | Deviations |
|---|---|---|---|---|
| 1. Leaked CI credential | | | | |
| 2. Vulnerable version | | | | |
| 3. Server lost | | | | |

## Retrospective

_Pending: summarised answers to the four questions in the [plan](README.md#retrospective)._
