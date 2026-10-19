# Architecture Decision Records — v2 multiecosistema

> Diseño de contratos para servir NuGet/.NET, npm/JavaScript y PyPI/Python desde el mismo
> registro privado. El alcance acordado en Obsidian (`Onepack-Alcance-v2`) se materializa
> aquí en decisiones técnicas; este documento es la fuente del repositorio.
>
> Tarea: OP-021. Depende de OP-020 (alcance verificado). Habilita OP-022 y las specs OP-023
> (npm) y OP-027 (PyPI). No implementa adaptadores.
>
> Baseline consultado: `88467a94eaee4fc7345bac54fdf6fa4449a913b8`.
> Fecha: 2026-10-19. Estado: **aceptado**. Revisión del 2026-10-19 por el mantenedor
> (instrucción en la sesión de trabajo); ADR-021…026 aceptados sin cambios de alcance.
> La implementación sigue correspondiendo a OP-022, OP-024 y OP-033.

## Índice

| ADR | Decisión | Estatus |
|---|---|---|
| [021](#adr-021) | Feeds tipados por ecosistema; `format` inmutable | Aceptado |
| [022](#adr-022) | Identidad, versión y orden delegados al adaptador | Aceptado |
| [023](#adr-023) | Múltiples archivos por versión con `version_file` | Aceptado |
| [024](#adr-024) | Rutas, capacidades y contratos aditivos por protocolo | Aceptado |
| [025](#adr-025) | `feed_format` como tabla autoritativa; el CHECK v1 queda como legado | Aceptado |
| [026](#adr-026) | Frontera de crates: adaptadores fuera de `core`/`storage` | Aceptado |

---

## 1. Alcance de esta especificación

Esta spec resuelve el diseño de los tres ecosistemas y deja listo el corte de OP-022. No
incluye:

- La matriz de clientes ni los flujos exactos de npm (OP-023) y PyPI (OP-027).
- La implementación de adaptadores, rutas o migraciones (OP-022, OP-024, OP-033).
- SSO, UI, PostgreSQL y alta disponibilidad (propuestas transversales).

La separación modular existente (OP-037/OP-039) se conserva. Este documento solo describe
los cambios reales que el soporte multiecosistema exige.

## 2. Inventario de consumidores y supuestos NuGet

Recorrido del baseline para separar base reutilizable de cambios necesarios. Los símbolos se
citan por archivo; no se fijan números de línea.

| Capa / archivo | Supuesto NuGet actual | Reutilizable | Cambio necesario |
|---|---|---|---|
| `migrations/0001_initial.sql` | `feed.format TEXT NOT NULL CHECK (format IN ('nuget'))`; `package_version.is_semver2`; un solo `blob_sha256` por versión | Identidad inmutable, `blob` por SHA-256, auditoría | Nuevo `feed_format` y `version_file`; documentar `is_semver2` como señal NuGet |
| `crates/core/src/lib.rs` | `Feed` sin formato; `NewVersion`/`PublishedVersion` con `is_prerelease` e `is_semver2`; `package_key`/`version_key` genéricos | Identidad `package_key + version_key` (ADR-007), cuotas, `PublishError` | Añadir `format` a `Feed`; documentar semántica de flags; sin tipos NuGet |
| `crates/core/src/auth.rs` | `PublishPattern` acepta `[A-Za-z0-9._-]` y `*` final; rechaza `@` y `/` | Roles acumulativos y patrón exacto/prefijo | Validación por adaptador; permitir `@` y `/` en patrones npm |
| `crates/storage/src/lib.rs` | `create_feed` inserta literal `'nuget'`; la cuota suma el blob de cada versión | SQL de publicación, staging, cuotas, auditoría | Insertar `feed_format`; cuota suma archivos de la versión |
| `crates/storage/src/catalog.rs` | Filtros `is_prerelease` e `is_semver2` en búsqueda | Paginación y búsqueda por `search_text` | Filtro `is_semver2` solo para NuGet; adaptador decide |
| `crates/server/src/lib.rs` | Rutas `/nuget/{feed}`; `backfill_metadata` lee `.nupkg` con `onepack_nuget` | Capas, auth, límites, métricas, mantenimiento | Rutas por protocolo; backfill despachado por adaptador |
| `crates/server/src/nuget_api.rs` | Construye `NewVersion` desde `.nuspec`; JSON de metadatos con `v3::catalog_metadata` | Flujo staging → inspección → `store.publish` | Extraer el flujo común; el adaptador produce `NewVersion` y archivos |
| `crates/server/src/admin_api.rs` | `Feed` DTO sin `format`; `capabilities` sin ecosistemas | Endpoints administrativos genéricos | Añadir `format`; capacidades `formats.*` |
| `crates/server/src/ops.rs` | `Surface::NuGet` como única superficie de paquete | Métricas y sondas | Superficies `npm` y `pypi` |
| `crates/api-client/src/lib.rs` | DTOs y capacidades sin formato | Contrato aditivo y tolerante (`serde(default)`) | `format` opcional; capacidades nuevas |
| `crates/cli/src/nuget_config.rs`, `nuget.rs` | `nuget init`, `onepack exec`, generación de `NuGet.Config` | `onepack exec`, credenciales, doctor | Comandos por ecosistema en OP-025/OP-034 |
| `docs/api.md`, `docs/compatibility.md`, `docs/security.md` | Documentan solo NuGet | Estructura y criterios | Secciones por protocolo al implementar |
| `crates/nuget/*`, `tests/conformance-dotnet/*` | Adaptador y corpus NuGet | Modelo de adaptador y corpus | Réplica para npm y PyPI |

**Conclusión del inventario:** `core` y `storage` no contienen tipos NuGet hoy. Las fugas
reales son: el `CHECK` de `feed.format`, los flags `is_semver2` (semántica NuGet), el literal
`'nuget'` en `create_feed`, las rutas del servidor, el backfill de metadatos, la superficie
de métricas y los patrones de publicación. Ninguna obliga a introducir `onepack-nuget` en
`core` o `storage`.

## 3. Decisiones

<a id="adr-021"></a>

### ADR-021 — Feeds tipados por ecosistema; `format` inmutable

**Contexto.** Un feed hoy es una fila con nombre único y `format` limitado a `nuget`. Los
tres protocolos tienen reglas de nombre, versión, orden y distribución incompatibles; mezclar
paquetes de distinto ecosistema en la misma identidad produce colisiones y permisos ambiguos.

**Decisión.** Cada feed declara exactamente un formato (`nuget`, `npm`, `pypi`) al crearse y
no puede cambiarlo. El nombre del feed sigue siendo único en todo el registro. Para usar el
mismo namespace lógico en dos ecosistemas se crean feeds distintos (`internal`,
`internal-npm`). El formato ausente en la API se interpreta como `nuget`, lo que mantiene el
comportamiento v1.

**Alternativas consideradas.**

- *Feed mixto:* una fila alberga varios formatos. Se descarta: exige identidad compuesta,
  búsqueda y permisos por protocolo dentro del mismo feed, y abre colisiones/confusiones sin
  un caso de uso confirmado.
- *Nombre único por formato* (`(name, format)`): permitiría `internal` en dos ecosistemas,
  pero cambia la clave de todas las rutas y grants. Se pospone hasta que exista necesidad
  demostrada.

**Consecuencias.** Rutas y permisos siguen resolviéndose por feed; no hay ambigüedad de
identidad entre protocolos. El costo es que el mismo nombre lógico requiere dos feeds y dos
grants si se usan dos ecosistemas. Se revisará si aparece esa necesidad real.

<a id="adr-022"></a>

### ADR-022 — Identidad, versión y orden delegados al adaptador

**Contexto.** `NewVersion`/`PublishedVersion` ya son genéricos salvo `is_semver2`, cuyo
significado es de NuGet. Cada ecosistema normaliza y ordena distinto: NuGet (ADR-008), SemVer
de npm, PEP 440 de Python.

**Decisión.**

- El dominio conserva `package_id`, `package_key`, `version`, `version_key`, `full_version`,
  `is_prerelease`, `metadata` y `search_text` como datos opacos.
- Cada adaptador valida, normaliza y ordena; el registro nunca interpreta reglas del
  ecosistema.
- `is_prerelease` es común: existe en los tres protocolos.
- `is_semver2` se documenta como **señal exclusiva de NuGet** para `semVerLevel`. Los
  adaptadores npm y PyPI escriben `false` y sus rutas no lo leen. No se renombra en v2 para
  no romper el esquema ni los filtros existentes; queda como deuda explícita a revisar si
  aparece otro ecosistema SemVer.
- El orden de versiones (registro, `latest`, páginas) lo calcula el adaptador; `storage`
  devuelve filas sin ordenar por versión.

**Ejemplos y contraejemplos.**

| Ecosistema | Id publicado | `package_key` | Versión publicada | `version_key` | `full_version` | Nota |
|---|---|---|---|---|---|---|
| NuGet | `Hemia.Logging` | `hemia.logging` | `1.0.0-Beta.1+sha.abc` | `1.0.0-beta.1` | `1.0.0-Beta.1+sha.abc` | Metadatos de build fuera de la identidad |
| NuGet | `hemia.logging` | `hemia.logging` | `1.0.0` | `1.0.0` | `1.0.0` | Segunda publicación: `409` con contenido distinto |
| npm | `@acme/widgets` | `@acme/widgets` | `1.2.3-beta.1+build.5` | `1.2.3-beta.1` | `1.2.3-beta.1+build.5` | Build fuera de la identidad SemVer |
| npm | `Widgets` | — | — | — | — | Inválido: npm exige minúsculas |
| PyPI | `Hemia_Logging` | `hemia-logging` | `1.0rc1` | `1.0rc1` | `1.0rc1` | Normalización PEP 503 y PEP 440 |
| PyPI | `hemia.logging` | `hemia-logging` | `1.0` | `1.0` | `1.0` | Colisiona con `hemia-logging`; `409`, no dos paquetes |

Reglas de identidad por ecosistema (el adaptador las implementa y las prueba con corpus):

- **NuGet:** caso-insensible; versión `1`, `1.0`, `1.0.0` y `1.0.0+meta` son la misma
  identidad; prerelease y SemVer 2.0.0 según ADR-008.
- **npm:** nombre en minúsculas, scoped `@scope/name` o no scoped; versión SemVer válida;
  el build metadata no entra en la identidad; los dist-tags (`latest`, `next`) no son
  versiones y no crean identidad.
- **PyPI:** nombre normalizado con PEP 503 (`_`, `.`, mayúsculas → `-` minúscula); versión
  PEP 440 canónica; las versiones locales (`+local`) se conservan tal cual en la identidad
  (no se colapsan); prerelease/release/post según PEP 440.

**Consecuencias.** Los corpus de conformidad son obligatorios por adaptador, como el de
`crates/nuget/tests/version_corpus.rs`. El coste de un ecosistema nuevo queda acotado a un
crate de adaptador más su corpus.

<a id="adr-023"></a>

### ADR-023 — Múltiples archivos por versión con `version_file`

**Contexto.** `package_version` guarda un único `blob_sha256`. NuGet publica un `.nupkg` por
versión, pero PyPI admite varios archivos por versión (varias wheels y sdist) y npm publica
un tarball más metadatos. Python es obligatorio en v2.

**Decisión.**

- Se añade `version_file(version_id, filename, kind, sha256, size)` con primaria
  `(version_id, filename)` y FK a `blob`.
- `package_version.blob_sha256` se conserva como **artefacto primario** para no romper las
  consultas v1. La migración rellena `version_file` con ese blob y un nombre sintetizado por
  el adaptador NuGet (`{id}.{version}.nupkg` en minúsculas, como el flat container).
- `kind` distingue el rol del archivo (`package`, `tarball`, `wheel`, `sdist`, `metadata`).
  La lista exacta se fija en OP-023/OP-027; el valor es opaco para `storage`.
- Un `filename` es inmutable: subir el mismo nombre con bytes distintos responde `409`
  (`VERSION_FILE_CONFLICT`). La comparación es exacta sobre el nombre; el adaptador
  normaliza cuando su protocolo lo exige. Subir otro nombre a una versión existente es
  válido solo para ecosistemas que lo permiten; el adaptador decide (PyPI sí; NuGet no).
- La cuota del feed suma el tamaño de los archivos de cada versión. Un blob compartido entre
  versiones se cuenta una vez por versión, como hoy.
- En el primer corte no se borran archivos desde el registro; retirar (yank/unlist) es un
  cambio de estado, no una eliminación.

**Consecuencias.** PyPI puede publicar varias distribuciones sin duplicar la fila de versión
ni inventar identidades. NuGet sigue sirviendo el artefacto primario sin cambios.

<a id="adr-024"></a>

### ADR-024 — Rutas, capacidades y contratos aditivos por protocolo

**Contexto.** El contrato `/api/v1` es aditivo y tolerante a campos desconocidos; el CLI
negocia capacidades. Las rutas actuales son solo `/nuget/{feed}`.

**Decisión.**

- Rutas namespaced por protocolo y feed: `/nuget/{feed}/...` (sin cambios), `/npm/{feed}/...`
  y `/pypi/{feed}/...`. La superficie exacta la fijan OP-023 y OP-027.
- Un cliente de protocolo que golpea un feed de otro formato recibe `404` con
  `FEED_NOT_FOUND`, igual que un feed inexistente: no se filtra existencia entre protocolos.
- Capacidades nuevas: `formats` y `formats.nuget`, `formats.npm`, `formats.pypi`. El servidor
  anuncia solo los adaptadores compilados; un CLI viejo ignora los valores nuevos.
- `/api/v1/feeds` añade `format` (`#[serde(default)]` → `nuget`); `POST /api/v1/feeds` acepta
  `format` opcional; `PATCH` no puede cambiarlo (`FEED_FORMAT_IMMUTABLE`).
- Métricas: la etiqueta `surface` pasa a `nuget`, `npm`, `pypi`, `admin` o `probe`.
- El CLI gana `feed create --format <nuget|npm|pypi>` y muestra el formato; los comandos de
  configuración por ecosistema llegan en OP-025 y OP-034.
- Errores nuevos: `FEED_FORMAT_IMMUTABLE`, `VERSION_FILE_CONFLICT`,
  `FEED_FORMAT_MISMATCH` (uso interno si una ruta y un feed no coinciden). Se conservan los
  códigos estables actuales.

**Compatibilidad.** Un CLI v1 contra un servidor v2: `format` desconocido se ignora;
`create feed` sin `format` crea `nuget`. Un CLI v2 contra un servidor v1: las capacidades
`formats.*` faltan y los comandos de otros ecosistemas fallan antes de llamar. Los clientes
NuGet no ven ninguna diferencia.

<a id="adr-025"></a>

### ADR-025 — `feed_format` como tabla autoritativa; el CHECK v1 queda como legado

**Contexto.** SQLite no permite ampliar un `CHECK` sin reconstruir la tabla, y `feed` tiene
FKs desde `package`, `package_version`, `feed_grant` y `audit_event`. Reconstruir la tabla en
una migración forward-only es más arriesgado que añadir una tabla.

**Decisión.** La migración `0006` crea:

```sql
CREATE TABLE feed_format (
    feed_id INTEGER PRIMARY KEY REFERENCES feed (id) ON DELETE CASCADE,
    format  TEXT NOT NULL CHECK (format IN ('nuget', 'npm', 'pypi'))
) STRICT;
```

- Rellena todas las filas existentes con `nuget`.
- `create_feed` sigue insertando `'nuget'` en la columna legada para satisfacer su `CHECK` y
  escribe el formato real en `feed_format`. La columna `feed.format` no se lee para decidir
  comportamiento; se documenta como legado de v1 y se eliminará cuando una reconstrucción de
  tabla esté justificada.
- El formato de un feed se lee con `LEFT JOIN feed_format`, con `nuget` como valor por
  defecto si la fila faltara.

**Consecuencias.** Migración segura, sin reconstrucción ni riesgo de FKs, y compatible con
bases v1. El costo es una columna legada que miente para feeds nuevos; queda registrada como
deuda con condición de salida (reconstrucción de `feed` en una migración futura).

<a id="adr-026"></a>

### ADR-026 — Frontera de crates: adaptadores fuera de `core`/`storage`

**Contexto.** `onepack-nuget` ya concentra validación, normalización, inspección y JSON de
NuGet. La regla ADR-003 prohíbe que `core`, `storage` y el dominio dependan de un ecosistema.

**Decisión.**

- Nuevos crates `onepack-npm` y `onepack-pypi`, espejo de `onepack-nuget`: inspección de
  archivos, normalización, orden y serialización de protocolo.
- El servidor gana módulos de rutas por protocolo (`npm_api`, `pypi_api`) y comparte el flujo
  común de subida: staging → límite → `tokio::task::spawn_blocking` → inspección del
  adaptador → `NewVersion` + archivos → `store.publish`.
- `backfill_metadata` deja de llamar a `onepack_nuget` directo: el adaptador del feed
  aporta el lector (registro por formato en el servidor).
- `core` y `storage` no reciben dependencias de ecosistema. Si aparece una interfaz común,
  será un rasgo mínimo definido en `core` con implementaciones en los crates de adaptador;
  no se crea una fábrica de adaptadores antes de que existan dos consumidores reales.

**Consecuencias.** Un ecosistema nuevo es un crate y un módulo de rutas. El binario crece con
los adaptadores compilados, como ya anuncia `capabilities`.

## 4. Permisos y patrones de publicación

Los roles siguen siendo `reader`, `publisher` y `maintainer`, acumulativos, con `is_admin`
global. La política de patrón es del registro y se aplica sobre `package_key`; su sintaxis
debe admitir la forma de cada ecosistema.

| Ecosistema | Patrón válido | `package_key` | ¿Coincide? |
|---|---|---|---|
| NuGet | `Hemia.Payments.*` | `hemia.payments.core` | Sí |
| NuGet | `Hemia.Payments.*` | `hemia.payments.core.extra` | Sí |
| NuGet | `Hemia.Payments.*` | `hemia.paymentsx` | No (el prefijo incluye el punto) |
| npm | `@acme/*` | `@acme/widgets` | Sí |
| npm | `@acme/widgets` | `@acme/widgets` | Sí |
| npm | `widgets` | `widgets` | Sí (`@acme/widgets` no coincide) |
| PyPI | `hemia-*` | `hemia-logging` | Sí |
| PyPI | `hemia-logging` | `hemia.logging` | Sí (normalizado a `hemia-logging`) |

Decisión: `PublishPattern` conserva el comodín final `*` y la comparación exacta o por
prefijo, en minúsculas. La validación de caracteres se delega al adaptador en el borde de la
API; el core deja de rechazar `@` y `/`. Un patrón vacío sigue significando «cualquier id del
feed». Sin patrones, no hay restricción adicional.

## 5. Contratos de API, CLI y errores

### API administrativa (`/api/v1`)

| Cambio | Forma | Compatibilidad |
|---|---|---|
| `Feed.format` | `"nuget" \| "npm" \| "pypi"` | Aditivo; ausente se interpreta `nuget` |
| `CreateFeed.format` | opcional; por defecto `nuget` | Un cliente v1 sigue creando NuGet |
| Capacidades | `formats`, `formats.nuget`, `formats.npm`, `formats.pypi` | Aditivas |
| `PATCH /feeds/{feed}` | no acepta `format` | `409 FEED_FORMAT_IMMUTABLE` |
| `PackageVersion` | sin cambio en v2; los archivos se exponen en la ruta del protocolo | Aditivo si se añade `files` después |

### CLI

| Comando | v1 | v2 |
|---|---|---|
| `onepack feed create` | crea NuGet | `--format` opcional; sin flag crea NuGet |
| `onepack feed show/list` | sin formato | muestran `format` |
| `onepack doctor` | comprueba feeds NuGet | comprueba el formato del feed y las capacidades del servidor |
| `onepack npm init` / `onepack python init` | — | OP-025 / OP-034 |

### Errores estables nuevos

| Código | Cuándo | Respuesta |
|---|---|---|
| `FEED_FORMAT_IMMUTABLE` | `PATCH` intenta cambiar `format` | `409` |
| `VERSION_FILE_CONFLICT` | mismo `filename`, bytes distintos | `409` |
| `FEED_FORMAT_MISMATCH` | ruta de protocolo sobre feed de otro formato | `404` visible como `FEED_NOT_FOUND` |

## 6. Migración y compatibilidad

La migración `0006_multiecosistema.sql` es forward-only (ADR-020) y hace todo en una
transacción:

1. Crea `feed_format` y `version_file`.
2. Rellena `feed_format` con `nuget` para cada feed existente.
3. Rellena `version_file` desde `package_version.blob_sha256` con `kind = 'package'` y un
   nombre sintetizado por el adaptador NuGet.
4. No toca `feed`, `package`, `package_version`, grants ni auditoría.

Antes de migrar se sigue el procedimiento de backup vigente
([docs/runbooks/backup-restore.md](../docs/runbooks/backup-restore.md)); el ensayo de
`onepackd backup` + migración + restauración es parte de la evidencia de OP-022. El backup
existente ya cubre las tablas nuevas y los blobs; no se introduce un formato de datos
distinto ni una recuperación nueva.

Compatibilidad verificable:

- Feeds y datos v1 siguen siendo NuGet y responden en las mismas rutas.
- Un `package_version` existente conserva su blob primario y gana una fila en
  `version_file`; las descargas NuGet no cambian.
- El CLI v1 habla con el servidor v2 y viceversa (campos nuevos tolerados; comandos nuevos
  bloqueados por capacidades).
- Los permisos, tokens, auditoría y cuotas se reutilizan sin cambios de significado.

## 7. Plan de implementación para OP-022 (handoff)

Orden mínimo sugerido, cada paso con su prueba:

1. Migración `0006` y pruebas de migración desde una base v1 con feeds y versiones.
2. `feed_format` en `storage` y `Feed.format` en `core`/`api-client`; `create_feed` con
   formato.
3. `version_file` en `storage` y lectura de archivos; cuota por archivos.
4. Flujo común de publicación extraído de `nuget_api`; registro de adaptadores en el
   servidor.
5. Rutas npm y PyPI vacías que respondan `FEED_FORMAT_MISMATCH` cuando corresponda; superficie
   de métricas por protocolo.
6. Adaptadores reales en OP-024 y OP-033; specs en OP-023 y OP-027.

Pruebas mínimas que OP-022 debe entregar:

- Migración: base v1 con feeds, versiones y blobs → `feed_format` y `version_file`
  completos; mismo contenido descargable y mismas cuotas.
- Fronteras: `./scripts/check-crate-deps.sh` y la matriz de dependencias confirman que
  `core`/`storage` no dependen de `onepack-nuget`, `onepack-npm` ni `onepack-pypi`.
- Compatibilidad: CLI v1 contra servidor v2 y CLI v2 contra servidor v1 (crear/leer feeds,
  `format` ausente); clientes NuGet sin regresión.
- Aislamiento: un token con grant en un feed no puede leer ni publicar en un feed de otro
  formato, y un feed de formato A no responde en la ruta del formato B.
- Conformidad: corpus de identidad/versión por adaptador antes de exponer sus rutas.

## 8. Decisiones pendientes para OP-023 y OP-027

- **npm:** matriz de clientes (npm 10/11; pnpm/yarn fuera del primer corte), mecanismo de
  autenticación de publicación, política de dist-tags (mover `latest`, no borrar), límites de
  tarball y corpus SemVer.
- **PyPI:** versiones de pip y twine, índice simple (PEP 503) y JSON (PEP 691), mapeo de
  yanked (PEP 592) sin borrar bytes, corpus PEP 440, y si se permite un archivo nuevo en una
  versión ya publicada (ADR-023 dice que el adaptador decide; la spec debe fijarlo).
- **Orden de versiones:** corpus ejecutables en CI, como el de NuGet, para npm y PyPI.
- **Búsqueda:** `search_text` y metadatos por adaptador; sin búsqueda cruzada entre formatos.

## 9. Verificación de esta especificación

- AC-01: inventario de la sección 2, con el `CHECK format IN ('nuget')` y `is_semver2`
  señalados.
- AC-02: ADR-021 (feeds) y la tabla de identidad con ejemplos y contraejemplos de los tres
  protocolos; decisión aceptada el 2026-10-19.
- AC-03: secciones 4–6 con contratos de API/CLI, errores, migración y compatibilidad; `core`
  y `storage` conservan su frontera sin dependencias NuGet.
- AC-CIERRE: cerrado con la aceptación registrada del 2026-10-19, la nota OP-021 y el
  backlog sincronizados.

Evidencia: ruta de este documento, SHA-256 del contenido aceptado, commit local `514e439`
(entrega) y la aceptación registrada en Obsidian el 2026-10-19.
