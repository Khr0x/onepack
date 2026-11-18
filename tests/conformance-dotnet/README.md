# Conformance tests with .NET clients

Test reference per [ADR-019](../../roadmap/adr-mvp.md#adr-019): packages are created with official
tools (`dotnet pack`) and real clients (`dotnet`, `nuget.exe`) are used to validate the server.
.NET is **not** a runtime dependency of `onepackd`.

- `fixtures/`: projects that generate the test `.nupkg` files.

Build the packages locally:

```bash
dotnet pack tests/conformance-dotnet/fixtures/Onepack.Fixture.Basic -c Release -o target/nupkgs
```
