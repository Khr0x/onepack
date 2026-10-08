# onepack
One binary, one command, your entire private registry

Self-hosted private NuGet package registry, operated from the CLI.

> Status: in development (see the [roadmap](roadmap/roadmap-mvp.md)). Not ready for use yet.

## Documentation

- [MVP proposal](roadmap/mvp-registro-privado-paquetes-nuget.md)
- [Roadmap by phase](roadmap/roadmap-mvp.md)
- [Architecture decisions (ADR)](roadmap/adr-mvp.md)
- [Conventions](docs/conventions.md)
- [Verified clients](docs/compatibility.md)
- [Security: limits, quotas and version blocking](docs/security.md)
- [Guide: from zero to restore in CI](docs/guide-zero-to-ci.md)
- [`onepack` CLI](docs/cli.md), [CLI installation](docs/install-cli.md) and [API `/api/v1`](docs/api.md)
- Operations: [server installation](docs/runbooks/install.md), [backup and restore](docs/runbooks/backup-restore.md), [upgrade](docs/runbooks/upgrade.md), [incident drill](docs/runbooks/incident-drill.md), [release](docs/runbooks/release.md), [observability](docs/operations.md) and [performance](docs/performance.md)
- [Pilot](docs/pilot/README.md) and [friction log](docs/pilot/friction-log.md)

## Development

Requirements: `rustup` (the Rust version comes from `rust-toolchain.toml`) and the .NET 8 SDK for the conformance tests.

```bash
cargo build
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-features
./scripts/check-crate-deps.sh
cargo deny check
```

## Running the server

```bash
cargo run -p onepack-server -- init --data-dir ./data
cargo run -p onepack-server -- feed create internal --data-dir ./data
cargo run -p onepack-server -- serve --data-dir ./data --public-url http://127.0.0.1:8080
```

`init` writes the initial administrative credential to `./data/initial-admin-token`. Everything else is done with the CLI:

```bash
cargo run -p onepack-cli -- context add local --url http://127.0.0.1:8080
cargo run -p onepack-cli -- login --token-stdin < ./data/initial-admin-token
cargo run -p onepack-cli -- feed list
```

The [guide from zero to restore in CI](docs/guide-zero-to-ci.md) walks through the full flow: feed, CI account, token, permissions, publishing, `NuGet.Config` and restore. The feed lives at `http://127.0.0.1:8080/nuget/<feed>/v3/index.json`. Use HTTPS (direct or through a reverse proxy) anywhere other than local.

Inspection limits, per-feed quotas and version blocking (`onepackd feed quota`, `onepackd package block`) are described in [docs/security.md](docs/security.md). `onepackd serve --help` lists every limit.

## Layout

```text
crates/
  core/         domain
  nuget/        NuGet adapter
  storage/      SQLite and blobs
  api-client/   /api/v1 contracts
  server/       onepackd binary
  cli/          onepack binary
migrations/     SQL migrations
tests/          integration, conformance-dotnet, security, recovery
packaging/      systemd, container
docs/           technical documentation
```

## License

[Apache License 2.0](LICENSE).
