# Changelog

All notable changes to onepack are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Before 1.0, minor versions may
include incompatible changes; release notes call them out.

## [Unreleased]

### Added

- Open-source quick start: multi-arch `ghcr.io/khr0x/onepack` image,
  checksum-verifying `install.sh`, and `compose.yaml`.
- Community health files, issue and pull request templates, and Dependabot
  configuration.

### Changed

- Project documentation, code comments, scripts, and test messages are now
  English-only.
- Updated `rustls-webpki`, `webpki-roots`, `webpki-root-certs`, and Tokio.
- Temporarily pinned `sqlx 0.9.1` from its release branch while
  `libsqlite3-sys 0.39` is unavailable on crates.io.

### Fixed

- RUSTSEC-2026-0381, RUSTSEC-2026-0382, RUSTSEC-2026-0383, and
  RUSTSEC-2026-0386.
- RUSTSEC-2026-0459 (bundled SQLite) via `libsqlite3-sys 0.39.1`.

## [0.1.0] - TBD

First public release. The version is not published yet; the entry will be
completed when the release is tagged.
