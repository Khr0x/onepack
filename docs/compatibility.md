# Matriz de clientes comprobados

> No se promete "compatibilidad total" con NuGet: solo lo que aparece aquí como comprobado
> ([ADR-009](../roadmap/adr-mvp.md#adr-009), [ADR-019](../roadmap/adr-mvp.md#adr-019)).
>
> Última actualización: 2026-10-04

## Recursos del protocolo

| Recurso (`@type`) | Ruta | Notas |
|---|---|---|
| `PackageBaseAddress/3.0.0` | `/nuget/{feed}/v3/flat/` | Incluye versiones no listadas. |
| `RegistrationsBaseUrl/3.6.0` | `/nuget/{feed}/v3/registration/` | Incluye SemVer 2.0.0 y versiones no listadas (`listed: false`). Páginas de 64 versiones, inline hasta 128. Sin compresión gzip. |
| `SearchQueryService` (y `/3.0.0-beta`, `/3.0.0-rc`, `/3.5.0`) | `/nuget/{feed}/v3/query` | `q`, `skip`, `take`, `prerelease`, `semVerLevel`, `packageType`. Solo versiones listadas. Sintaxis: términos libres, `id:` y `packageid:`. |
| `SearchAutocompleteService` (y `/3.0.0-beta`, `/3.0.0-rc`, `/3.5.0`) | `/nuget/{feed}/v3/autocomplete` | Ids (`q`) y versiones de un id (`id`). |
| `PackagePublish/2.0.0` | `/nuget/{feed}/v2/package` | `PUT` (publicar), `DELETE` (unlist), `POST` (relist). |

No implementados: API V2 (OData), catálogo, `ReadmeUriTemplate`, `PackageDetailsUriTemplate`, servidor de símbolos, contadores de descargas (siempre 0) e iconos embebidos (`iconUrl` solo si el paquete declara una URL).

## Clientes

Leyenda: ✅ comprobado · ⏳ pendiente · ➖ fuera del alcance actual.

CI de referencia: ejecución [37185158143](https://github.com/Khr0x/onepack/actions/runs/37185158143) (2026-10-04).

| Cliente | Versión | Plataforma | Operaciones comprobadas | Cómo | Estado |
|---|---|---|---|---|---|
| `dotnet` CLI | SDK 8.0 (LTS anterior) | Linux | push, 409, search (con y sin prerelease, exacta), restore con rango, delete (unlist) | `scripts/e2e-dotnet.sh` en CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| `dotnet` CLI | SDK 10.0 (LTS actual) | Linux, detrás de nginx con TLS | igual que arriba, por HTTPS con CA propia | `ONEPACK_E2E_TLS=1` en CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| `dotnet` CLI | SDK 8.0 y 10.0 | macOS | igual que arriba | `scripts/e2e-dotnet.sh` en CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| `dotnet` CLI | SDK 8.0 | Windows | restore con credenciales en `NuGet.Config`, run | `tests/conformance-dotnet/e2e-windows.ps1` en CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| `nuget.exe` | última de dist.nuget.org | Windows | push, 409, search (con y sin prerelease), install con rango | `tests/conformance-dotnet/e2e-windows.ps1` en CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| GitHub Actions | — | Linux, macOS, Windows | push y restore con credenciales inyectadas por variables de entorno | los propios jobs de CI | ✅ [CI](https://github.com/Khr0x/onepack/actions/runs/37185158143) |
| Visual Studio | — | Windows | navegación y restore | manual | ⏳ diferido al piloto (Fase 8) |
| JetBrains Rider | — | Windows / macOS | navegación y restore | manual | ⏳ diferido al piloto (Fase 8) |
| Azure Pipelines | — | Linux | restore y push | — | ➖ sin entorno de prueba |

### Verificación manual (IDE)

1. Añade el feed con credenciales (Visual Studio: *Herramientas → Opciones → NuGet → Orígenes*; Rider: *NuGet → Sources*).
2. Busca `Onepack.Fixture` con y sin prerelease y abre el detalle de `Onepack.Fixture.Rich` (título, autores, licencia, etiquetas).
3. Instala `Onepack.Fixture.Ranged` en un proyecto y comprueba que se restaura `Onepack.Fixture.Basic` 1.1.0.
4. Anota aquí la versión del IDE, la fecha y el resultado.

## Particularidades encontradas

| Cliente | Comportamiento | Cómo se resuelve |
|---|---|---|
| `dotnet nuget push` | Hace el `PUT` sobre `{PackagePublish}/` con barra final. | Se aceptan las dos formas. |
| `dotnet nuget push` | Consulta el service index con Basic (tras el `401`) y publica con `X-NuGet-ApiKey`. | Las rutas NuGet aceptan ambos mecanismos (ADR-011). |
| `nuget.exe search` | No acepta `-ConfigFile` (sí `push` e `install`); lee el `NuGet.Config` del directorio actual. | Se ejecuta desde el directorio con la configuración del feed. |
