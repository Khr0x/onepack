# Roadmap

onepack is pre-1.0. The current milestone is a private NuGet registry that is
safe to operate, documented, and validated with a real pilot. The next milestone
extends the same registry to npm and PyPI.

## v1 — Private NuGet registry

- NuGet V3 publish, search, metadata, download, restore, unlist, and relist.
- Feeds, users, service accounts, roles, scoped publish patterns, expiring and
  revocable tokens.
- SQLite metadata, immutable content-addressed blobs, inspection limits, quotas,
  and version blocking.
- CLI administration, backup and restore, health checks, metrics, audit log,
  systemd, container, and reverse-proxy deployment.
- Pilot with a real team, incident exercise, accepted signed candidate, and the
  `v0.1.0` release.

Detailed phases and status: [`roadmap/roadmap-mvp.md`](../roadmap/roadmap-mvp.md).
Architecture decisions: [`roadmap/adr-mvp.md`](../roadmap/adr-mvp.md).

## v2 — NuGet, npm, and PyPI in one registry

- Shared identity, feeds, storage, and operations.
- Private npm hosting and PyPI hosting with per-ecosystem adapters.
- Mixed-ecosystem migration, isolation, and recovery validation.
- Python is mandatory in this milestone.

Design: [`roadmap/adr-v2.md`](../roadmap/adr-v2.md), [`roadmap/npm-v2.md`](../roadmap/npm-v2.md), and
[`roadmap/pypi-v2.md`](../roadmap/pypi-v2.md).

## Outside the current roadmap

SSO, a web UI, PostgreSQL, and high availability are independent proposals. They
enter the roadmap only if the project explicitly prioritizes them. A new
ecosystem also requires its own specification and acceptance criteria.

## How priorities change

Open an issue with the problem and the smallest proposed change. Large changes
need an ADR before implementation. Security and data-integrity fixes take
priority over new features.
