# Incident drill

> Phase 8. Three incidents, in order and timed, on the pilot server. The automated version of this drill is [`scripts/e2e-acceptance.sh`](../../scripts/e2e-acceptance.sh), which runs on every change.
> Last updated: 2026-10-04

Rules: only `onepack`, `onepackd` and the documentation. No web UI, no opening the database and no help from whoever wrote the code. Record the times and any deviation in [the log](../pilot/friction-log.md#incident-drill).

Before starting:

- [ ] There is a CI pipeline that restores from the feed with the service account (for example, `ci`).
- [ ] There is a recent backup ([backup](backup-restore.md)) **copied off the server**.
- [ ] You have an administrator credential in the CLI (`onepack whoami` says `administrador`).

In the examples, the feed is `internal`, the CI account is `ci` and the library is `Hemia.Core`.

## 1. The CI credential has leaked

Goal: the leaked token stops working and the pipeline works again with a new one.

```bash
onepack token list --principal ci                          # find the token (ID column)
onepack token revoke <ID>                                  # asks for confirmation
onepack token create --principal ci --name pipeline --expires-in-days 90
```

1. Store the new token in the pipeline secret ([guide](../guide-zero-to-ci.md)) **before** re-running it.
2. Check that the old token no longer works: `onepack --token-env OLD whoami` exits with code 3. With `onepack exec`, the error says the credential is not valid; with `dotnet restore` directly, NuGet only shows `NU1301` and the server logs `autenticación rechazada reason="revoked"`.
3. Re-run the pipeline: it must restore.
4. Review what the token did while it was leaked: `onepack audit list --limit 100` (actor `ci`) and the *last used* column (`ÚLTIMO USO`) of `onepack token list`.

**Done when:** the pipeline restores with the new token and `token list` shows the old one as revoked.

## 2. A published version is vulnerable

Goal: nobody can download the vulnerable version, and whoever had it sees why.

```bash
onepack package block --feed internal Hemia.Core 1.4.0 --reason "CVE-2026-0001"
```

1. On a machine with no cache (empty `NUGET_PACKAGES`), a restore that needs it fails with `410 (PACKAGE_BLOCKED - version blocked by the registry)` ([security](../security.md#bloqueo-de-versiones)). The version is still listed as deprecated in the metadata.
2. Publish the fixed version (for example, `1.4.1`) from CI and update the project reference.
3. When blocking is no longer needed, or if it was a mistake: `onepack package unblock --feed internal Hemia.Core 1.4.0 --reason "..."`.

Local NuGet caches that already had the version are **not** invalidated: blocking prevents new downloads. Tell the team to clear `~/.nuget/packages/hemia.core/1.4.0` if needed.

**Done when:** a clean restore cannot get the blocked version and `onepack audit list --action package.block` shows the block and who did it.

## 3. The server has been lost

Goal: the service comes back on another machine (or the same one, with an empty disk) from the latest backup, with the same public URL.

Follow [backup and restore](backup-restore.md#restaurar). In short, on the new machine:

```bash
onepackd restore --from /path/to/backup --data-dir /var/lib/onepack
onepackd check --data-dir /var/lib/onepack
sudo systemctl start onepackd        # or the container, with the same volume
curl -fs https://packages.example.com/readyz
```

1. Point DNS or the reverse proxy to the new machine if it changed.
2. Re-run the CI pipeline **without changing its token**: credentials travel in the backup.
3. Anything published after the backup is lost: publish it again from CI (`onepack package push --skip-existing-identical` avoids conflicts with what was kept).
4. Security changes made after the backup are lost too: **repeat the revocations and blocks** you made after the backup (check your incident notes; the restored audit log does not include them).

**Done when:** `/readyz` answers `ready`, the pipeline restores and `onepackd check` reports no errors.

## After the drill

- Record in the log how long each incident took and every step where the documentation was not enough.
- If an administrator token was created for the drill, revoke it.
