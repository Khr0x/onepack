# Security policy

## Supported versions

onepack is pre-1.0. Security fixes are applied to the latest published release
and to `main`. Older pre-release versions are not supported.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's private vulnerability
reporting:

<https://github.com/Khr0x/onepack/security/advisories/new>

Do **not** open a public issue for a suspected vulnerability. Include:

- Affected version or commit.
- Component (server, CLI, package parsing, authentication, storage, release
  artifact, container image).
- Reproduction steps or a proof of concept.
- Impact and any known mitigations.
- Whether the report may be credited publicly.

## Response

Reports are reviewed on a best-effort basis. The maintainer will acknowledge a
complete report within 7 days when possible, keep the reporter informed of the
assessment, and coordinate disclosure after a fix is available.

## Scope

Security-relevant areas include:

- Authentication, tokens, grants, and feed isolation.
- NuGet package parsing, validation, and decompression limits.
- Upload, download, backup, restore, and recovery paths.
- Secret handling in the CLI, logs, configuration, and CI.
- Release artifacts, checksums, Sigstore signatures, and the container image.

General hardening questions and non-sensitive bugs can use the public issue
tracker.

## Release verification

Published binaries include `SHA256SUMS` and a Sigstore bundle. See
[`docs/runbooks/install.md`](docs/runbooks/install.md) for verification steps.
