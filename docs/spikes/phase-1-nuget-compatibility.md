# Spike report — Phase 1: NuGet compatibility

> Date: 2026-10-04 · Roadmap: [Phase 1](../../roadmap/roadmap-mvp.md#phase-1--nuget-compatibility-spike) · Decision: [ADR-002](../../roadmap/adr-mvp.md#adr-002)

## Question

Can a Rust server be accepted by real NuGet clients without reusing the official .NET libraries, at a reasonable cost?

## What was built

| Component | Location |
|---|---|
| NuGet version normalization and comparison | `crates/nuget/src/version.rs` |
| Package id validation and identity | `crates/nuget/src/id.rs` |
| Reading the `.nuspec` from the `.nupkg` (identity and dependency groups) | `crates/nuget/src/package.rs` |
| Axum server: service index, `PackageBaseAddress/3.0.0`, `PackagePublish/2.0.0` | `crates/server/src/` |
| Provisional on-disk storage (replaced in Phase 2) | `crates/server/src/feed_store.rs` |
| Reference corpus generated with `NuGet.Versioning` 7.9.0 | `tests/conformance-dotnet/version-corpus/` |
| E2E with `dotnet pack` / `nuget push` / `restore` / `run` | `scripts/e2e-dotnet.sh` |

## Results

### Version normalization (ADR-008)

- Corpus: **168 entries**, of which 122 are valid versions, and about **15,000 pairs** (122²) compared for order and identity.
- **Divergences: 0.**
- A mutation test confirms that the test detects errors: reversing a comparison rule produces 395 divergences.

NuGet behaviors that a generic SemVer library does not cover and that the corpus pinned down:

| Case | NuGet behavior |
|---|---|
| `1`, `1.0`, `1.0.0.0`, `01.00` | Valid and equivalent to `1.0.0`. |
| `1. 0.0`, `1.0.0 -beta`, `1　.0` | Valid: numeric components allow Unicode whitespace around them. |
| `1.0.0-beta 1`, `1.0.0-beta +meta` | Invalid: labels do not allow whitespace. |
| `1.0.0-01`, `1.0.0-a.01` | Invalid: numeric label with a leading zero. `+01` is valid in metadata. |
| `1.0.0-Alpha` vs `1.0.0-alpha` | Same identity; the normalized form preserves capitalization. |
| `1.0.0-alpha.-1` < `1.0.0-alpha.1` | `-1` counts as a **numeric** label (`Int32.TryParse`). |
| `1.0.0-2147483648` | Alphanumeric label, because it overflows `Int32`. |
| `1.0.0-0` vs `1.0.0--0` | `Compare` returns 0 but `Equals` says they differ: a NuGet inconsistency. The corpus records both relations separately and onepack reproduces both. |

### Real clients (ADR-019)

Run with the .NET 8 SDK (NuGet 6.10) against `onepackd` on loopback:

| Operation | Result |
|---|---|
| `dotnet nuget push` of 2 packages | ✅ `201 Created` |
| Duplicate push of the same identity | ✅ `409`, the client reports it as an error |
| `dotnet restore` with empty `NUGET_PACKAGES` and HTTP cache | ✅ resolves the transitive dependency per framework |
| Restored bytes vs. published bytes | ✅ identical (`cmp`) |
| `dotnet run` of the consumer | ✅ runs code from both packages |

Restore works by advertising **only** `PackageBaseAddress/3.0.0`, without registries or search. This confirms that ADR-009 can advertise resources incrementally.

### Incompatibilities found

1. **Trailing slash in publishing.** The NuGet client sends the `PUT` to the PackagePublish `{@id}` with a trailing slash, even though the service index advertises the URL without one. The server now accepts both forms and has a specific test.

It was the only incompatibility, and the E2E caught it on the first attempt. It is exactly the kind of failure ADR-019 wanted to discover with real clients.

## Cost

- Version normalization: about 190 lines of Rust, excluding tests, which matched the corpus on the first try.
- Package reading, ids, server, and provisional storage: about 680 lines, excluding tests.
- Added dependencies (all with a license allowed by `cargo-deny`): `axum`, `tokio`, `tower-http`, `clap`, `serde_json`, `tracing`, `zip` (deflate only), and `quick-xml`.

## Known limitations and where they are resolved

| Limitation | Phase |
|---|---|
| Upload is read into memory, bounded by `--max-package-size-mib` | 2 (streaming to staging, ADR-006) |
| Directory-based storage and a mutex instead of SQLite | 2 (ADR-004, ADR-005) |
| No authentication: `X-NuGet-ApiKey` is ignored | 4 (ADR-011) |
| The id is lowercased with Rust's `to_lowercase`, not with `ToLowerInvariant`; `\w` is approximate | 3: add ids to the corpus |
| No registries, search, autocomplete, or unlist | 3 |
| Only tested with the .NET 8 SDK; no `nuget.exe`, IDEs, or other SDK versions | 3 (client matrix) |
| Basic inspection limits: only the `.nuspec` size and DTD rejection | 5 (ADR-014) |

## Recommendation

**Ratify ADR-002 (Rust).** The two risks that justified the C# alternative were versioning rules and the behavior of real clients. Versioning rules are covered by a corpus generated with the official library and verified in every CI. Client behavior is covered by an E2E that publishes and restores with `dotnet`. The cost was low and the only incompatibility appeared and was fixed within the spike itself.

**Ratified on 2026-10-04:** gate met in [CI](https://github.com/Khr0x/onepack/actions/runs/37176042099); ADR-002 moves to `Accepted`.
