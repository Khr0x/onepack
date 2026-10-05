# Release

> Phase 8. How a onepack version is published. The workflow is [`release.yml`](../../.github/workflows/release.yml).
> Last updated: 2026-10-04

## What the workflow does

1. **Build**: release binaries for the server (Linux x86-64 and ARM64, static musl) and the CLI (Linux, macOS x86-64/ARM64, Windows). Each archive includes `README.md` and `LICENSE` (Apache-2.0). On a tag, it first checks that the tag matches the workspace version.
2. **Verify**: runs the [acceptance test](../../scripts/e2e-acceptance.sh) with the freshly built Linux x86-64 binaries (not a debug build): install, publish, restore from CI, revoke, block and recover from a backup.
3. **Publish**: `SHA256SUMS`, Sigstore *keyless* signature (`SHA256SUMS.sigstore.json`), verification of that signature and, when triggered by a tag, a **draft** release on GitHub.

If `verify` fails, nothing is signed or published.

## Dry run (no tag)

Before the first tag, and whenever the workflow changes:

1. GitHub → *Actions* → *Release* → *Run workflow* on `main`.
2. The binaries carry version `0.0.0-dev.<sha>` and are kept as run artifacts (`release`), with no GitHub release.
3. Download the `release` artifact and check the signature as in [installation](install.md#1-descargar-y-verificar), using `--certificate-identity "https://github.com/Khr0x/onepack/.github/workflows/release.yml@refs/heads/main"`.

## Publishing a version

1. `main` is green in CI.
2. Update `version` in the root `Cargo.toml` (for example, `0.1.0`) and `Cargo.lock` (`cargo check`) in a PR. The workflow rejects a tag that does not match that version.
3. After the merge, tag and push:

   ```bash
   git switch main && git pull
   git tag -a v0.1.0 -m "onepack 0.1.0"
   git push origin v0.1.0
   ```

4. When the workflow finishes, review the draft release (files, `SHA256SUMS`, signature bundle), write the notes and publish it.
5. Check the installation from the published release by following [installation](install.md) on a clean machine.

To repeat the acceptance test with downloaded binaries: unpack `onepack` and `onepackd` into a directory and run `ONEPACK_BIN_DIR=<dir> ./scripts/e2e-acceptance.sh` from the repository (requires the .NET 8 SDK).

## Before `v0.1.0`

- A green dry run.
