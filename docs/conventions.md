# Project conventions

## Binaries and crates

| Binary | Crate | Role |
|---|---|---|
| `onepackd` | `onepack-server` | Registry server. |
| `onepack` | `onepack-cli` | Administrative CLI. |

Libraries: `onepack-core`, `onepack-nuget`, `onepack-storage`, `onepack-api-client`. The allowed dependencies between them are in [ADR-003](../roadmap/adr-mvp.md#adr-003) and are checked by `scripts/check-crate-deps.sh` in CI.

## Toolchain

- Version pinned in `rust-toolchain.toml`. That value is also the MSRV (the workspace `rust-version`).
- To bump the version, change both values in the same PR.
- `unsafe` is forbidden across the workspace (`unsafe_code = "forbid"`).

The CLI uses `keyring-core` with native credential stores: Keychain on macOS,
Credential Manager on Windows, and `dbus-secret-service-keyring-store` on Unix.
The Unix backend enables `crypto-rust` and `vendored` to preserve static musl
binaries. Building it requires a C toolchain, make, Perl and pkg-config (in
addition to musl tools when cross-compiling); users of the binaries need a
running Secret Service for `login`, without installing libdbus or OpenSSL.
For ARM64 musl, `.cargo/config.toml` keeps C atomics inline so libdbus does not
require the outlined atomic helpers from libgcc at link time.
Credential attributes remain `service=onepack` and `username=<context>` in the
default collection. To check compatibility with an earlier CLI, run
`ONEPACK_PREVIOUS_BIN_DIR=/path/to/old/bin ./scripts/e2e-cli-linux.sh` on Linux.

## Language

- Everything users see is in English: CLI and server messages, `--help` texts, logs and metrics descriptions.
- Documentation, commits and pull requests are written in English. A document still in Spanish is translated in full the next time it is edited, rather than mixing languages.

## Errors

Every error that reaches a user (API `/api/v1`, CLI or operations log) has:

| Field | Example | Rule |
|---|---|---|
| `code` | `AUTH_SCOPE_MISSING` | `SCREAMING_SNAKE_CASE`, prefixed by area, **stable**: never renamed once published. |
| `message` | `the credential is valid, but lacks the packages:publish permission on this feed` | Readable and free of secrets. |
| `action` | `ask an administrator for a sufficient role: …` | Optional. Says what to do. |
| `request_id` | `3f9c0a1b2c3d4e5f` | Present in every HTTP response. |

JSON format in `/api/v1`:

```json
{
  "error": {
    "code": "AUTH_SCOPE_MISSING",
    "message": "the credential is valid, but lacks the packages:publish permission on this feed",
    "action": "ask an administrator for a sufficient role: `onepack grant add --principal <principal> --feed <feed> --role <role>`",
    "request_id": "3f9c0a1b2c3d4e5f"
  }
}
```

The NuGet protocol endpoints respond with the HTTP status the protocol requires. They also include `X-Request-Id`.

### Code prefixes

| Prefix | Area |
|---|---|
| `AUTH_` | Authentication and authorization. |
| `FEED_` | Feeds. |
| `PKG_` | Packages and versions (validation, duplicates, states). |
| `LIMIT_` | Quotas, size limits and rate limits. |
| `STORAGE_` | Database and blobs. |
| `MAINT_` | Maintenance mode, backup, migrations. |
| `CLI_` | Local CLI errors (configuration, keychain, input). |

Secrets never appear in messages, logs or diagnostic output.

## CLI exit codes

The table is fixed since Phase 6 closed; changing it requires a major CLI version ([ADR-015](../roadmap/adr-mvp.md#adr-015)). The full table, with codes 0 to 10, is in [docs/cli.md](cli.md#exit-codes).

## Commits and branches

- Commits follow [Conventional Commits](https://www.conventionalcommits.org/): `feat(nuget): …`, `fix(storage): …`, `docs(adr): …`, `ci: …`, `chore: …`.
- Scope = the crate's short name (`core`, `nuget`, `storage`, `api-client`, `server`, `cli`) or an area (`adr`, `roadmap`, `ci`).
- Branches: `feature/phase-N-<short-description>` for roadmap work and `fix/<description>` for fixes.
- Every PR goes through green CI before it is merged into `main`.

## Decisions

- Architecture decisions go in [roadmap/adr-mvp.md](../roadmap/adr-mvp.md). Their life cycle is described in that document.
- A PR that contradicts an `Accepted` ADR must include the ADR that replaces it.
