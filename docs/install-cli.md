# Installing the `onepack` CLI

> Phase 8. How to install, configure and update the CLI on a workstation, a server or a CI agent.
> For the server (`onepackd`), see [installation](runbooks/install.md).
> Last updated: 2026-11-17

## Quick install (Linux and macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/Khr0x/onepack/v0.1.0/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
onepack --version
```

The script detects the OS and architecture, downloads the signed release archive, verifies it against `SHA256SUMS`, and installs `onepack` in `~/.local/bin` by default. Useful variables:

```bash
curl -fsSL https://raw.githubusercontent.com/Khr0x/onepack/v0.1.0/install.sh | ONEPACK_VERSION=0.1.0 sh
curl -fsSL https://raw.githubusercontent.com/Khr0x/onepack/v0.1.0/install.sh | ONEPACK_INSTALL_DIR=/usr/local/bin sh
curl -fsSL https://raw.githubusercontent.com/Khr0x/onepack/v0.1.0/install.sh | ONEPACK_INSTALL_SERVER=1 sh
```

`ONEPACK_INSTALL_SERVER=1` also installs `onepackd` from its own release archive. The server is published for Linux only: on macOS the script fails instead of silently installing only the CLI, and a missing archive or checksum is an error.

To install in `/usr/local/bin`, run the script with sufficient permissions (`sudo`). The macOS binaries are not notarized yet; Gatekeeper may block them until the release process adds notarization. For a manual download, checksum/Sigstore verification, and shell completion, continue below.

## Supported platforms

| Platform | Binary | Status |
|---|---|---|
| Linux x86-64 | `x86_64-unknown-linux-musl` | ✅ Verified on every release: Ubuntu 24.04, Debian 12, Fedora 41, Rocky Linux 9, Alpine 3.20 |
| Linux ARM64 | `aarch64-unknown-linux-musl` | ✅ Verified on every release, same distributions |
| macOS (Apple Silicon, Intel) | `aarch64-apple-darwin`, `x86_64-apple-darwin` | ⏳ Built, not yet verified and not notarized |
| Windows x86-64 | `x86_64-pc-windows-msvc` | ⏳ Built, not yet verified and not code-signed |

The Linux binaries are static (musl): they do not depend on the glibc version and run on any distribution. "Verified" means the [release workflow](runbooks/release.md) runs [`scripts/e2e-cli-linux.sh`](../scripts/e2e-cli-linux.sh) against the published binary: system keychain, headless machine, TLS with the system trust store and with `--ca-cert`, and the distributions above.

## Install on Linux

Pick the version and the architecture (`uname -m`: `x86_64` or `aarch64`):

```bash
VERSION=0.1.0
TARGET=$(uname -m)-unknown-linux-musl
base=https://github.com/Khr0x/onepack/releases/download/v$VERSION
curl -fLO "$base/onepack-$VERSION-$TARGET.tar.gz"
curl -fLO "$base/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
```

Optionally, check that `SHA256SUMS` was produced by this repository's release workflow (needs [cosign](https://docs.sigstore.dev/cosign/system_config/installation/)):

```bash
curl -fLO "$base/SHA256SUMS.sigstore.json"
cosign verify-blob --bundle SHA256SUMS.sigstore.json \
  --certificate-identity "https://github.com/Khr0x/onepack/.github/workflows/release.yml@refs/tags/v$VERSION" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
```

Install it for every user (`/usr/local/bin`) or only for yours (`~/.local/bin`, already in the `PATH` on most distributions):

```bash
tar -xzf "onepack-$VERSION-$TARGET.tar.gz"
sudo install -m 0755 "onepack-$VERSION-$TARGET/onepack" /usr/local/bin/onepack
onepack --version
```

### Before the first release, or to build it yourself

With [rustup](https://rustup.rs):

```bash
cargo install --locked --git https://github.com/Khr0x/onepack onepack-cli
```

The binary goes to `~/.cargo/bin/onepack`.

## Shell completion

```bash
# bash
mkdir -p ~/.local/share/bash-completion/completions
onepack completion bash > ~/.local/share/bash-completion/completions/onepack
# zsh (a directory in your $fpath)
onepack completion zsh > "${fpath[1]}/_onepack"
# fish
onepack completion fish > ~/.config/fish/completions/onepack.fish
```

## Credentials on macOS and Windows

`onepack login` stores the token in the native system store: **macOS Keychain**
or **Windows Credential Manager**. `onepack logout` deletes it. The native
paths are covered by [`scripts/e2e-keychain-macos.sh`](../scripts/e2e-keychain-macos.sh)
and [`scripts/e2e-keychain-windows.ps1`](../scripts/e2e-keychain-windows.ps1).

## Credentials on Linux

The CLI never stores a token in plain text ([CLI credentials](cli.md#credentials)). Which option fits depends on the machine:

| Machine | How to give the CLI a token |
|---|---|
| Desktop with GNOME Keyring or KWallet (Secret Service) | `onepack login` stores it in the keychain. |
| Server or SSH session without a desktop | No Secret Service: `login` fails with `KEYCHAIN_UNAVAILABLE` (exit 9). Use `ONEPACK_TOKEN` or `--token-env VAR`, filled from your secrets manager. |
| CI agent or container | `ONEPACK_TOKEN` (or `--token-env`) from the pipeline secret; set `ONEPACK_KEYRING=off` so the CLI does not look for a keychain. |

`onepack doctor` shows where the token in use comes from (`system keychain` or the variable name), never the token itself.

## Servers with a private CA

The CLI trusts the system's certificate store. For a server whose certificate comes from an internal CA, either:

- add the CA to the system store (Debian/Ubuntu: copy it to `/usr/local/share/ca-certificates/` and run `sudo update-ca-certificates`; Fedora/RHEL: copy it to `/etc/pki/ca-trust/source/anchors/` and run `sudo update-ca-trust`), or
- give it to the CLI only: `onepack context add prod --url https://packages.example.com --ca-cert ./ca.pem` (stored in the context) or `--ca-cert` / `ONEPACK_CA_CERT` per command.

An untrusted certificate fails with `TLS_FAILED` (exit 7).

## Compatibility with the server

The CLI and the server share version numbers. Before each administrative command the CLI asks the server what it supports (`GET /api/v1/capabilities`):

- Same minor version (for example CLI 0.1.x and server 0.1.y): supported.
- One minor version apart in either direction: commands both sides support work; a command the server does not offer fails with `CAPABILITY_MISSING` (exit 8) and the server version.
- Before 1.0, a minor version may include an incompatible change; the release notes say so. Upgrade the CLI and the server together in that case.

## Update and uninstall

To update, install the new binary over the old one, the same way. Contexts and stored tokens are kept.

To uninstall, first remove the stored tokens, then the binary and the configuration:

```bash
onepack context list                    # for each context with a stored token:
onepack --context <name> logout
sudo rm /usr/local/bin/onepack          # or ~/.local/bin/onepack
rm -rf ~/.config/onepack                # or $ONEPACK_CONFIG_DIR / $XDG_CONFIG_HOME/onepack
```
