<p align="center">
  <img src="docs/assets/logo.png" alt="onepack" width="480">
</p>

<p align="center">
  <strong>An open-source, self-hosted registry for private packages.</strong>
</p>

<p align="center">
  <a href="https://github.com/Khr0x/onepack/actions/workflows/ci.yml"><img src="https://github.com/Khr0x/onepack/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI status"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-4fa8f5?style=flat-square" alt="License: Apache-2.0"></a>
  <a href="rust-toolchain.toml"><img src="https://img.shields.io/badge/rust-1.96.1-f5b512?style=flat-square" alt="Rust 1.96.1"></a>
</p>

<p align="center">
  <a href="#quick-start">Try it from source</a> &middot;
  <a href="#documentation">Docs</a> &middot;
  <a href="docs/cli.md">CLI reference</a> &middot;
  <a href="#contributing">Contributing</a>
</p>

onepack brings package hosting, access control, and administration into a registry you run on your own infrastructure. Written in Rust, it pairs a standalone server (`onepackd`) with a command-line client (`onepack`) for local development and CI workflows.

**Available today:** NuGet / .NET. **Planned:** npm / JavaScript and PyPI / Python. See the [roadmap](docs/roadmap.md).

> **Development status:** `v0.1.0` has yet to be published. The project is experimental, and interfaces and behavior may change before 1.0.

## Quick start

### Build from source

