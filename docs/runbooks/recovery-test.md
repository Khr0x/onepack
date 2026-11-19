# Recovery test

> Phase 7 gate: backup on one server, restore on **other infrastructure**, and `dotnet restore` of a project against the restored server ([ADR-017](../../roadmap/adr-mvp.md#adr-017)).
> Last updated: 2026-10-04

## Automated (CI)

[`scripts/e2e-recovery.sh`](../../scripts/e2e-recovery.sh) splits the two halves, and CI runs them on different machines and operating systems:

| Job | Machine | What it does |
|---|---|---|
| `Recovery (1/2) backup on Linux` | `ubuntu-24.04` | Publishes the fixtures with `dotnet pack` and `onepack package push`, makes the backup **with the server running**, checks that maintenance ends and writes are accepted again, and runs `onepackd check`. Uploads the backup as an artifact. |
| `Recovery (2/2) restore on macOS` | `macos-15` | Downloads the backup and restores it into an empty directory (and checks that it does not restore over existing data). Runs `check`, starts with **a different public URL**, and verifies that the origin's CI token still works. Runs `onepack nuget init` and `onepack exec -- dotnet restore` with empty caches, runs the consumer, and compares the downloaded packages byte for byte with those published at the origin. |

In local mode, `scripts/e2e-recovery.sh` with no arguments runs both halves on the same machine.

## Manual (real infrastructure)

To repeat it with your own servers:

1. **Origin**: with the service running, `onepackd backup --data-dir /var/lib/onepack --output /tmp/onepack-backup` and `tar -czf onepack-backup.tar.gz -C /tmp onepack-backup`.
2. Copy the file to a **different machine** (another host, network, or provider), with onepackd installed ([installation](install.md)).
3. **Destination**: `tar -xzf onepack-backup.tar.gz`, `onepackd restore --from onepack-backup --data-dir /var/lib/onepack`, `onepackd check --data-dir /var/lib/onepack`. Configure `ONEPACK_PUBLIC_URL` with the destination URL and start the service.
4. From a workstation with a project that depends on packages from the registry: `onepack context add destination --url <URL>`, `onepack nuget init --feed <feed> --pattern '<Prefix>.*'`, empty the caches (`dotnet nuget locals all --clear`), and run `onepack exec --feed <feed> -- dotnet restore`.
5. Record the result below.

## Record

| Date | Origin | Destination | Versions / blobs | Backup / restore duration | `dotnet restore` | Owner |
|---|---|---|---|---|---|---|
| 2026-10-04 | macOS (local) | macOS (local, another directory and URL) | 5 / 5 | < 1 s / < 1 s | ✅ | automated |
| | | | | | | |
