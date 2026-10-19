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
| F-002 | 2026-10-17 | OP-036 candidate validation, SHA `88467a9` | Recheck dependency policy before promoting the signed candidate | Fresh `cargo deny check` exits 1 for unmaintained dependencies: `zvariant` (RUSTSEC-2026-0370), `zbus_names` (RUSTSEC-2026-0371), and `zvariant_derive` (RUSTSEC-2026-0372), through the Linux Secret Service keyring backend. These are maintenance advisories, not evidence of an exploit. | — | B | Open: dependency-policy gate blocks promotion; tracked as OP-045 in Obsidian. [Release dry run](https://github.com/Khr0x/onepack/actions/runs/39068572157), signature and downloaded Linux binary tests passed; they do not override this failure. |

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
