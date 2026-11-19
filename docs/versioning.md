# Versioning and compatibility

onepack uses [Semantic Versioning](https://semver.org/). The workspace version
is the release version and is shared by the server and the CLI.

## Before 1.0

The project is pre-1.0. Minor releases may include incompatible changes when
the alternative is a security or data-integrity problem. Release notes call out
breaking changes, required migrations, and compatibility limits.

- Patch releases contain fixes and compatible dependency updates.
- Minor releases may add features or change interfaces.
- There is no promise of long-term support for pre-1.0 versions. Security fixes
  target the latest published release and `main`.

## CLI and server compatibility

The CLI checks `GET /api/v1/capabilities` before administrative commands.

- Same minor version: supported.
- One minor version apart: commands supported by both sides work; a missing
  command fails with `CAPABILITY_MISSING` and the server version.
- Before 1.0, a minor version may be incompatible. Upgrade the CLI and server
  together when the release notes say so.

See [`install-cli.md`](install-cli.md#compatibility-with-the-server).

## API and data compatibility

- `/api/v1` error codes and JSON output are treated as contracts; snapshots are
  checked in tests.
- NuGet behavior follows the protocol and the verified client matrix.
- Database migrations are forward-only and take a backup before changing an
  existing data directory.
- Backups are restored into an empty data directory after manifest verification.

## Release artifacts

A release publishes:

- Static Linux binaries, macOS binaries, and a Windows CLI archive.
- `SHA256SUMS` and its Sigstore bundle.
- A multi-arch container image tagged `vX.Y.Z` and `latest`.

Verify checksums and the signature as described in
[the installation runbook](runbooks/install.md#1-download-and-verify).

## Support

The latest release and `main` are supported before 1.0. Security reports go
through [SECURITY.md](../SECURITY.md); other issues use the
[issue tracker](https://github.com/Khr0x/onepack/issues).
