# Pilot

> Phase 8. Plan to validate the MVP with a real team before tagging `v0.1.0`.
> Last updated: 2026-10-04

The pilot answers one question: **can a team that did not write onepack install it, use it in their daily work and recover from an incident using only the documentation?** Anything that prevents this is an MVP blocker.

The [decisive test](../../roadmap/roadmap-mvp.md#prueba-decisiva-del-mvp) already runs automatically on every change ([`scripts/e2e-acceptance.sh`](../../scripts/e2e-acceptance.sh)). The pilot checks what a script cannot: that people complete it without help, on their own infrastructure and with their own projects.

## Pilot team

Requirements:

- At least one in-house .NET library that is currently shared some other way (folder, copy, another registry).
- At least one CI pipeline that restores that library.
- One person who operates the server (installs it, takes backups) and who did **not** write onepack's code.
- A Linux server or a container host ([requirements](../runbooks/install.md)).

Record who takes part, the agreed period and the environment in [the log](friction-log.md#pilot-details).

## Schedule

| Week | What | Outcome |
|---|---|---|
| 0 | Preparation: release candidate (see [release](../runbooks/release.md)), 30-minute kickoff meeting. | Signed and verified binaries available. |
| 1 | **Installation using only the documentation**: [installation](../runbooks/install.md) and the [zero to CI restore guide](../guide-zero-to-ci.md). Nobody from the project helps unless the team is stuck; if that happens, it is a friction. | Server running, feed created, library published from CI and restored in another pipeline. |
| 1 | IDE check: Visual Studio and Rider ([steps](../compatibility.md#verificación-manual-ide)). | IDE rows filled in `docs/compatibility.md`. |
| 1–4 | **Regular use**: publish new versions from CI, restore in builds and on workstations, rotate a token. Daily scheduled backup ([backup](../runbooks/backup-restore.md)). | Frictions logged as they happen. |
| 2–3 | **Incident drill** ([runbook](../runbooks/incident-drill.md)): revoke the CI credential, block a version and restore from a backup, timed. | Times and deviations recorded in the log. |
| end | Retrospective (see below) and final classification of frictions. | List of MVP blockers closed or scheduled. |

Recommended period: 2–4 weeks. Less than two weeks does not cover enough publishing cycles or a real backup being restored.

## What to log

Everything goes in [friction-log.md](friction-log.md). A friction is any moment when someone:

- had to ask something the documentation should have answered;
- saw an error that did not explain what to do;
- had to edit a file by hand, open the database or read the code;
- lost more than 10 minutes on a step;
- found behaviour different from what is documented.

There is no need to propose a fix: what they were trying to do, what happened and how long it took is enough.

## Classification

| Class | Criteria | What happens |
|---|---|---|
| **MVP blocker** | Data loss or corruption; access across feeds; the decisive test cannot be completed without help, without a UI or without touching the database; a security bug. | Fixed before `v0.1.0`. |
| **Post-MVP** | Everything else: convenience, performance within targets, new features. | Goes to the retrospective and the evolution order. |

## Exit criteria

The pilot succeeds when, in addition to the [MVP exit criteria](../../roadmap/roadmap-mvp.md#criterios-de-cierre-del-mvp):

- the team installed and published without direct help (or every bit of help is logged and fixed in the documentation);
- the incident drill was completed and all three incidents were resolved with the runbooks;
- at least one scheduled backup was restored on another machine;
- no **MVP blocker** friction remains open.

## Retrospective

A 45-minute session at the end with these questions, with the answers summarised in the log:

1. What would you do differently if you installed it again tomorrow?
2. Which step in the documentation was unnecessary or missing?
3. What would make you stop using it?
4. From the [out-of-MVP list](../../roadmap/roadmap-mvp.md#fuera-del-mvp-referencia) (credential provider, S3, nuget.org proxy, npm…), what did you miss first?

The answer to question 4 updates the evolution order in the roadmap.
