# `onepack` CLI

> Phase 6 ([ADR-015](../roadmap/adr-mvp.md#adr-015), [ADR-016](../roadmap/adr-mvp.md#adr-016)).
> Last updated: 2026-10-07

`onepack` runs the registry from the terminal, for people and pipelines alike. It is a separate binary from the server (`onepackd`) and does not need the server installed.

```bash
cargo build --release -p onepack-cli   # target/release/onepack
```

## Contexts

A context is a named server. The configuration (`config.json`) never contains secrets.

```bash
onepack context add prod --url https://packages.example.com --use
onepack context add lab --url https://lab.example.test --ca-cert ./ca.pem
onepack context list
onepack context use lab
onepack context remove lab --yes
```

Location: `$ONEPACK_CONFIG_DIR`, else `$XDG_CONFIG_HOME/onepack`, else `~/.config/onepack` (`%APPDATA%\onepack` on Windows). `--context` or `ONEPACK_CONTEXT` picks another context for one command; `--url` or `ONEPACK_URL` points at a server without a context.

## Credentials

The token is resolved in this order:

1. `--token-env VAR`: the token is in the variable `VAR`. This is the option for pipelines.
2. `ONEPACK_TOKEN`.
3. The system keychain (macOS Keychain, Windows Credential Manager, Secret Service on Linux), stored with `onepack login`.

```bash
onepack login                  # prompts for the token without echo
printf %s "$TOKEN" | onepack login --token-stdin
onepack logout
onepack whoami
```

`login` validates the token against the server before storing it. **There is no plain-text fallback**: without a keychain, `login` fails with `KEYCHAIN_UNAVAILABLE` (exit 9) and suggests `--token-env`. `ONEPACK_KEYRING=off` turns the keychain off on purpose (containers, CI).

## Commands

| Command | What it does | Requires |
|---|---|---|
| `whoami` | Principal, id and expiry of the token in use, and per-feed permissions. | authenticated |
| `feed list`, `feed show NAME` | Feeds visible to the credential, with usage and quotas. | read |
| `feed create NAME` | Creates a feed. | admin |
| `feed configure NAME [--max-storage-mib N] [--max-versions N]` | Quotas; anything not given is kept, `0` means no limit. | admin |
| `principal create NAME [--kind user\|service] [--admin]` | Creates a user (default) or a service account; `--admin` grants the global administrator role. | admin |
| `principal list` | Principals, kind, whether they are administrators and whether they are active. | admin |
| `principal disable NAME [--yes]` | Cuts off all of its tokens immediately. It is permanent: there is no `enable`. The last administrator cannot be disabled. | admin |
| `token create --principal P [--name N] [--expires-in-days D]` | Issues a token. Expires after 90 days by default (1 to 3650). In text mode only the token goes to stdout; it is never shown again. | admin |
| `token list [--principal P]` | Active tokens, without secrets. | admin |
| `token revoke ID [--yes]` | `ID` is the 16 characters after `opk_`. | admin |
| `grant add --principal P --feed F --role R [--publish-pattern X]…` | Assigns or replaces the role (see [Roles](#roles)). `--publish-pattern` is repeatable. | admin |
| `grant remove --principal P --feed F [--yes]` | Removes the principal's access to the feed. | admin |
| `grant list [--principal P] [--feed F]` | | admin |
| `package list --feed F` | Packages with their number of versions and the highest one. | read |
| `package inspect --feed F ID [VERSION]` | Without a version, the versions by precedence; with one, its state, SHA-256 and size. Only a Maintainer or an admin sees why a version is blocked. | read |
| `package push --feed F FILE… [--skip-existing-identical]` | Publishes through the same route as `dotnet nuget push`. Validates the package locally before uploading. With the option, an existing version with the **same** content (SHA-256) is not an error; with different content it is. | publish |
| `package unlist\|relist --feed F ID VERSION` | Visibility in search; does not affect downloads. | publish |
| `package block --feed F ID VERSION --reason R [--yes]` | Prevents downloads ([security](security.md#bloqueo-de-versiones)). | Maintainer |
| `package unblock --feed F ID VERSION --reason R` | Allows downloads again. The reason is also required and audited. | Maintainer |
| `audit list [--feed F] [--action PREFIX] [--limit N]` | Newest first; 50 events by default. `--action` filters by prefix (`package.`, `token.create`). | admin |
| `nuget init --feed F… --pattern P… [--config PATH] [--dry-run] [--yes]` | See [NuGet.Config](#nugetconfig-and-nuget-credentials). `--feed` and `--pattern` are repeatable. | — |
| `exec --feed F… [--source-name N] -- CMD…` | See [NuGet.Config](#nugetconfig-and-nuget-credentials). `--source-name` only with a single feed. | read |
| `doctor [--feed F] [--require read\|publish\|maintain] [--config PATH]` | See [Diagnostics](#diagnostics). `--require` defaults to `read`. | — |
| `completion bash\|zsh\|fish\|powershell\|elvish` | Shell completion script. | — |

Destructive operations (`token revoke`, `principal disable`, `grant remove`, `package block`, `context remove`) ask for confirmation. Without a terminal, or with `--no-input` (`ONEPACK_NO_INPUT=1`), they require `--yes`.

## Roles

Permissions are assigned per feed with `grant add` and are cumulative: each role includes the previous one.

| Role | Can | Scope (`doctor --require`) |
|---|---|---|
| `reader` | Search, read metadata, download and restore; `package list` and `inspect`. | `packages:read` (`read`) |
| `publisher` | All of the above, plus publish new versions and `unlist`/`relist`. | `packages:publish` (`publish`) |
| `maintainer` | All of the above, plus block and unblock versions and see why a version is blocked. | `packages:maintain` (`maintain`) |

- `--publish-pattern` limits which ids the principal can publish (and unlist or relist) in that feed: an exact id or a prefix ending in `*` (`Hemia.Payments.*`), case-insensitive. Without patterns it can publish any id in the feed. It does not affect reading.
- **Administrator** (`principal create --admin`) is not a per-feed role: it is global. It manages feeds, quotas, principals, tokens, grants and the audit log, and has `maintainer` access to every feed.
- A feed you cannot read responds exactly like a feed that does not exist (exit 5).

## Common tasks

**Give a CI pipeline access** (full walkthrough in the [guide](guide-zero-to-ci.md)):

```bash
onepack principal create ci-payments --kind service
onepack grant add --principal ci-payments --feed payments --role publisher --publish-pattern 'Hemia.Payments.*'
onepack token create --principal ci-payments --name github-actions --expires-in-days 180
```

**Rotate a token** without downtime: issue the new one, swap it into the pipeline secret, then revoke the old one.

```bash
onepack token list --principal ci-payments          # note the old token's ID
onepack token create --principal ci-payments --name github-actions-2026-10
# … update the pipeline secret and check a build …
onepack token revoke <old-ID> --yes
```

**Offboard a person**: `principal disable` cuts off all of their tokens at once and is permanent. If they should only lose access to one feed, use `grant remove`.

```bash
onepack principal disable ana --yes
onepack audit list --action principal. --limit 20   # check that it was recorded
```

**Leaked credential**: revoke that token (`token revoke`) and issue a new one. The full procedure, including caches and the audit log, is in the [incident drill](runbooks/incident-drill.md).

**Vulnerable version**: block it so it cannot be downloaded. Blocking does not delete copies already downloaded into client caches and CI agents ([security](security.md#bloqueo-de-versiones)).

```bash
onepack package block --feed payments Hemia.Payments 2.3.1 --reason "CVE-2026-1234" --yes
onepack package unblock --feed payments Hemia.Payments 2.3.1 --reason "false positive"
```

**Find out why a restore or a push fails**:

```bash
onepack doctor --feed payments --require publish
```

## NuGet.Config and NuGet credentials

Storing a token in the CLI does not make `dotnet restore` use it ([ADR-016](../roadmap/adr-mvp.md#adr-016)). There are two pieces:

**`onepack nuget init`** adds each feed to `NuGet.Config` with the key `onepack_<feed>` (`-` becomes `_`) and a `packageSourceMapping` with the given patterns:

```bash
onepack nuget init --feed internal --pattern 'Hemia.*' --dry-run   # shows the diff
onepack nuget init --feed internal --pattern 'Hemia.*' --yes
```

- Keeps everything already there (comments, other sources, other sections); whatever does not change is rewritten byte for byte.
- **Never writes credentials.** It warns if it finds a `ClearTextPassword`.
- If the file had no `packageSourceMapping`, it maps its other sources to `*` so that what already restored keeps working (onepack's patterns are more specific and win). If the file does not start with `<clear />`, it also maps `nuget.org`, which is usually inherited from the user configuration. Any other inherited source must be mapped by hand.
- With an `http://` URL it adds `allowInsecureConnections="true"` and warns: use it only locally.
- `packageSourceMapping` reduces the risk of *dependency confusion*, but does not guarantee that internal ids are never looked up in other sources. Provenance tests must use clean caches.

**`onepack exec`** runs a command with `NuGetPackageSourceCredentials_<key>` set **only in the child process environment**. The token is not written to disk and does not stay in the shell. The exit code is the command's.

Before running it, `exec` checks the credential (`GET /api/v1/whoami`): without that check, NuGet would only say `NU1301: Unable to load the service index`. If the token is invalid (revoked, expired, or belonging to a disabled principal), it fails with exit 3; if it lacks access to one of the feeds, with exit 4 and `AUTH_SCOPE_MISSING`. In both cases the command does not run. If the server does not respond, it warns and runs the command anyway: the local cache may be enough.

```bash
onepack exec --feed internal -- dotnet restore
onepack exec --feed internal --feed customer-a -- dotnet build
```

CI does not need `exec`: set the variable directly from the pipeline secret (see the [guide](guide-zero-to-ci.md)).

## Diagnostics

`onepack doctor --feed F` checks, in order: the URL, DNS, the TCP connection, TLS (or warns about HTTP without TLS outside loopback), the credential (showing only the token id and where it came from), authentication and expiry, the server and its capabilities, permissions on the feed (`--require`), the service index (its URLs must start with the context's URL, i.e. the server's `--public-url` must match), and the `NuGet.Config` in the current directory or its parents: source, plain-text credentials and source mapping.

Each check reports `ok`, `warn`, `fail` or `skip`. A failure includes a code and the action to take. It exits with 10 if any check fails.

## Output contract

- Data goes to **stdout**; diagnostics (warnings, progress, errors) go to **stderr**.
- `--json` writes one JSON document per command. Its shape (keys and types) is pinned by a snapshot in `crates/cli/tests/snapshots/json-schema.json`. An incompatible change requires a major CLI version. Lists are arrays with every page.
- With `--json`, errors go to stderr as `{"error": {"code", "message", "action", "request_id"}}`. Without `--json`:

  ```text
  error: AUTH_SCOPE_MISSING: the credential is valid, but lacks the packages:publish permission on this feed
    action: ask an administrator for a sufficient role: `onepack grant add --principal <principal> --feed <feed> --role <role>`
    request_id: 3f9c0a1b2c3d4e5f
  ```

  The `request_id` also appears in the server log. Messages never include the token.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Unclassified error (local I/O, cancelled operation, …). |
| 2 | Usage error: arguments, invalid input, unknown context, or confirmation required with `--no-input`. |
| 3 | No valid credential: missing, expired or revoked (`401`). |
| 4 | Valid credential without permission (`403`). |
| 5 | Not found (`404`). |
| 6 | Conflict: already exists (`409`), e.g. a version published with different content. |
| 7 | Unavailable and retryable: network, DNS, TLS, timeout, `429` or `5xx`. |
| 8 | Incompatible server: it does not offer a capability the command needs. |
| 9 | No system keychain. |
| 10 | `doctor` found failures. |

`exec` returns the exit code of the command it ran (128 + signal if it was killed by a signal).

## Network and compatibility

- `--timeout SECS` (`ONEPACK_TIMEOUT`): 30 s per request; 600 s for `package push`.
- Only idempotent requests (`GET`, `PUT`, `DELETE`) are retried, up to 3 attempts, on connection errors, timeouts and `429`/`502`/`503`/`504`, honouring `Retry-After`. `package push` and `POST` operations are never retried.
- TLS uses the system trust roots; `--ca-cert` (`ONEPACK_CA_CERT`) adds a custom CA in PEM.
- Before each administrative command the CLI calls `GET /api/v1/capabilities`. If the capability it needs is missing, it fails with `CAPABILITY_MISSING` (exit 8) and reports the server version. A server without that endpoint (older than Phase 6) is treated as having no administrative capabilities. The API is [v1](api.md).