On Linux or macOS, install Git, [rustup](https://rustup.rs), and curl. Build the server and CLI with the repository's pinned Rust toolchain.

```bash
git clone https://github.com/Khr0x/onepack.git &&
  cd onepack &&
  cargo build --release --locked -p onepack-server -p onepack-cli
```

From the checkout, initialize a temporary data directory and start the registry on loopback.

```bash
demo_dir="$(mktemp -d "${TMPDIR:-/tmp}/onepack-demo.XXXXXX")" &&
  ./target/release/onepackd init --data-dir "$demo_dir/data" &&
  ./target/release/onepackd serve --data-dir "$demo_dir/data" \
    --listen 127.0.0.1:8080 --public-url http://127.0.0.1:8080
```

Leave the server running and verify that it responds in another terminal.

```bash
curl -fsS http://127.0.0.1:8080/readyz
```

When you're ready to publish and restore a sample NuGet package, stop this server with `Ctrl+C` and follow the [from-source walkthrough](docs/guide-zero-to-ci.md). The walkthrough uses the same port and requires the .NET SDK 8 or later.

### Install the CLI

If you already have a registry to connect to, install the [pinned Rust toolchain](rust-toolchain.toml) with rustup, then build and install the `onepack` CLI from Git.

```bash
cargo +1.96.1 install --locked --git https://github.com/Khr0x/onepack onepack-cli
```

The commands below illustrate proposed CLI packages. These package names are provisional, and the packages are pending publication.

| Channel | Example command |
| --- | --- |
| [Homebrew](https://docs.brew.sh/Taps) (macOS) | `brew install khr0x/tap/onepack` |
| [Cargo](https://doc.rust-lang.org/cargo/commands/cargo-install.html) (crates.io) | `cargo install --locked onepack-cli` |
| [WinGet](https://learn.microsoft.com/en-us/windows/package-manager/winget/install) (Windows) | `winget install --id Khr0x.Onepack --exact --source winget` |

To install the `onepackd` server, use the [release deployment instructions](#installation-from-a-release) once `v0.1.0` is published.

### Installation from a release

Check [GitHub Releases](https://github.com/Khr0x/onepack/releases) for availability. The Docker workflow below requires the `v0.1.0` release and its container image. For other deployment options, see the [CLI installation guide](docs/install-cli.md) and [server installation guide](docs/runbooks/install.md).

<details>
<summary>Docker quick start for v0.1.0 (planned)</summary>

The commands use a bash-compatible shell and bind the local registry to `127.0.0.1`. Use HTTPS for deployments beyond local development.

### 1. Start the registry

```bash
curl -fsSLO https://raw.githubusercontent.com/Khr0x/onepack/v0.1.0/compose.yaml
docker compose run --rm onepackd init
docker compose up -d
```

Initialization creates the data volume and writes the initial administrative token to `/data/initial-admin-token` in the container. The release image is distroless and has no shell utilities; `onepackd initial-token` reads and removes the file.

### 2. Install the CLI and create a feed

```bash
curl -fsSL https://raw.githubusercontent.com/Khr0x/onepack/v0.1.0/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"

unset ONEPACK_CONTEXT ONEPACK_URL ONEPACK_TOKEN
if ! onepack context add local --url http://127.0.0.1:8080 --use; then
  echo "Could not select the local context; keeping /data/initial-admin-token." >&2
elif docker compose exec -T onepackd /usr/local/bin/onepackd initial-token print --data-dir /data \
     | onepack login --token-stdin; then
  onepack whoami \
    && onepack feed create internal \
    && docker compose exec onepackd /usr/local/bin/onepackd initial-token remove --data-dir /data
else
  echo "Login failed; keeping /data/initial-admin-token for the headless flow." >&2
fi
```

Clearing the inherited variables keeps the guide on `local`; `--use` also makes it the current context. If saving the context fails, the block stops before reading the token. `login` stores the credential in the system keychain. The removal is inside the successful branch, so it depends on the login that just ran, not on a session that was already stored.

On a headless machine without a keychain, supply the token through the environment instead:

```bash
unset ONEPACK_CONTEXT ONEPACK_URL
if ! onepack context add local --url http://127.0.0.1:8080 --use; then
  echo "Could not select the local context; keeping /data/initial-admin-token." >&2
elif ONEPACK_TOKEN="$(docker compose exec -T onepackd /usr/local/bin/onepackd initial-token print --data-dir /data)"; then
  export ONEPACK_TOKEN
  onepack whoami
  onepack feed create internal
else
  echo "Could not read the initial token; keeping /data/initial-admin-token." >&2
fi
```

Store `ONEPACK_TOKEN` in a secrets manager before treating it as saved. Once it is saved, remove the initial file:

```bash
docker compose exec -T onepackd /usr/local/bin/onepackd initial-token remove --data-dir /data
```

Use a dedicated service identity with limited permissions for CI.

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

For service accounts, restricted publishing, and pipeline examples, follow the [complete publish-and-restore guide](docs/guide-zero-to-ci.md).

</details>

### Production deployment (planned for v0.1.0)

This single-host example uses the `v0.1.0` release image once it is published. The registry's host port is bound to loopback, and its data lives in a persistent Docker volume. Nginx serves HTTPS and applies per-client rate limits.

Use a Linux host with Docker Compose and Nginx managed by systemd. Point your registry's DNS name at the host, make ports 80 and 443 reachable, and provision a TLS certificate and key. The [Nginx config](packaging/proxy/nginx.conf) uses HTTP/2 syntax supported by [Nginx 1.25.1 and later](https://nginx.org/en/docs/http/ngx_http_v2_module.html#http2).

Download the deployment files into a new directory.

```bash
base=https://raw.githubusercontent.com/Khr0x/onepack/v0.1.0
curl -fsSLo compose.yaml "$base/compose.yaml" &&
  curl -fsSLo onepack.nginx.conf "$base/packaging/proxy/nginx.conf"
```

Save the following as `.env`. Replace `packages.example.com` here and in `onepack.nginx.conf`. Update the two certificate paths in that config to match your host.

```dotenv
ONEPACK_VERSION=v0.1.0
ONEPACK_PUBLIC_URL=https://packages.example.com
```

Save this as `compose.override.yaml` in the same directory. Nginx handles per-client rate limiting, so the registry can accept traffic from the proxy without counting all clients as one IP.

```yaml
services:
  onepackd:
    environment:
      ONEPACK_IP_RATE_LIMIT: "0"
```

Initialize the volume, start the registry, then enable and check the HTTPS endpoint.

```bash
docker compose pull &&
  docker compose run --rm onepackd init &&
  docker compose up -d --wait &&
  sudo install -m 0644 onepack.nginx.conf /etc/nginx/conf.d/onepack.conf &&
  sudo nginx -t &&
  sudo systemctl reload nginx &&
  curl -fsS https://packages.example.com/readyz
```

Transfer the initial administrator token directly to your secrets manager using `docker compose exec -T onepackd /usr/local/bin/onepackd initial-token print --data-dir /data`. With the [CLI installed](docs/install-cli.md) on your admin workstation, export the token from the secrets manager as `ONEPACK_TOKEN` and create the first feed.

```bash
unset ONEPACK_CONTEXT ONEPACK_URL
onepack context add production --url https://packages.example.com --use &&
  onepack whoami &&
  onepack feed create internal
```

After confirming access and storing the credential, remove the bootstrap file with `docker compose exec -T onepackd /usr/local/bin/onepackd initial-token remove --data-dir /data`. Configure [scoped CI credentials](docs/guide-zero-to-ci.md#4-create-the-feed-and-the-ci-account) for publishing and [back up the registry](docs/runbooks/backup-restore.md) before relying on it.

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
| NuGet / .NET | Available. Publish, search, inspect metadata, download, restore, unlist, and relist. See the [client compatibility matrix](docs/compatibility.md). |
| npm / JavaScript | Planned for [v2](docs/roadmap.md#v2--nuget-npm-and-pypi-in-one-registry). |
| PyPI / Python | Planned for [v2](docs/roadmap.md#v2--nuget-npm-and-pypi-in-one-registry). |

## Documentation

| Topic | Guide |
| --- | --- |
| Getting started | [Publish and restore packages locally and in CI](docs/guide-zero-to-ci.md) |
| CLI | [Installation](docs/install-cli.md) · [Command reference and credentials](docs/cli.md) |
| Architecture | [Components, data flow, and deployment](docs/architecture.md) · [ADRs](docs/adr/README.md) |
| API | [Administration API reference](docs/api.md) |
| Compatibility | [NuGet resources and verified clients](docs/compatibility.md) |
| Versioning | [SemVer, compatibility, and support](docs/versioning.md) |
| Roadmap | [v1, v2, and scope](docs/roadmap.md) |
| Deployment | [Server, systemd, containers, and TLS](docs/runbooks/install.md) |
| Security | [Package inspection, quotas, rate limits, and version blocking](docs/security.md) |
| Operations | [Health checks, metrics, and logging](docs/operations.md) |
| Data management | [Backup and restore](docs/runbooks/backup-restore.md) · [Upgrades](docs/runbooks/upgrade.md) |
| Troubleshooting | [Incident response](docs/runbooks/incident-drill.md) · [Performance](docs/performance.md) |

For connection and permission issues, start with:

```bash
onepack doctor --feed internal
```

## Project policies

- [Contributing](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security policy](SECURITY.md)
- [Support](SUPPORT.md)
- [Maintainers](MAINTAINERS.md)
- [Changelog](CHANGELOG.md)

## Contributing

Bug reports, documentation improvements, and focused pull requests are welcome. [Open an issue](https://github.com/Khr0x/onepack/issues) to report a reproducible problem or discuss a larger change before implementing it.

For bug reports, include the onepack version, operating system, reproduction steps, and relevant output with credentials removed.

Start with the [source build](#build-from-source). Install the .NET 8 SDK for the conformance tests, then run:

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
