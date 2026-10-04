# Architecture Decision Records — MVP registro privado NuGet

> Decisiones de arquitectura del MVP descrito en [mvp-registro-privado-paquetes-nuget.md](mvp-registro-privado-paquetes-nuget.md) y planificado en [roadmap-mvp.md](roadmap-mvp.md).
>
> Última actualización: 2026-10-03

---

## Cómo usar este documento

- Cada ADR registra **una** decisión, su contexto, alternativas y consecuencias.
- Un ADR `Aceptado` no se edita en su fondo: si la decisión cambia, se crea un ADR nuevo que lo **reemplaza** y el anterior pasa a `Reemplazado por ADR-XXX`.
- Correcciones de redacción, enlaces o la sección *Revisar cuando* sí pueden editarse.
- Cada fase del roadmap indica qué ADRs deben estar `Aceptado` antes de pasar a `EN PROGRESO`.

### Estatus de ADR

| Estatus | Significado |
|---|---|
| 📝 `Propuesto` | Redactado, pendiente de revisión. No vincula todavía. |
| 🧪 `Condicionado` | Aceptado de forma provisional hasta superar una validación concreta (indicada en el ADR). |
| ✅ `Aceptado` | Vigente. El código debe respetarlo. |
| ❌ `Rechazado` | Evaluado y descartado. Se conserva como registro. |
| ♻️ `Reemplazado por ADR-XXX` | Fue vigente; otra decisión lo sustituye. |
| 🗄️ `Obsoleto` | Ya no aplica (p. ej. el componente desapareció), sin reemplazo directo. |

### Plantilla

```markdown
<a id="adr-XXX"></a>
## ADR-XXX — Título en forma de decisión

**Estatus:** 📝 Propuesto · **Fecha:** AAAA-MM-DD · **Fase:** N · **Relacionados:** ADR-YYY

### Contexto
Qué problema o fuerza obliga a decidir.

### Decisión
Lo que se hará, en una o pocas frases afirmativas.

### Alternativas consideradas
- Opción — por qué no.

### Consecuencias
- Positivas / negativas / obligaciones que genera.

### Revisar cuando
Señal concreta que justificaría reabrir la decisión.
```

---

## Índice

