# Changelog

All notable changes to onepack are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Before 1.0, minor versions may
include incompatible changes; release notes call them out.

Release notes for a tag are generated from the matching version section by
[`scripts/changelog-notes.sh`](scripts/changelog-notes.sh).

<!-- Maintainers: move Unreleased entries to the version section and replace TBD with the tag date before publishing. -->

## [Unreleased]

### Fixed

- The quick start reads and removes the initial administrative credential with
  `onepackd initial-token`, which works inside the distroless image.
- `install.sh` installs `onepackd` when `ONEPACK_INSTALL_SERVER=1`, verifies its
  own release archive, and fails if the archive or checksum is missing. The
  server is Linux-only.
- The quick start pins the `local` context with `--use`, clears inherited
  `ONEPACK_CONTEXT`/`ONEPACK_URL`, and removes the initial credential only when
  the login that just ran succeeds.
- The container runbook installs the CLI and selects the Compose registry before
  login, so a fresh profile no longer fails with `CONTEXT_MISSING`.
- `install.sh` validates both archives before replacing either binary, so a
  broken server archive cannot leave a half-updated installation.

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
- RUSTSEC-2026-0592 via the transitive `url` dependency, updated to 2.5.9.
