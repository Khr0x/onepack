# Version upgrade

> Phase 7 ([ADR-020](../../roadmap/adr-mvp.md#adr-020)). Forward-only migrations.
> Last updated: 2026-10-04

1. Full **backup** ([backup](backup-restore.md)):

   ```bash
   sudo -u onepack onepackd backup --data-dir /var/lib/onepack --output /var/backups/onepack/pre-upgrade
   ```

2. **Install** the new binary, verifying its signature ([installation](install.md#1-download-and-verify)).

3. **Check** which migrations it will apply, without changing anything:

   ```bash
   sudo -u onepack onepackd migrate --check --data-dir /var/lib/onepack
   ```

4. **Stop the service, migrate, and start**:

   ```bash
   sudo systemctl stop onepackd
   sudo -u onepack onepackd migrate --data-dir /var/lib/onepack
   sudo systemctl start onepackd
   curl -fs http://127.0.0.1:8080/readyz
   ```

   `migrate` first saves a copy of the database in `/var/lib/onepack/backups/`. A server with pending migrations does not start: it indicates that `migrate` must be run.

5. **Verify**: `onepackd check --quick` and a `dotnet restore` of a real project.

## Rolling back

There are no backward migrations. To return to the previous version, install the previous binary and restore the backup from step 1 into an empty directory ([restore](backup-restore.md#restore)). An older version refuses to open a newer schema, so there is no risk of corrupting it by mistake.

## How it is tested

`crates/storage/tests/ops.rs` creates a directory with the previous version's schema (all migrations except the last) and data, checks that `migrate --check` reports the pending one and that `migrate` applies it, preserves the data, and leaves the prior backup.
