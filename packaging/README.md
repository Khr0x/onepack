# Packaging

| Directory | Contents | Documentation |
|---|---|---|
| [`systemd/`](systemd/) | Hardened unit and example environment file. | [systemd installation](../docs/runbooks/install.md#2-systemd-service) |
| [`container/`](container/) | Rootless image on distroless, static binary. `Containerfile` builds from source; `Containerfile.release` uses the verified binaries and publishes `ghcr.io/khr0x/onepack`. | [Container](../docs/runbooks/install.md#3-container) |
| [`proxy/`](proxy/) | Nginx and Caddy with TLS, without serving blobs directly. | [Reverse proxy](../docs/runbooks/install.md#4-reverse-proxy-and-tls) |

The signed binaries for each version are published by the [`release.yml`](../.github/workflows/release.yml) workflow. Tests for these artifacts are in [`tests/ops/`](../tests/ops/) and run in CI (job *systemd, container and reverse proxy examples*).
