# Changelog

All notable changes to onepack are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Before 1.0, minor versions may
include incompatible changes; release notes call them out.

Release notes for a tag are generated from the matching version section by
[`scripts/changelog-notes.sh`](scripts/changelog-notes.sh).

<!-- Maintainers: move Unreleased entries to the version section and replace TBD with the tag date before publishing. -->

## [Unreleased]

No changes yet.

## [0.1.0] - TBD

First public release.

### Added

- Private NuGet V3 registry: publish, search, metadata, download, restore,
  unlist, and relist.
- Feeds, users, service accounts, per-feed roles, optional publish patterns,
  expiring tokens, and immediate revocation.
- SQLite metadata, immutable content-addressed blobs, package inspection,
  quotas, version blocking, audit log, and integrity checks.
- CLI administration, credential contexts, `onepack exec`, and `nuget init`.
- Backup, restore, recovery, health checks, Prometheus metrics, systemd and
  container deployment examples.
- Open-source quick start: multi-arch `ghcr.io/khr0x/onepack` image,
  checksum-verifying `install.sh`, and `compose.yaml`.
- Community health files, issue and pull request templates, and Dependabot
  configuration.

### Changed

- Project documentation, code comments, scripts, and test messages are now
  English-only.
- Updated `rustls-webpki`, `webpki-roots`, `webpki-root-certs`, and Tokio.
- Updated `sqlx` to 0.9.1 and `libsqlite3-sys` to 0.39.1.

### Fixed

- RUSTSEC-2026-0381, RUSTSEC-2026-0382, RUSTSEC-2026-0383, and
  RUSTSEC-2026-0386.
- RUSTSEC-2026-0459 (bundled SQLite) via `libsqlite3-sys` 0.39.1.
