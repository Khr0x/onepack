# Backup and restore

> Phase 7 ([ADR-017](../../roadmap/adr-mvp.md#adr-017)).
> Last updated: 2026-10-04

## What it includes

`onepackd backup` produces a directory with:

- `metadata.sqlite`: consistent copy of the database (`VACUUM INTO`, inside a read transaction);
- `blobs/sha256/…`: the blobs referenced by that copy, verified against their hash when copied;
- `manifest.json`: format and onepackd version, schema version, number of feeds and versions, and the SHA-256 and size of the database and of each blob.

**Does not include** the service configuration (`/etc/onepack/onepackd.env` or the container variables), TLS certificates, or the initial administrative credential. Store them separately; issued tokens do survive, because the database stores their verifiers.

## Making a backup

With the server running:

```bash
sudo -u onepack onepackd backup --data-dir /var/lib/onepack --output /var/backups/onepack/$(date +%Y%m%d-%H%M%S)
```

While it lasts, the registry is in **maintenance mode**:

- reads (restore, search, downloads) keep working;
- mutations (publish, unlist, block, administrative API) receive `503 MAINTENANCE` with `Retry-After: 30`. `dotnet nuget push` does not retry: a pipeline that publishes during the backup fails and must be rerun. Schedule backups outside publishing hours;
- orphan blob cleanup is postponed.

Maintenance ends when the backup finishes, also if it fails. If the process dies midway, it expires on its own (`--maintenance-timeout-secs`, 1 h by default) or is ended manually:

```bash
sudo -u onepack onepackd maintenance status --data-dir /var/lib/onepack
sudo -u onepack onepackd maintenance off --data-dir /var/lib/onepack
```

`backup` fails, without leaving anything half-done, if the destination is not empty, if another maintenance is already running, or if a source blob does not match its hash (in that case, run `onepackd check`).

Copy the resulting directory off the server (another disk, another machine, object storage). A `tar` is enough: the manifest allows verifying it later.

## Restore

In an **empty or nonexistent** data directory, with the service stopped:

```bash
sudo systemctl stop onepackd
sudo -u onepack onepackd restore --from /path/to/backup --data-dir /var/lib/onepack
sudo -u onepack onepackd check --data-dir /var/lib/onepack
sudo systemctl start onepackd
```

`restore`:

1. verifies the manifest, the database, and each blob against their hashes, **before** copying anything;
2. copies and verifies again;
3. if the backup is from an older version, applies migrations (with their prior backup, [ADR-020](../../roadmap/adr-mvp.md#adr-020));
4. checks the integrity of the result.

If something fails, it leaves the directory empty. It rejects a backup from a schema newer than the binary: upgrade onepackd first.

If the restored server has a different public URL, change `ONEPACK_PUBLIC_URL`: protocol URLs are always generated from it ([ADR-018](../../roadmap/adr-mvp.md#adr-018)).

## Checking integrity

```bash
sudo -u onepack onepackd check --data-dir /var/lib/onepack          # reads and verifies each blob
sudo -u onepack onepackd check --data-dir /var/lib/onepack --quick  # existence and size only
```

It reports:

- **missing** blobs: those versions cannot be downloaded;
- **corrupted** blobs: size or hash mismatch;
- orphans: files that no version references and that cleanup removes after the grace period.

It exits with a non-zero code if there are missing or corrupted blobs. Affected versions appear as `feed/Id@version`: restore their blobs from a backup by copying the corresponding `blobs/sha256/…` files.
