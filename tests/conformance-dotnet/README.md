# Pruebas de conformidad con clientes .NET

Referencia de pruebas según [ADR-019](../../roadmap/adr-mvp.md#adr-019): los paquetes se crean con herramientas
oficiales (`dotnet pack`) y los clientes reales (`dotnet`, `nuget.exe`) se usan para validar el servidor.
.NET **no** es dependencia de `onepackd` en runtime.

- `fixtures/`: proyectos que generan los `.nupkg` de prueba.

Generar los paquetes localmente:

```bash
dotnet pack tests/conformance-dotnet/fixtures/Onepack.Fixture.Basic -c Release -o target/nupkgs
```
