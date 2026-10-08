# Verified client matrix

> We do not promise "full compatibility" with NuGet: only what is listed here as verified
> ([ADR-009](../roadmap/adr-mvp.md#adr-009), [ADR-019](../roadmap/adr-mvp.md#adr-019)).
>
> Last updated: 2026-10-07

## Protocol resources

| Resource (`@type`) | Route | Notes |
|---|---|---|
| `PackageBaseAddress/3.0.0` | `/nuget/{feed}/v3/flat/` | Includes unlisted and blocked versions; downloading a blocked one responds `410`. |
| `RegistrationsBaseUrl/3.6.0` | `/nuget/{feed}/v3/registration/` | Includes SemVer 2.0.0, unlisted versions (`listed: false`) and blocked ones (with `deprecation`). Pages of 64 versions, inline up to 128. No gzip compression. |
| `SearchQueryService` (and `/3.0.0-beta`, `/3.0.0-rc`, `/3.5.0`) | `/nuget/{feed}/v3/query` | `q`, `skip`, `take`, `prerelease`, `semVerLevel`, `packageType`. Listed versions only. Syntax: free terms, `id:` and `packageid:`. |
| `SearchAutocompleteService` (and `/3.0.0-beta`, `/3.0.0-rc`, `/3.5.0`) | `/nuget/{feed}/v3/autocomplete` | Ids (`q`) and the versions of an id (`id`). |
| `PackagePublish/2.0.0` | `/nuget/{feed}/v2/package` | `PUT` (publish), `DELETE` (unlist), `POST` (relist). Errors carry a stable code at the start of the text (`CODE: message`). |

Not implemented: V2 API (OData), catalog, `ReadmeUriTemplate`, `PackageDetailsUriTemplate`, symbol server, download counts (always 0) and embedded icons (`iconUrl` only if the package declares a URL).

## Clients

Legend: ✅ verified · ⏳ pending · ➖ out of current scope.

Reference CI: run [37185158143](https://github.com/Khr0x/onepack/actions/runs/37185158143) (2026-10-04).

| Client | Version | Platform | Verified operations | How | Status |
|---|---|---|---|---|---|
| `dotnet` CLI | SDK 8.0 (previous LTS) | Linux | push, 409, search (with and without prerelease, exact), restore with a range, delete (unlist) | `scripts/e2e-dotnet.sh` in CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| `dotnet` CLI | SDK 10.0 (current LTS) | Linux, behind nginx with TLS | same as above, over HTTPS with a private CA | `ONEPACK_E2E_TLS=1` in CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| `dotnet` CLI | SDK 8.0 and 10.0 | macOS | same as above | `scripts/e2e-dotnet.sh` in CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| `dotnet` CLI | SDK 8.0 | Windows | restore with credentials in `NuGet.Config`, run | `tests/conformance-dotnet/e2e-windows.ps1` in CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| `nuget.exe` | latest from dist.nuget.org | Windows | push, 409, search (with and without prerelease), install with a range | `tests/conformance-dotnet/e2e-windows.ps1` in CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| GitHub Actions | — | Linux, macOS, Windows | push and restore with credentials injected through environment variables | the CI jobs themselves | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| Visual Studio | — | Windows | browsing and restore | manual | ⏳ deferred to the pilot (Phase 8) |
| JetBrains Rider | — | Windows / macOS | browsing and restore | manual | ⏳ deferred to the pilot (Phase 8) |
| Azure Pipelines | — | Linux | restore and push | — | ➖ no test environment |

### Manual verification (IDE)

1. Add the feed with credentials (Visual Studio: *Tools → Options → NuGet Package Manager → Package Sources*; Rider: *NuGet → Sources*).
2. Search for `Onepack.Fixture` with and without prerelease and open the details of `Onepack.Fixture.Rich` (title, authors, license, tags).
3. Install `Onepack.Fixture.Ranged` in a project and check that `Onepack.Fixture.Basic` 1.1.0 is restored.
4. Record the IDE version, the date and the result here.

## Client quirks found

| Client | Behavior | How it is handled |
|---|---|---|
| `dotnet nuget push` | Sends the `PUT` to `{PackagePublish}/` with a trailing slash. | Both forms are accepted. |
| `dotnet nuget push` | Queries the service index with Basic (after the `401`) and publishes with `X-NuGet-ApiKey`. | The NuGet routes accept both mechanisms (ADR-011). |
| `dotnet restore` | On a download error it shows the HTTP status and reason phrase, not the body. | Blocked versions respond `410` with the reason phrase `PACKAGE_BLOCKED - version blocked by the registry` ([docs/security.md](security.md)). |
| `dotnet restore` | With a revoked or expired token it only shows `NU1301: Unable to load the service index`, without mentioning the credential. | `onepack exec` checks the credential before running the command and explains the failure ([cli](cli.md#nugetconfig-and-nuget-credentials)); the server log records the reason (`reason="revoked"`). |
| `nuget.exe search` | Does not accept `-ConfigFile` (`push` and `install` do); it reads the `NuGet.Config` in the current directory. | It runs from the directory with the feed configuration. |
