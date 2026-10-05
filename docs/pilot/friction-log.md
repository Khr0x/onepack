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
| F-001 | 2026-10-04 | Automated acceptance test | Restore from CI with a revoked token | `dotnet restore` only said `NU1301: unable to load the service index`, without mentioning the credential. The server did log `reason="revoked"`. | — | B | Fixed: `onepack exec` checks the credential before running the command and fails with exit code 3 and a clear action ([cli](../cli.md#nugetconfig-y-credenciales-de-nuget)). |

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
