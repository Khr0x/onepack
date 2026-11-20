# Contributing to onepack

Thanks for taking the time to contribute. onepack is an open-source, self-hosted
package registry. Bug reports, documentation fixes, tests, and focused pull
requests are welcome.

## Language

All code, comments, documentation, commit messages, issue reports, and pull
request descriptions must be written in **English**. This keeps the project
accessible to contributors and users across ecosystems.

## Before you start

- For a reproducible bug, open an issue with the bug report template.
- For a larger change or a new feature, open an issue first so the scope can be
  agreed before implementation.
- For security issues, do **not** open a public issue. Follow
  [SECURITY.md](SECURITY.md).

## Development environment

Requirements:

- [rustup](https://rustup.rs) (the toolchain is pinned in
  [`rust-toolchain.toml`](rust-toolchain.toml)).
- Git.
- .NET SDK 8 or later for the NuGet conformance tests.
- Docker for the container and end-to-end checks (optional for small changes).

Clone and build:

```bash
git clone https://github.com/Khr0x/onepack.git
cd onepack
cargo build --workspace --locked
```

## Checks

Run the checks that match the change. Before opening a pull request, the
baseline is:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
./scripts/check-crate-deps.sh
cargo deny check
```

Integration and operational checks:

```bash
./scripts/e2e-cli.sh          # CLI against a local server
./scripts/e2e-acceptance.sh   # publish/restore, revocation, backup/restore
./tests/recovery/disk-full.sh # Linux, requires sudo and tmpfs
./tests/ops/container.sh      # container image and probes
./tests/ops/systemd.sh        # systemd hardening and restart
```

No test that was skipped or not run may be reported as passed.

## Code guidelines

- Keep crate boundaries from [`docs/conventions.md`](docs/conventions.md).
- Prefer the standard library and existing helpers over new dependencies.
- Add a test for non-trivial behavior, especially authentication, package
  parsing, storage, and recovery paths.
- Do not log tokens, credentials, private package contents, or data directory
  contents.
- Update documentation when behavior, configuration, or operations change.
- Add a `CHANGELOG.md` entry for user-visible changes.

## Commit messages

Use Conventional Commits, for example:

```text
feat(cli): add shell completion for fish
fix(server): reject duplicate package versions
docs: document backup retention
```

Keep commits focused. A pull request may contain more than one commit when the
changes are separable.

## Pull requests

- Link the related issue when one exists.
- Describe the user-visible result and the tests that were run.
- Call out breaking changes, migrations, security implications, and skipped
  checks.
- Keep the PR small enough to review; split unrelated changes.
- CI must be green before merge.

## License

By contributing, you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE), the same license as the project. No separate
contributor license agreement is required.

## Code of conduct

Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
