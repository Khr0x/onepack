# Informe del spike — Fase 1: compatibilidad NuGet

> Fecha: 2026-10-04 · Roadmap: [Fase 1](../../roadmap/roadmap-mvp.md#fase-1--spike-de-compatibilidad-nuget) · Decide: [ADR-002](../../roadmap/adr-mvp.md#adr-002)

## Pregunta

¿Puede un servidor en Rust ser aceptado por clientes NuGet reales sin reutilizar las bibliotecas oficiales de .NET, con un coste razonable?

## Qué se construyó

| Pieza | Ubicación |
|---|---|
| Normalización y comparación de versiones NuGet | `crates/nuget/src/version.rs` |
| Validación e identidad de ids de paquete | `crates/nuget/src/id.rs` |
| Lectura del `.nuspec` desde el `.nupkg` (identidad y grupos de dependencias) | `crates/nuget/src/package.rs` |
| Servidor Axum: service index, `PackageBaseAddress/3.0.0`, `PackagePublish/2.0.0` | `crates/server/src/` |
| Almacenamiento provisional en disco (se reemplaza en la Fase 2) | `crates/server/src/feed_store.rs` |
| Corpus de referencia generado con `NuGet.Versioning` 7.9.0 | `tests/conformance-dotnet/version-corpus/` |
| E2E con `dotnet pack` / `nuget push` / `restore` / `run` | `scripts/e2e-dotnet.sh` |

## Resultados

### Normalización de versiones (ADR-008)

- Corpus: **168 entradas**, de las que 122 son versiones válidas, y unos **15 000 pares** (122²) comparados en orden y en identidad.
- **Divergencias: 0.**
- Un test de mutación confirma que el test detecta errores: al invertir una regla de comparación aparecen 395 divergencias.

Comportamientos de NuGet que una librería SemVer genérica no cubre y que el corpus fijó:

| Caso | Comportamiento de NuGet |
|---|---|
| `1`, `1.0`, `1.0.0.0`, `01.00` | Válidas y equivalentes a `1.0.0`. |
| `1. 0.0`, `1.0.0 -beta`, `1　.0` | Válidas: los componentes numéricos admiten espacios Unicode alrededor. |
| `1.0.0-beta 1`, `1.0.0-beta +meta` | Inválidas: las etiquetas no admiten espacios. |
| `1.0.0-01`, `1.0.0-a.01` | Inválidas: etiqueta numérica con cero a la izquierda. `+01` sí es válido en metadatos. |
| `1.0.0-Alpha` vs `1.0.0-alpha` | Misma identidad; la forma normalizada conserva la capitalización. |
| `1.0.0-alpha.-1` < `1.0.0-alpha.1` | `-1` cuenta como etiqueta **numérica** (`Int32.TryParse`). |
| `1.0.0-2147483648` | Etiqueta alfanumérica, porque desborda `Int32`. |
| `1.0.0-0` vs `1.0.0--0` | `Compare` devuelve 0 pero `Equals` dice que son distintas: una inconsistencia de NuGet. El corpus registra las dos relaciones por separado y onepack reproduce ambas. |

### Clientes reales (ADR-019)

Ejecutado con el SDK de .NET 8 (NuGet 6.10) contra `onepackd` en loopback:

| Operación | Resultado |
|---|---|
| `dotnet nuget push` de 2 paquetes | ✅ `201 Created` |
| Push duplicado de la misma identidad | ✅ `409`, el cliente lo reporta como error |
| `dotnet restore` con `NUGET_PACKAGES` y caché HTTP vacíos | ✅ resuelve la dependencia transitiva por framework |
| Bytes restaurados frente a los publicados | ✅ idénticos (`cmp`) |
| `dotnet run` del consumidor | ✅ ejecuta código de ambos paquetes |

El restore funciona anunciando **solo** `PackageBaseAddress/3.0.0`, sin registros ni búsqueda. Eso confirma que ADR-009 puede anunciar recursos de forma incremental.

### Incompatibilidades encontradas

1. **Barra final en la publicación.** El cliente NuGet hace el `PUT` sobre `{@id de PackagePublish}/`, con barra final, aunque el service index anuncie la URL sin ella. El servidor ahora acepta las dos formas y tiene un test específico.

Fue la única incompatibilidad, y el E2E la detectó en el primer intento. Es justo el tipo de fallo que ADR-019 quería descubrir con clientes reales.

## Coste

- Normalización de versiones: unas 190 líneas de Rust, sin contar tests, que coincidieron con el corpus a la primera.
- Lectura de paquetes, ids, servidor y almacenamiento provisional: unas 680 líneas, sin contar tests.
- Dependencias añadidas (todas con licencia permitida por `cargo-deny`): `axum`, `tokio`, `tower-http`, `clap`, `serde_json`, `tracing`, `zip` (solo deflate) y `quick-xml`.

## Limitaciones conocidas y dónde se resuelven

| Limitación | Fase |
|---|---|
| La subida se lee en memoria, acotada por `--max-package-size-mib` | 2 (streaming a staging, ADR-006) |
| Almacenamiento por directorios y un mutex en lugar de SQLite | 2 (ADR-004, ADR-005) |
| Sin autenticación: se ignora `X-NuGet-ApiKey` | 4 (ADR-011) |
| El id se pasa a minúsculas con `to_lowercase` de Rust, no con `ToLowerInvariant`; `\w` es aproximado | 3: añadir ids al corpus |
| Sin registros, búsqueda, autocompletado ni unlist | 3 |
| Solo probado con SDK .NET 8; sin `nuget.exe`, IDEs ni otras versiones del SDK | 3 (matriz de clientes) |
| Límites de inspección básicos: solo el tamaño del `.nuspec` y el rechazo de DTD | 5 (ADR-014) |

## Recomendación

**Ratificar ADR-002 (Rust).** Los dos riesgos que justificaban la alternativa en C# eran las reglas de versionado y el comportamiento de los clientes reales. Las reglas de versionado quedan cubiertas por un corpus generado con la biblioteca oficial y verificado en cada CI. El comportamiento de los clientes queda cubierto por un E2E que publica y restaura con `dotnet`. El coste fue bajo y la única incompatibilidad apareció y se corrigió dentro del propio spike.

**Ratificado el 2026-10-04:** gate cumplido en [CI](https://github.com/Khr0x/onepack/actions/runs/37176042099); ADR-002 pasa a `Aceptado`.
