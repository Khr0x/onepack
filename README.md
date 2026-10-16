<p align="center">
  <img src="docs/assets/logo.png" alt="onepack" width="480">
</p>

<p align="center">
  One private registry. Every package ecosystem.
</p>

<p align="center">
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/version-0.1.0-f5b512?style=flat-square" alt="Version 0.1.0"></a>
  <a href="https://github.com/Khr0x/onepack/actions/workflows/ci.yml"><img src="https://github.com/Khr0x/onepack/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI status"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-4fa8f5?style=flat-square" alt="License: Apache-2.0"></a>
  <a href="rust-toolchain.toml"><img src="https://img.shields.io/badge/rust-1.96.1-f5b512?style=flat-square" alt="Rust 1.96.1"></a>
</p>

<p align="center">
  <a href="#documentation">Docs</a> &middot;
  <a href="#quick-start">Quick start</a> &middot;
  <a href="docs/cli.md">CLI reference</a> &middot;
  <a href="#contributing">Contributing</a>
</p>

# onepack

**A self-hosted package registry built for teams working across languages and ecosystems.**

onepack brings package hosting, access control, and administration into a registry you run on your own infrastructure. Written in Rust, it pairs a standalone server (`onepackd`) with a command-line client (`onepack`) for local development and CI workflows.

The project is being built as a **multi-ecosystem registry**, starting with NuGet. NuGet is currently the implemented package protocol; support for additional package ecosystems is planned.

> **Development status:** onepack is under active development and is not yet ready for production use. Interfaces and behavior may change before 1.0.

## Features

- **Self-hosted storage:** SQLite metadata and package blobs on disk, with no external database service required.
- **Private feeds:** organize packages into separate feeds with reader, publisher, and maintainer roles.
- **Scoped access:** user and service identities, expiring and revocable tokens, and package-name restrictions for publishing.
- **Package integrity:** immutable versions, upload validation, feed quotas, and version blocking.
- **CLI administration:** manage feeds, credentials, permissions, and packages; use JSON output in scripts.
- **CI integration:** publish packages and restore dependencies with credentials supplied through environment variables.
- **Operational tooling:** audit logs, backup and restore, health checks, and Prometheus metrics.
- **Flexible deployment:** run the server directly, under systemd, or in a container behind a TLS reverse proxy.

## Package ecosystem support

| Ecosystem | Availability |
| --- | --- |
| NuGet / .NET | Implemented: publish, search, metadata, download, restore, unlist, and relist. See the [client compatibility matrix](docs/compatibility.md) for verified clients and limitations. |
| Additional ecosystems | Planned; not available in the current implementation. |

## Installation

### Build from source

Install [rustup](https://rustup.rs) and Git. The repository pins its Rust toolchain in [`rust-toolchain.toml`](rust-toolchain.toml).

```bash
git clone https://github.com/Khr0x/onepack.git
cd onepack
cargo build --release -p onepack-server -p onepack-cli
export PATH="$PWD/target/release:$PATH"

onepackd --version
onepack --version
```

`onepackd` runs the registry. Developer workstations and CI agents only need `onepack` when using its administration or credential helpers; NuGet clients connect directly to the registry.

For binary installation and deployment options, see the [CLI installation guide](docs/install-cli.md) and [server installation guide](docs/runbooks/install.md).

## Quick start

The following example runs a local registry from the repository directory after building the binaries. The commands use a bash-compatible shell.

### 1. Initialize and start the registry

```bash
onepackd init --data-dir ./data
onepackd serve --data-dir ./data --public-url http://127.0.0.1:8080
```

Keep the server running. Initialization creates the data directory and writes the initial administrative token to `./data/initial-admin-token`.

### 2. Connect the CLI and create a feed

In another terminal, from the same repository directory:

```bash
export PATH="$PWD/target/release:$PATH"
onepack context add local --url http://127.0.0.1:8080
onepack login --token-stdin < ./data/initial-admin-token
onepack whoami
onepack feed create internal
```

`login` stores the credential in the system keychain. On a headless machine without a keychain, supply it through the environment instead:

```bash
export ONEPACK_TOKEN="$(cat ./data/initial-admin-token)"
onepack whoami
onepack feed create internal
```

Once the credential is safely stored in the keychain or a secrets manager, remove the initial token file. Use a dedicated service identity with limited permissions for CI.

### 3. Publish a NuGet package

With the .NET SDK installed, create and publish a sample package:

```bash
dotnet new classlib -n Example.Library -o /tmp/onepack-example/Example.Library
dotnet pack /tmp/onepack-example/Example.Library -c Release \
  -o /tmp/onepack-example/packages -p:Version=1.0.0
onepack package push --feed internal \
  /tmp/onepack-example/packages/Example.Library.1.0.0.nupkg
onepack package list --feed internal
```

### 4. Restore from the registry

```bash
dotnet new console -n Example.Consumer -o /tmp/onepack-example/Example.Consumer
cd /tmp/onepack-example/Example.Consumer
onepack nuget init --feed internal --pattern 'Example.*' --yes
dotnet add package Example.Library --version 1.0.0 --no-restore
onepack exec --feed internal -- dotnet restore
dotnet build --no-restore
```

`onepack nuget init` configures the package source and source mapping without writing credentials to `NuGet.Config`. `onepack exec` supplies credentials to the child process for restore.

The feed's NuGet service index is:

```text
http://127.0.0.1:8080/nuget/internal/v3/index.json
```

Use HTTPS for deployments beyond local development. For service accounts, restricted publishing, and pipeline examples, follow the [complete publish-and-restore guide](docs/guide-zero-to-ci.md).

## Documentation

| Topic | Guide |
| --- | --- |
| Getting started | [Publish and restore packages locally and in CI](docs/guide-zero-to-ci.md) |
| CLI | [Installation](docs/install-cli.md) · [Command reference and credentials](docs/cli.md) |
| API | [Administration API reference](docs/api.md) |
| Compatibility | [NuGet resources and verified clients](docs/compatibility.md) |
| Deployment | [Server, systemd, containers, and TLS](docs/runbooks/install.md) |
| Security | [Package inspection, quotas, rate limits, and version blocking](docs/security.md) |
| Operations | [Health checks, metrics, and logging](docs/operations.md) |
| Data management | [Backup and restore](docs/runbooks/backup-restore.md) · [Upgrades](docs/runbooks/upgrade.md) |
| Troubleshooting | [Incident response](docs/runbooks/incident-drill.md) · [Performance](docs/performance.md) |

For connection and permission issues, start with:

```bash
onepack doctor --feed internal
```

## Contributing

Bug reports, documentation improvements, and focused pull requests are welcome. [Open an issue](https://github.com/Khr0x/onepack/issues) to report a reproducible problem or discuss a larger change before implementing it.

For bug reports, include the onepack version, operating system, reproduction steps, and relevant output with credentials removed.

To work on the project, install rustup and the .NET 8 SDK for the conformance tests, then run:

```bash
cargo build --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
./scripts/check-crate-deps.sh
```

Dependency checks also run in CI. With `cargo-deny` installed, run `cargo deny check` locally. See the [project conventions](docs/conventions.md) for shared interfaces and error handling.

## License

onepack is licensed under the [Apache License 2.0](LICENSE).