| ADR | Decisión | Estatus | Fase |
|---|---|---|---|
| [001](#adr-001) | MVP: registro NuGet privado, una organización, un nodo | ✅ Aceptado | 0 |
| [002](#adr-002) | Rust + Tokio + Axum como stack del servidor y CLI | ✅ Aceptado | 1 |
| [003](#adr-003) | Monolito modular en un workspace de crates | ✅ Aceptado | 0 |
| [004](#adr-004) | SQLite en WAL con `synchronous=FULL` para metadatos | ✅ Aceptado | 2 |
| [005](#adr-005) | Blobs direccionados por SHA-256 en filesystem local, bytes originales | ✅ Aceptado | 2 |
| [006](#adr-006) | Blob durable antes de confirmar metadatos | ✅ Aceptado | 2 |
| [007](#adr-007) | Versiones inmutables y `409` ante duplicados | ✅ Aceptado | 2 |
| [008](#adr-008) | Normalización NuGet propia validada contra `NuGet.Versioning` | ✅ Aceptado | 1 |
| [009](#adr-009) | Recursos NuGet V3 anunciados en el MVP | ✅ Aceptado | 1, 3 |
| [010](#adr-010) | Tokens opacos con verificador hash, no JWT | ✅ Aceptado | 4 |
| [011](#adr-011) | Tres contextos de autenticación separados | ✅ Aceptado | 4 |
| [012](#adr-012) | Roles por feed y restricción opcional por prefijo | ✅ Aceptado | 4 |
| [013](#adr-013) | `listed` y `availability` como estados independientes | 📝 Propuesto | 3, 5 |
| [014](#adr-014) | Paquetes como contenido no confiable con presupuesto de inspección | 📝 Propuesto | 5 |
| [015](#adr-015) | `onepackd` y `onepack` separados; CLI con contrato estable | 📝 Propuesto | 6 |
| [016](#adr-016) | Credenciales NuGet vía `onepack exec`; credential provider pospuesto | 📝 Propuesto | 6 |
| [017](#adr-017) | Backup consistente en modo mantenimiento | 📝 Propuesto | 7 |
| [018](#adr-018) | `public_url` explícito y blobs solo a través del servidor | 📝 Propuesto | 3, 7 |
| [019](#adr-019) | Conformidad probada con clientes .NET reales, sin .NET en runtime | ✅ Aceptado | 0, 1 |
| [020](#adr-020) | Migraciones forward-only con backup previo | ✅ Aceptado | 2, 7 |

---

<a id="adr-001"></a>
## ADR-001 — MVP: registro NuGet privado, una organización, un nodo

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-03 · **Fase:** 0 · **Relacionados:** ADR-002, ADR-004

### Contexto
Existen alternativas ligeras (BaGet, BaGetter). "NuGet self-hosted y ligero" no diferencia. Cada funcionalidad adicional (proxy, S3, SSO, UI, multinodo) abre un frente propio de compatibilidad, seguridad y operación.

### Decisión
El MVP es un registro **privado** de paquetes NuGet, para **una organización**, con **varios feeds**, en **un único nodo**, operado **desde CLI**. La diferenciación es: instalación simple, operación completa desde terminal, credenciales acotadas, versiones inmutables y recuperación predecible.

Fuera del MVP: UI web, proxy de nuget.org, feeds virtuales, S3, PostgreSQL, HA, SSO, npm/PyPI, símbolos, antimalware.

### Alternativas consideradas
- **Registro multiformato desde el inicio** — multiplica el trabajo de compatibilidad antes de validar demanda.
- **Proxy universal de dependencias** — introduce acceso saliente, credenciales upstream y colisiones de identidad; es un proyecto de seguridad propio.
- **Multi-tenant SaaS** — exige aislamiento y operación que no aportan a la validación inicial.

### Consecuencias
- Los proyectos usan simultáneamente el feed privado y nuget.org con `packageSourceMapping`.
- El dominio debe quedar desacoplado de NuGet para no cerrar la puerta a otros formatos (ver ADR-003).
- Cualquier inclusión de funcionalidades excluidas requiere un ADR nuevo.

### Revisar cuando
El piloto (Fase 8) muestre que una funcionalidad excluida es condición para adoptar el producto.

---

<a id="adr-002"></a>
## ADR-002 — Rust + Tokio + Axum como stack del servidor y CLI

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 1 · **Relacionados:** ADR-008, ADR-019

### Contexto
El producto prioriza binarios nativos, pocos componentes, control de recursos y un CLI central. C#/ASP.NET Core reduciría el riesgo de NuGet al reutilizar `NuGet.Packaging`, `NuGet.Versioning` y `NuGet.Protocol`.

### Decisión
Servidor y CLI en **Rust**: Tokio, Axum, Tower/tower-http, SQLx (SQLite), clap, reqwest, serde (JSON/TOML).

**Condición:** la decisión se ratifica solo si el spike de la Fase 1 demuestra publish/restore con `dotnet` real y cero divergencias de normalización frente a `NuGet.Versioning`. Si no, se crea un ADR que lo reemplace a favor de C#.

**Validación:** condición cumplida el 2026-10-04: 0 divergencias frente a `NuGet.Versioning` y E2E con `dotnet` en verde ([CI](https://github.com/Khr0x/onepack/actions/runs/37176042099), [informe](../docs/spikes/fase-1-compatibilidad-nuget.md)).

### Alternativas consideradas
- **C# / ASP.NET Core** — menor riesgo de compatibilidad; runtime más pesado y CLI menos nativo. Descartada tras el spike de la Fase 1.
- **Go** — viable, sin ventaja clara frente a Rust y con el mismo trabajo de compatibilidad NuGet.

### Consecuencias
- Hay que implementar en Rust la normalización de versiones y la lectura del `.nuspec` (ver ADR-008).
- .NET se usa solo en pruebas (ver ADR-019).
- Sin Redis, Elasticsearch ni colas externas.

### Revisar cuando
Falle el gate de la Fase 1, o el coste acumulado de compatibilidad NuGet supere de forma sostenida al del resto del producto.

---

<a id="adr-003"></a>
## ADR-003 — Monolito modular en un workspace de crates

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-03 · **Fase:** 0 · **Relacionados:** ADR-001

### Contexto
Un único nodo y un equipo pequeño no justifican microservicios, pero el dominio no debe convertirse en "reglas NuGet" si se añaden formatos.

### Decisión
Un proceso servidor con dos superficies HTTP (NuGet y `/api/v1`), organizado en crates:

| Crate | Responsabilidad | No puede depender de |
|---|---|---|
| `core` | Dominio: feeds, versiones, permisos, casos de uso | `nuget`, `sqlx`, `axum` |
| `nuget` | Adaptador: lectura de paquetes, normalización, rutas del protocolo | `server`, `cli` |
| `storage` | Persistencia SQLite y `BlobStore` | `nuget`, `axum` |
| `api-client` | Contratos de la API admin compartidos | `server` |
| `server` | Binario `onepackd`, composición y HTTP | — |
| `cli` | Binario `onepack` | `server`, `storage` |

Tareas de mantenimiento internas en el mismo proceso. Sin plugins dinámicos.

### Alternativas consideradas
- **Microservicios** — coste operativo sin beneficio a esta escala.
- **Crate único** — más rápido al inicio, pero las fronteras se erosionan sin que el compilador lo impida.

### Consecuencias
- Las reglas de dependencia se verifican en CI con `scripts/check-crate-deps.sh` (sobre `cargo tree`, incluye dependencias transitivas). Los paquetes se llaman `onepack-<crate>`.
- Añadir un formato = nuevo crate adaptador, no cambios en `core`.

### Revisar cuando
Aparezca un requisito real de escalar una superficie de forma independiente.

---

<a id="adr-004"></a>
## ADR-004 — SQLite en WAL con `synchronous=FULL` para metadatos

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 2 · **Relacionados:** ADR-001, ADR-017

### Contexto
El MVP no debe exigir instalar otra base de datos. SQLite en WAL admite lectores concurrentes y **un solo escritor**, y no es apto para compartirse por filesystem de red.

### Decisión
- SQLite como único almacén de metadatos, en modo WAL.
- `synchronous=FULL` por defecto (durabilidad ante pérdida de energía), evaluando su coste en benchmarks.
- Escrituras serializadas por un único punto de escritura; transacciones cortas; `busy_timeout` configurado.
- Restricciones de integridad en la base (`UNIQUE`, `FOREIGN KEY`), no solo en código.
- Un servidor activo, disco local. **No** réplicas compartiendo el archivo.

### Alternativas consideradas
- **PostgreSQL** — añade un componente a instalar y operar; se reserva para cuando haya necesidad de multinodo demostrada.
- **`synchronous=NORMAL`** — mejor rendimiento, pero puede perder transacciones confirmadas ante pérdida de energía.

### Consecuencias
- Techo de escritura conocido; aceptable para el volumen de publicación esperado.
- Las descargas no deben generar escrituras síncronas (contadores agregados en memoria y volcados por lotes).
- El backup debe usar la API de backup de SQLite (ver ADR-017).

### Revisar cuando
Los benchmarks muestren contención de escritura relevante o se requiera más de un nodo.

---

<a id="adr-005"></a>
## ADR-005 — Blobs direccionados por SHA-256 en filesystem local, bytes originales

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 2 · **Relacionados:** ADR-006, ADR-018

### Contexto
Los `.nupkg` pueden ser grandes; guardarlos en SQLite penaliza la base y el backup. Se necesita poder responder "¿cuál es el archivo exacto que distribuimos?".

### Decisión
- El `.nupkg` se guarda **exactamente** como se recibió, en `blobs/sha256/<2>/<2>/<hash>`.
- El hash sirve para integridad y deduplicación; **nunca** como credencial ni como ruta pública.
- Acceso a través de una abstracción `BlobStore` mínima (put desde staging, get en streaming, exists, delete).
- El `.nuspec` se sirve extrayéndolo del blob (o cacheado), no reempaquetando.

### Alternativas consideradas
- **Blobs en SQLite** — base enorme, backups lentos, peor streaming.
- **Ruta por id/versión** — rompe deduplicación y complica la inmutabilidad.
- **Descomprimir y reempaquetar** — altera los bytes y rompe firmas del autor.

### Consecuencias
- Un mismo blob puede estar referenciado por varias versiones/feeds; el borrado requiere conteo de referencias y periodo de gracia.
- Habilita un futuro driver S3 sin cambiar el dominio.

### Revisar cuando
Se implemente almacenamiento S3 compatible (primera evolución).

---

<a id="adr-006"></a>
## ADR-006 — Blob durable antes de confirmar metadatos

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 2 · **Relacionados:** ADR-004, ADR-005, ADR-007

### Contexto
SQLite y el filesystem no comparten transacción. Hay que elegir qué inconsistencia se tolera ante una caída.

### Decisión
Orden fijo de publicación:

1. Autenticar, autorizar, comprobar cuota.
2. Recibir en streaming a `staging/` con límite de tamaño.
3. Calcular hash e inspeccionar con límites (ADR-014).
4. Validar identidad, versión y metadatos.
5. `fsync` del archivo, `rename` atómico a su ruta definitiva, `fsync` del directorio.
6. Transacción única: metadatos de versión + referencia al blob + evento de auditoría.
7. Responder `201`; la versión es visible desde el commit.

Se tolera un **blob huérfano** (eliminable); nunca una **versión sin blob**. Ninguna transacción SQLite permanece abierta durante la subida.

### Alternativas consideradas
- **Metadatos primero, blob después** — puede exponer versiones sin artefacto.
- **Estado "pendiente" en BD durante la subida** — más escrituras y estados intermedios visibles.

### Consecuencias
- Tarea de mantenimiento que limpia staging y blobs sin referencias tras un periodo de gracia, pausada durante backups.
- `onepackd check` verifica la invariante BD → blob.

### Revisar cuando
Se introduzca un `BlobStore` remoto con semántica de consistencia distinta.

---

<a id="adr-007"></a>
## ADR-007 — Versiones inmutables y `409` ante duplicados

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 2 · **Relacionados:** ADR-006, ADR-008, ADR-013

### Contexto
Reemplazar el contenido de una versión publicada rompe la reproducibilidad y la trazabilidad, y el protocolo NuGet prevé `409` para identidades existentes.

### Decisión
- Identidad: `feed + normalized_package_id + normalized_version`, con `UNIQUE` en la base.
- Una versión publicada **no se reemplaza ni se borra** desde la API en el MVP. Para corregir, se publica otra versión; para retirar, unlist o block (ADR-013).
- Duplicado → `409`.
- `onepack package push --skip-existing-identical` trata el duplicado como éxito **solo** si el hash coincide; con bytes distintos sigue siendo error.

### Alternativas consideradas
- **Permitir sobrescritura con permiso de admin** — rompe cachés de clientes y la auditoría.
- **Hard delete** — se pospone; si llega, será una operación administrativa auditada, no parte del flujo normal.

### Consecuencias
- La carrera de dos publicaciones simultáneas se resuelve en la base: una gana, la otra recibe `409`.
- Los errores de publicación cuestan una versión; hay que documentarlo.

### Revisar cuando
Exista un requisito legal de eliminación (p. ej. contenido filtrado por error) — requerirá un ADR de purga auditada.

---

<a id="adr-008"></a>
## ADR-008 — Normalización NuGet propia validada contra `NuGet.Versioning`

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 1 · **Relacionados:** ADR-002, ADR-019

### Contexto
Las versiones NuGet no son SemVer puro: cuarto segmento, equivalencias (`1.0` ≡ `1.0.0`), metadatos de build ignorados en la identidad, ids case-insensitive. Un error aquí produce duplicados o colisiones.

### Decisión
- Implementar en el crate `nuget` la normalización de ids y versiones, y la comparación de versiones.
- Validarla con un **corpus de casos generado por `NuGet.Versioning`** (programa .NET de prueba) y ejecutado en CI.
- No usar una librería SemVer genérica como fuente de verdad.

### Alternativas consideradas
- **Crate SemVer genérico** — no cubre las reglas NuGet.
- **Llamar a .NET en runtime** — rompe ADR-002 y ADR-019.

### Consecuencias
- Cualquier divergencia detectada en el corpus es un bug bloqueante.
- El corpus crece con cada caso raro encontrado en el piloto.

### Revisar cuando
NuGet cambie sus reglas de versionado.

---

<a id="adr-009"></a>
## ADR-009 — Recursos NuGet V3 anunciados en el MVP

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 1, 3 · **Relacionados:** ADR-013, ADR-018

### Contexto
El cliente descubre capacidades en el service index. Anunciar un recurso sin cumplir su contrato provoca fallos difíciles de diagnosticar.

### Decisión
El service index anuncia **solo** estos recursos, y únicamente cuando estén completos:

| Recurso | Ruta |
|---|---|
| `PackageBaseAddress/3.0.0` | `/nuget/{feed}/v3/flat/` |
| `RegistrationsBaseUrl/3.6.0` | `/nuget/{feed}/v3/registration/` |
| `SearchQueryService` | `/nuget/{feed}/v3/query` |
| `SearchAutocompleteService` | `/nuget/{feed}/v3/autocomplete` |
| `PackagePublish/2.0.0` | `/nuget/{feed}/v2/package` |

No se implementa la API V2 (OData). `PackagePublish/2.0.0` es el recurso de publicación de V3, no la API V2.

### Alternativas consideradas
- **Implementar V2 OData** — amplía superficie para clientes antiguos sin demanda.
- **Anunciar todo desde el spike** — confunde a clientes mientras falten recursos.

### Consecuencias
- Se publica una matriz de clientes comprobados en lugar de prometer "compatibilidad total".
- Búsqueda debe respetar paginación, prerelease y `semVerLevel`.

### Revisar cuando
Un cliente relevante del piloto requiera un recurso no implementado (p. ej. `ReadmeUriTemplate`, `RepositorySignatures`).

---

<a id="adr-010"></a>
## ADR-010 — Tokens opacos con verificador hash, no JWT

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 4 · **Relacionados:** ADR-011, ADR-012

### Contexto
Se necesita expiración, revocación inmediata y permisos acotados. Los JWT de larga duración no se revocan sin una lista de bloqueo, lo que anula su ventaja.

### Decisión
- Token = prefijo identificable (p. ej. `pkg_`) + id público corto + secreto aleatorio ≥ 256 bits.
- Se almacena el id y un **verificador** (SHA-256 del secreto; suficiente por su alta entropía), nunca el token recuperable. Comparación en tiempo constante.
- Se muestra una sola vez al crearlo.
- Expiración obligatoria (configurable), revocación inmediata, `last_used_at` actualizado de forma agregada.

### Alternativas consideradas
- **JWT** — revocación compleja, riesgo de tokens de larga vida.
- **Argon2/bcrypt para el verificador** — innecesario con secretos de alta entropía y costoso por request.
- **API key global** — sin trazabilidad ni mínimo privilegio.

### Consecuencias
- Cada request autenticada consulta la base (mitigable con caché corta e invalidación al revocar).
- El prefijo permite detección por escáneres de secretos.

### Revisar cuando
Se integre SSO/OIDC o federación de identidades de CI (p. ej. OIDC de GitHub Actions).

---

<a id="adr-011"></a>
## ADR-011 — Tres contextos de autenticación separados

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 4 · **Relacionados:** ADR-010, ADR-016

### Contexto
Los clientes NuGet autentican lectura y publicación por mecanismos distintos, y el CLI administrativo tiene su propio canal.

### Decisión

| Contexto | Mecanismo |
|---|---|
| API administrativa `/api/v1` | `Authorization: Bearer <token>` |
| Lectura NuGet (index, flat, registration, search, autocomplete) | Basic sobre HTTPS; usuario ignorado o informativo, contraseña = token. `401` + `WWW-Authenticate: Basic` |
| Publicación NuGet | `X-NuGet-ApiKey: <token>` |

Todos resuelven al mismo modelo de principal y permisos. La autorización se aplica en **todas** las rutas, incluidas `HEAD` y respuestas condicionales.

**Precisión (Fase 4, 2026-10-04).** El contexto se determina por la superficie, no por la operación: en las rutas NuGet se aceptan tanto Basic como `X-NuGet-ApiKey`, porque `dotnet nuget push` consulta el service index con Basic (tras el `401`) y publica con la API key. La API administrativa acepta **solo** Bearer: así un navegador que haya cacheado credenciales Basic no puede usarlas contra ella.

### Alternativas consideradas
- **Un solo mecanismo** — no es compatible con lo que envían los clientes NuGet.
- **Proteger solo la publicación** — expone paquetes privados.

### Consecuencias
- Test automatizado que recorre todas las rutas registradas y verifica que exigen autenticación.
- `anonymous_read=false` por defecto.

### Revisar cuando
Se implemente el credential provider (ADR-016) o lectura anónima por feed.

---

<a id="adr-012"></a>
## ADR-012 — Roles por feed y restricción opcional por prefijo

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 4 · **Relacionados:** ADR-010, ADR-011

### Contexto
Una cuenta de CI debe poder publicar solo lo suyo. NuGet no tiene namespaces en el protocolo.

### Decisión
- Grants por `(principal, feed, rol)` con roles acumulativos: **Reader** ⊂ **Publisher** ⊂ **Maintainer**; **Administrator** es global (identidades, permisos, configuración).
- Restricción opcional de publicación por patrones de prefijo de id (`Hemia.Payments.*`), como **política del registro**.
- Feeds sin permiso de lectura responden `404` para no revelar su existencia.
- Bootstrap: `onepackd init` genera la credencial admin inicial en un archivo `0600`; sin contraseña por defecto ni registro público de admins.

### Alternativas consideradas
- **ACL por paquete** — granularidad sin demanda probada.
- **Roles globales sin feeds** — rompe el aislamiento por cliente.

### Consecuencias
- Errores de autorización con códigos estables (`AUTH_SCOPE_MISSING`, `AUTH_PREFIX_DENIED`).
- Los cambios de grants se auditan.

### Revisar cuando
Aparezcan requisitos de grupos/equipos o SSO.

---

<a id="adr-013"></a>
## ADR-013 — `listed` y `availability` como estados independientes

**Estatus:** 📝 Propuesto · **Fecha:** 2026-10-03 · **Fase:** 3, 5 · **Relacionados:** ADR-007

### Contexto
En NuGet, unlist retira de búsqueda pero el paquete sigue descargable. Eso no sirve para responder a incidentes.

### Decisión
Cada versión tiene dos atributos ortogonales:

```text
listed:       true | false          (descubrimiento; Publisher+ vía protocolo NuGet)
availability: available | blocked   (descarga; Maintainer+ vía /api/v1, con motivo)
```

- `listed=false`: fuera de búsqueda y autocompletado; presente en flat container y registros con `listed=false`; descargable.
- `blocked`: el servidor rechaza la descarga del `.nupkg` y `.nuspec`; visible para Maintainer/Admin.

### Alternativas consideradas
- **Un único estado** — mezcla una operación de curación con una de seguridad.

### Consecuencias
- Se documenta que bloquear no elimina copias ya descargadas en cachés de clientes.
- Bloqueos y desbloqueos se auditan con motivo.

### Revisar cuando
Se requiera cuarentena automática (p. ej. tras un escáner).

---

<a id="adr-014"></a>
## ADR-014 — Paquetes como contenido no confiable con presupuesto de inspección

**Estatus:** 📝 Propuesto · **Fecha:** 2026-10-03 · **Fase:** 5 · **Relacionados:** ADR-006

### Contexto
Un `.nupkg` es un ZIP con XML: expuesto a ZIP bombs, XML bombs, path traversal y fallos de parser.

### Decisión
- Límites configurables: tamaño comprimido, número de entradas, tamaño descomprimido por entrada y total, tamaño del `.nuspec`, profundidad XML, tiempo de inspección.
- Leer solo lo necesario del ZIP en memoria acotada; **no** extraer a disco.
- Parser XML sin DTD ni entidades externas.
- Rechazar rutas peligrosas.
- Inspección con concurrencia limitada, aislada de la atención de descargas.
- Nunca ejecutar ni descargar nada referenciado por el paquete.
- No anunciar análisis antimalware ni validación de firmas.

### Alternativas consideradas
- **Confiar en publicadores autenticados** — un token comprometido o un paquete defectuoso tumbaría el servicio.
- **Escaneo antimalware integrado** — fuera del MVP; requeriría dependencias externas.

### Consecuencias
- Paquetes legítimos muy grandes pueden requerir ajustar límites; los límites son del registro, no de NuGet.
- Suite `tests/security` con artefactos maliciosos sintéticos.

### Revisar cuando
Se añada verificación de firmas o integración con escáneres.

---

<a id="adr-015"></a>
## ADR-015 — `onepackd` y `onepack` separados; CLI con contrato estable

**Estatus:** 📝 Propuesto · **Fecha:** 2026-10-03 · **Fase:** 6 · **Relacionados:** ADR-003, ADR-016

### Contexto
El CLI es la interfaz principal del producto, usada por personas y por pipelines.

### Decisión
- `onepackd` (servidor) y `onepack` (cliente) son binarios distintos que comparten contratos vía `api-client`. Instalar `onepack` no implica el servidor.
- Contrato del CLI: `--json` con esquema estable, códigos de salida documentados, `--no-input`, timeouts, datos a stdout y diagnóstico a stderr.
- Credenciales: entrada sin eco o `--token-stdin`; keychain del SO; **sin fallback silencioso a texto plano** (si no hay keychain, error explícito y opción `--token-env`).
- Reintentos solo en operaciones idempotentes.
- Negociación de capacidades con el servidor (`/api/v1/capabilities`), compatibilidad N/N-1.
- Errores con código, acción sugerida y `request_id`, nunca con secretos.

### Alternativas consideradas
- **Un solo binario con subcomandos de servidor y cliente** — obliga a distribuir el servidor a todas las estaciones.
- **Script sobre `curl`** — sin manejo de credenciales ni contrato estable.

### Consecuencias
- Cambios incompatibles en `--json` o códigos de salida exigen versión mayor del CLI.
- `onepack doctor` se trata como funcionalidad de producto, no como utilidad de depuración.

### Revisar cuando
Se añada UI web o SDK de terceros sobre `/api/v1`.

---

<a id="adr-016"></a>
## ADR-016 — Credenciales NuGet vía `onepack exec`; credential provider pospuesto

**Estatus:** 📝 Propuesto · **Fecha:** 2026-10-03 · **Fase:** 6 · **Relacionados:** ADR-011, ADR-015

### Contexto
Guardar un token en el CLI no hace que `dotnet restore` lo lea. El cifrado de contraseñas en `NuGet.Config` solo funciona en Windows.

### Decisión
- **CI:** credenciales inyectadas por mecanismos estándar de NuGet (`NuGetPackageSourceCredentials_{source}` o `dotnet nuget add source` con secretos del pipeline).
- **Desarrollo local:** `onepack exec --feed X -- <comando>` define las variables de credencial solo en el entorno del proceso hijo.
- `onepack nuget init` genera `NuGet.Config` **sin secretos**, con `packageSourceMapping`, preservando lo existente y mostrando el diff.
- El credential provider multiplataforma se pospone a la primera evolución.

### Alternativas consideradas
- **Escribir el token en `NuGet.Config`** — texto plano fuera de Windows; riesgo de commit accidental.
- **Credential provider en el MVP** — mejor experiencia en IDE, pero protocolo de plugin adicional y más superficie.

### Consecuencias
- La experiencia en IDE requiere configuración manual documentada durante el MVP.
- Se documenta que `packageSourceMapping` no garantiza confidencialidad absoluta de ids, y que las pruebas de procedencia usan cachés limpias.

### Revisar cuando
El piloto identifique el uso desde IDE como fricción principal.

---

<a id="adr-017"></a>
## ADR-017 — Backup consistente en modo mantenimiento

**Estatus:** 📝 Propuesto · **Fecha:** 2026-10-03 · **Fase:** 7 · **Relacionados:** ADR-004, ADR-005, ADR-006

### Contexto
Un backup de la base sin blobs no recupera el registro. Copiar el archivo SQLite activo sin coordinación no es fiable.

### Decisión
- `onepackd backup` activa modo mantenimiento: rechaza mutaciones (`503` con `Retry-After`) y pausa la limpieza de blobs; las lecturas continúan.
- Copia la base con la **SQLite Online Backup API**, los blobs referenciados y la configuración necesaria (sin secretos de TLS salvo indicación explícita).
- Genera un manifiesto con hashes y versión de esquema.
- `onepackd restore` exige directorio vacío y verifica el manifiesto.
- **Criterio de éxito:** restaurar en otro servidor y que un proyecto ejecute `dotnet restore` con éxito.

### Alternativas consideradas
- **Backup en caliente sin pausa** — riesgo de blobs recolectados entre la copia de BD y la de blobs.
- **Delegar en snapshots del filesystem** — válido como complemento, no como único mecanismo portable.

### Consecuencias
- Ventana corta sin publicaciones durante el backup; se documenta.
- Backup incremental y en caliente quedan para evolución.

### Revisar cuando
El tamaño de datos haga inaceptable la ventana de mantenimiento.

---

<a id="adr-018"></a>
## ADR-018 — `public_url` explícito y blobs solo a través del servidor

**Estatus:** 📝 Propuesto · **Fecha:** 2026-10-03 · **Fase:** 3, 7 · **Relacionados:** ADR-005, ADR-009, ADR-011

### Contexto
Detrás de un reverse proxy, derivar URLs de la cabecera `Host` permite envenenamiento de URLs. Servir blobs directamente desde el proxy eludiría la autorización.

### Decisión
- `server.public_url` es obligatorio y es la única base para URLs absolutas del service index y recursos.
- TLS directo o terminado en reverse proxy; el servidor escucha por defecto en `127.0.0.1`.
- Los blobs se sirven **solo** por el servidor, tras autorización. Los ejemplos de Nginx/Caddy no exponen el directorio de datos.

### Alternativas consideradas
- **Derivar de `Host`/`X-Forwarded-*`** — manipulable.
- **`X-Accel-Redirect`/`X-Sendfile`** — optimización válida a futuro si se mantiene la autorización previa; no en el MVP.

### Consecuencias
- Un cambio de dominio requiere cambiar configuración y reiniciar.

### Revisar cuando
Las descargas grandes saturen el proceso y se necesite delegar el envío de bytes.

---

<a id="adr-019"></a>
## ADR-019 — Conformidad probada con clientes .NET reales, sin .NET en runtime

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-03 · **Fase:** 0, 1 · **Relacionados:** ADR-002, ADR-008, ADR-009

### Contexto
Probar solo con el CLI propio puede ocultar incompatibilidades compartidas entre cliente y servidor.

### Decisión
- `tests/conformance-dotnet` usa `dotnet` SDK, `nuget.exe` y pequeños programas .NET (p. ej. con `NuGet.Versioning`) como **referencia de pruebas**.
- Los paquetes de prueba se crean con herramientas oficiales (`dotnet pack`).
- Las pruebas de restore usan cachés aisladas (`NUGET_PACKAGES` temporal).
- .NET **no** es dependencia del servidor desplegado.

### Alternativas consideradas
- **Fixtures estáticos de respuestas** — no detectan regresiones de comportamiento de clientes.

### Consecuencias
- CI necesita SDK .NET y, para `nuget.exe`, un runner Windows.
- Se mantiene una matriz de clientes comprobados con versiones exactas.

### Revisar cuando
Se añada otro formato (npm, PyPI), que requerirá su propia suite de conformidad.

---

<a id="adr-020"></a>
## ADR-020 — Migraciones forward-only con backup previo

**Estatus:** ✅ Aceptado · **Fecha:** 2026-10-03 · **Aceptado:** 2026-10-04 · **Fase:** 2, 7 · **Relacionados:** ADR-004, ADR-017

### Contexto
Volver al binario anterior no revierte una migración de datos. Las migraciones "down" rara vez se prueban.

### Decisión
- Migraciones SQLx embebidas en `onepackd`, numeradas y **solo hacia adelante**.
- Al arrancar: comprobar versión de esquema; si hay migraciones pendientes, exigir `onepackd migrate` (o flag explícito), que genera un backup previo.
- Un binario se niega a arrancar con un esquema más nuevo que el que conoce.
- El rollback consiste en restaurar el backup previo con el binario anterior.

### Alternativas consideradas
- **Migraciones automáticas silenciosas al arrancar** — sorpresas en actualizaciones.
- **Migraciones reversibles** — coste de mantenimiento sin garantía real.

### Consecuencias
- Notas de versión indican si una release incluye migración.
- Prueba de actualización N-1 → N en CI.

### Revisar cuando
Se adopte PostgreSQL o despliegues multinodo.
