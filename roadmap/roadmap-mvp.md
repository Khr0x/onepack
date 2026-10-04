# Roadmap MVP — Registro privado de paquetes NuGet

> Plan por fases derivado de [mvp-registro-privado-paquetes-nuget.md](mvp-registro-privado-paquetes-nuget.md).
> Las decisiones de arquitectura están en [adr-mvp.md](adr-mvp.md).
>
> Última actualización: 2026-10-04

---

## Prueba decisiva del MVP

> Un equipo instala el servidor, configura su proyecto, publica una librería y la restaura desde CI; después revoca una credencial y recupera el servicio desde un backup, **sin abrir una interfaz web ni editar manualmente la base de datos**.

Todas las fases existen para que esa frase sea verificable de punta a punta en la Fase 8.

---

## Sistema de estatus

### Estatus de fase

| Estatus | Significado | Condición para entrar |
|---|---|---|
| ⚪ `NO INICIADA` | Planificada, sin trabajo activo. | Estado inicial. |
| 🔵 `EN DISEÑO` | Se definen alcance fino, ADRs y casos de prueba. Todavía no se escribe código de producto. | Dependencias en `EN VALIDACIÓN` o `COMPLETADA`. |
| 🟡 `EN PROGRESO` | Implementación activa de entregables. | ADRs de la fase en estado `Aceptado` y criterios de aceptación escritos. |
| 🟣 `EN VALIDACIÓN` | Entregables implementados; se ejecutan las pruebas bloqueantes y la revisión del gate. | Todos los entregables marcados como hechos. |
| 🟢 `COMPLETADA` | Gate de salida superado y evidencia registrada. | Todas las pruebas bloqueantes en verde y evidencia enlazada. |
| 🔴 `BLOQUEADA` | No puede avanzar. Debe indicar **motivo**, **condición de desbloqueo** y **responsable**. | Cualquier estado activo. |
| ⚫ `POSPUESTA` | Se saca del MVP de forma deliberada. Requiere ADR o nota de decisión. | Decisión explícita de alcance. |

```mermaid
stateDiagram-v2
    [*] --> NO_INICIADA
    NO_INICIADA --> EN_DISEÑO
    EN_DISEÑO --> EN_PROGRESO : ADRs aceptados
    EN_PROGRESO --> EN_VALIDACIÓN : entregables hechos
    EN_VALIDACIÓN --> EN_PROGRESO : prueba bloqueante falla
    EN_VALIDACIÓN --> COMPLETADA : gate superado
    EN_DISEÑO --> BLOQUEADA
    EN_PROGRESO --> BLOQUEADA
    EN_VALIDACIÓN --> BLOQUEADA
    BLOQUEADA --> EN_DISEÑO
    BLOQUEADA --> EN_PROGRESO
    NO_INICIADA --> POSPUESTA
    EN_DISEÑO --> POSPUESTA
    COMPLETADA --> [*]
```

**Reglas:**

1. Una fase solo pasa a `COMPLETADA` con evidencia enlazada (PR, reporte de pruebas, log de ejecución). "Funciona en mi máquina" no es evidencia.
2. Si una prueba bloqueante falla en `EN VALIDACIÓN`, la fase vuelve a `EN PROGRESO`; no se avanza con excepciones informales.
3. Una fase posterior puede empezar `EN DISEÑO` mientras su dependencia está `EN VALIDACIÓN`, pero no `EN PROGRESO`.
4. `BLOQUEADA` siempre lleva un bloque con: *motivo*, *desde* (fecha), *condición de desbloqueo*, *responsable*.
5. Un cambio que reabre una fase `COMPLETADA` se registra como regresión en la fase actual, no reabriendo la anterior.

### Estatus de entregable

Cada entregable dentro de una fase usa una casilla con marcador:

| Marcador | Significado |
|---|---|
| `[ ]` | Pendiente |
| `[~]` | En curso |
| `[x]` | Hecho (con evidencia) |
| `[!]` | Bloqueado (añadir nota) |
| `[-]` | Descartado / movido fuera de la fase (añadir nota) |

---

## Vista general

| # | Fase | Estatus | Depende de | Gate de salida (resumen) |
|---|---|---|---|---|
| 0 | Fundaciones del proyecto | 🟢 `COMPLETADA` | — | Workspace compila en CI y ADRs base aceptados. |
| 1 | Spike de compatibilidad NuGet | 🟣 `EN VALIDACIÓN` | 0 | `dotnet` real publica y restaura contra el servidor; decisión Rust/C# ratificada. |
| 2 | Núcleo de dominio y persistencia | ⚪ `NO INICIADA` | 1 | Publicación consistente, inmutable y resistente a caídas. |
| 3 | Superficie NuGet V3 completa | ⚪ `NO INICIADA` | 2 | Matriz de clientes comprobados en verde. |
| 4 | Identidad, autenticación y autorización | ⚪ `NO INICIADA` | 2 | Feeds aislados en todos los endpoints; tokens revocables. |
| 5 | Endurecimiento frente a paquetes y abuso | ⚪ `NO INICIADA` | 3, 4 | ZIP/XML maliciosos rechazados; bloqueo de versiones operativo. |
| 6 | API administrativa y CLI `onepack` | ⚪ `NO INICIADA` | 4 | Operación completa del registro desde terminal. |
| 7 | Operación, recuperación y distribución | ⚪ `NO INICIADA` | 5, 6 | Backup restaurado en otro servidor; binarios publicados. |
| 8 | Piloto y cierre del MVP | ⚪ `NO INICIADA` | 7 | Prueba decisiva ejecutada por un equipo real. |

```mermaid
flowchart LR
    F0[0 Fundaciones] --> F1[1 Spike NuGet]
    F1 --> F2[2 Dominio y persistencia]
    F2 --> F3[3 NuGet V3 completo]
    F2 --> F4[4 Identidad y permisos]
    F3 --> F5[5 Endurecimiento]
    F4 --> F5
    F4 --> F6[6 API admin y CLI]
    F5 --> F7[7 Operación y distribución]
    F6 --> F7
    F7 --> F8[8 Piloto]
```

Las fases 3 y 4 pueden avanzar en paralelo. La fase 6 puede avanzar en paralelo con la 5.

---

## Fase 0 — Fundaciones del proyecto

**Estatus:** 🟢 `COMPLETADA` — evidencia: [CI run 37174401011](https://github.com/Khr0x/onepack/actions/runs/37174401011).
**Depende de:** —
**ADRs:** [ADR-001](adr-mvp.md#adr-001), [ADR-003](adr-mvp.md#adr-003), [ADR-019](adr-mvp.md#adr-019)

### Objetivo
Dejar un repositorio donde cualquier cambio se compila, se prueba y se revisa de forma reproducible, y donde las decisiones base están registradas.

### Entregables
- [x] Workspace Cargo con crates vacíos: `core`, `nuget`, `storage`, `api-client`, `server`, `cli` (paquetes `onepack-*`). Fronteras de ADR-003 verificadas por `scripts/check-crate-deps.sh`.
- [x] Estructura de carpetas: `migrations/`, `tests/{integration,conformance-dotnet,security,recovery}`, `packaging/{systemd,container}`, `docs/`.
- [x] CI (`.github/workflows/ci.yml`, runners `ubuntu-24.04`): `cargo fmt --check`, `clippy -D warnings`, `cargo test`, fronteras entre crates, `cargo deny` (licencias, advisories, fuentes). [Verde en GitHub](https://github.com/Khr0x/onepack/actions/runs/37174401011).
- [x] Toolchain fijado (`rust-toolchain.toml`, 1.96.1); MSRV = `rust-version` 1.96, documentado en `docs/conventions.md`.
- [x] Job de CI con SDK .NET 8 que empaqueta las fixtures de `tests/conformance-dotnet/fixtures` (sin ser dependencia del servidor). [Verde en GitHub](https://github.com/Khr0x/onepack/actions/runs/37174401011).
- [x] Convenciones en `docs/conventions.md`: formato de errores, prefijos de códigos, códigos de salida provisionales del CLI, commits y ramas.
- [x] Nombres de binarios confirmados: `onepackd` (servidor) y `onepack` (CLI).
- [x] ADR-001, ADR-003 y ADR-019 en `Aceptado` (2026-10-03); ADR-002 sigue `Condicionado` hasta el gate de la Fase 1.

### Fuera de alcance
Cualquier endpoint funcional.

### Gate de salida
- Un PR de ejemplo pasa todo el pipeline de CI en Linux x86-64.
- ADRs base en `Aceptado`.

---

## Fase 1 — Spike de compatibilidad NuGet

**Estatus:** 🟣 `EN VALIDACIÓN` — pruebas bloqueantes verdes en local; falta el CI en GitHub.
**Depende de:** Fase 0
**ADRs:** [ADR-002](adr-mvp.md#adr-002), [ADR-008](adr-mvp.md#adr-008), [ADR-009](adr-mvp.md#adr-009), [ADR-019](adr-mvp.md#adr-019)

### Objetivo
Reducir el mayor riesgo técnico **antes** de invertir en administración, permisos o CLI: demostrar que un servidor Rust mínimo es aceptado por clientes NuGet reales. Esta fase es el punto de decisión para ratificar o revertir Rust frente a C#.

### Entregables
- [x] Servidor Axum mínimo con un feed fijo (`--feed`), sin autenticación, almacenamiento provisional en disco (`crates/server`).
- [x] `index.json` (service index) con solo los recursos implementados; URLs construidas desde `--public-url` (ADR-018).
- [x] `PackagePublish/2.0.0`: `PUT` multipart; `X-NuGet-ApiKey` se acepta sin validar. Acepta `/v2/package` y `/v2/package/` (el cliente añade la barra final).
- [x] `PackageBaseAddress/3.0.0`: lista de versiones en orden NuGet, descarga de `.nupkg` (bytes originales) y `.nuspec`.
- [x] Lectura del `.nuspec` desde el ZIP: id, versión y dependencias por framework (`crates/nuget/src/package.rs`).
- [x] Normalización de id y versión NuGet (`crates/nuget/src/{id,version}.rs`).
- [x] Suite de conformidad: corpus de 168 casos generado con `NuGet.Versioning` 7.9.0 y verificado en CI (`scripts/version-corpus.sh --check`, `crates/nuget/tests/version_corpus.rs`).
- [x] Prueba E2E `scripts/e2e-dotnet.sh`: `dotnet pack` → `dotnet nuget push` → `dotnet restore` con caché aislada → `dotnet run`.
- [x] Informe del spike: [docs/spikes/fase-1-compatibilidad-nuget.md](../docs/spikes/fase-1-compatibilidad-nuget.md).

### Pruebas bloqueantes
| Escenario | Resultado esperado |
|---|---|
| Push y restore con `dotnet` SDK actual | Éxito con caché vacía. |
| Versiones `1.0`, `1.0.0`, `1.0.0.0` | Tratadas como la misma identidad. |
| Prerelease y metadatos de build | Normalización idéntica a `NuGet.Versioning`. |
| Paquete con dependencias por framework | El restore resuelve la dependencia transitiva. |

### Gate de salida
- E2E verde en CI.
- Divergencias de normalización = 0 sobre el corpus de casos.
- ADR-002 pasa de `Condicionado` a `Aceptado` (Rust) **o** se reemplaza por un ADR a favor de C#. Si se cambia de stack, se reescribe este roadmap desde la Fase 2.

---

## Fase 2 — Núcleo de dominio y persistencia

**Estatus:** ⚪ `NO INICIADA`
**Depende de:** Fase 1
**ADRs:** [ADR-004](adr-mvp.md#adr-004), [ADR-005](adr-mvp.md#adr-005), [ADR-006](adr-mvp.md#adr-006), [ADR-007](adr-mvp.md#adr-007), [ADR-020](adr-mvp.md#adr-020)

### Objetivo
Convertir el spike en un núcleo durable: varios feeds, versiones inmutables, publicación consistente aunque el proceso caiga.

### Entregables
- [ ] Esquema SQLite con migraciones SQLx embebidas: `feed`, `package`, `package_version`, `blob`, `audit_event` (identidades en Fase 4).
- [ ] `UNIQUE(feed_id, normalized_package_id, normalized_version)` en la base.
- [ ] SQLite en WAL, `synchronous=FULL`, `busy_timeout`, un pool de escritura serializado.
- [ ] Abstracción `BlobStore` con implementación filesystem: `blobs/sha256/ab/cd/<hash>`, `staging/`.
- [ ] Flujo de publicación: streaming a staging → hash → inspección → `fsync` del archivo y directorio → rename atómico → transacción de metadatos + auditoría.
- [ ] Ninguna transacción SQLite abierta durante la subida.
- [ ] Conflicto de identidad → `409`; contenido idéntico detectable para `--skip-existing-identical`.
- [ ] Tarea de mantenimiento: limpieza de staging y de blobs huérfanos con periodo de gracia.
- [ ] Verificación al arranque: esquema, permisos de directorio, espacio libre.
- [ ] Separación de crates respetada: `core` no depende de `nuget` ni de `sqlx`.
- [ ] Gestión de feeds por configuración interna o tests (la gestión por CLI llega en Fase 6).

### Pruebas bloqueantes
| Escenario | Resultado esperado |
|---|---|
| Dos publicaciones simultáneas de la misma identidad | Solo una confirmada; la otra recibe `409`; sin sobrescritura. |
| Kill del proceso durante la subida | No aparece una versión parcialmente disponible. |
| Kill entre persistir blob y confirmar BD | Blob huérfano detectado y limpiado tras el periodo de gracia. |
| Disco lleno durante la subida | Error controlado; ningún metadato apunta a un blob inexistente. |
| Reinicio tras publicar | Bytes descargados idénticos (hash) a los publicados. |
| Mismo paquete en dos feeds | Dos identidades independientes; blob puede deduplicarse. |

### Gate de salida
Todas las pruebas bloqueantes automatizadas en `tests/integration` o `tests/recovery` y verdes en CI.

---

## Fase 3 — Superficie NuGet V3 completa

**Estatus:** ⚪ `NO INICIADA`
**Depende de:** Fase 2
**ADRs:** [ADR-009](adr-mvp.md#adr-009), [ADR-013](adr-mvp.md#adr-013), [ADR-019](adr-mvp.md#adr-019)

### Objetivo
Implementar todos los recursos anunciados con sus requisitos reales y publicar una matriz de clientes comprobados.

### Entregables
- [ ] `RegistrationsBaseUrl/3.6.0`: índice, páginas y hojas; grupos de dependencias por framework; rangos de versión; `listed`.
- [ ] `SearchQueryService`: `q`, `skip`, `take`, `prerelease`, `semVerLevel`, paginación estable.
- [ ] `SearchAutocompleteService`: ids y versiones de un id.
- [ ] `DELETE` (unlist) y relist vía `PackagePublish`.
- [ ] Versiones no listadas: fuera de búsqueda, presentes en flat container y descargables.
- [ ] Soporte de SemVer 2.0.0 (`semVerLevel=2.0.0`) correctamente filtrado.
- [ ] Generación de URLs absolutas a partir de `public_url` (no de la cabecera `Host`).
- [ ] Corpus de paquetes de prueba creado con herramientas oficiales: multi-target, dependencias, prerelease, SemVer2, metadatos extensos, iconos/readme embebidos.
- [ ] Matriz de clientes comprobados en `docs/compatibility.md`.

### Matriz de clientes objetivo (mínimo)
| Cliente | Plataforma | Operaciones |
|---|---|---|
| `dotnet` CLI (SDK LTS actual y anterior) | Linux, macOS, Windows | push, restore, search, list |
| `nuget.exe` | Windows | push, restore, search |
| Visual Studio / Rider | Windows / macOS | navegación y restore (manual, documentado) |
| CI (GitHub Actions / Azure Pipelines) | Linux | restore y push con credenciales inyectadas |

### Pruebas bloqueantes
| Escenario | Resultado esperado |
|---|---|
| Versión no listada | No aparece en búsqueda; sigue restaurable por versión exacta. |
| Búsqueda paginada con prerelease y SemVer2 | Resultados coherentes con el protocolo. |
| Dependencia transitiva con rango | `dotnet restore` resuelve la versión correcta. |
| Detrás de reverse proxy con TLS | URLs del service index correctas. |

### Gate de salida
Matriz de clientes con todas las celdas automatizables en verde en CI y las manuales documentadas con fecha y versión.

---

## Fase 4 — Identidad, autenticación y autorización

**Estatus:** ⚪ `NO INICIADA`
**Depende de:** Fase 2 (paralelizable con Fase 3)
**ADRs:** [ADR-010](adr-mvp.md#adr-010), [ADR-011](adr-mvp.md#adr-011), [ADR-012](adr-mvp.md#adr-012)

### Objetivo
Autenticación obligatoria y permisos por feed aplicados en **todos** los endpoints, incluidos metadatos, `HEAD` y respuestas condicionales.

### Entregables
- [ ] Entidades `principal` (usuario / servicio), `token`, `grant`.
- [ ] Tokens opacos de alta entropía con prefijo identificable, almacenados como verificador (hash), mostrados una sola vez.
- [ ] Expiración, revocación y `last_used_at` (actualizado sin escribir en cada request).
- [ ] Roles por feed: Reader, Publisher, Maintainer, Administrator.
- [ ] Restricción de publicación por prefijo de id (`Hemia.Payments.*`).
- [ ] Tres contextos de autenticación:
  - [ ] API admin: `Authorization: Bearer`.
  - [ ] Lectura NuGet: Basic sobre HTTPS con token como contraseña; `401` con `WWW-Authenticate`.
  - [ ] Publicación NuGet: `X-NuGet-ApiKey`.
- [ ] Middleware de autorización único, con test que recorre **todas** las rutas registradas.
- [ ] `onepackd init`: genera credencial administrativa inicial en archivo con permisos `0600`; sin contraseña por defecto.
- [ ] Respuesta `404` (no `403`) para feeds sin permiso de lectura, para no revelar existencia.
- [ ] Auditoría de: login fallido, creación/revocación de tokens, cambios de grants, publicaciones.
- [ ] Redacción de secretos en logs (cabeceras `Authorization`, `X-NuGet-ApiKey`).

### Pruebas bloqueantes
| Escenario | Resultado esperado |
|---|---|
| Token revocado | La siguiente solicitud recibe `401`. |
| Token expirado | `401`; no se renueva implícitamente. |
| Lectura entre feeds sin permiso | Ninguna filtración vía búsqueda, autocompletado, registros, flat container, `.nuspec` o `HEAD`. |
| Publisher fuera de su prefijo | `403` con código `AUTH_PREFIX_DENIED`. |
| Reader intenta publicar o hacer unlist | `403`. |
| Logs tras flujo completo | No contienen ningún token. |
| `dotnet restore` con credenciales por `NuGetPackageSourceCredentials_*` | Éxito. |

### Gate de salida
Test de cobertura de rutas: el 100 % de rutas registradas exige autenticación salvo la lista explícita de rutas públicas (vacía por defecto con `anonymous_read=false`).

---

## Fase 5 — Endurecimiento frente a paquetes y abuso

**Estatus:** ⚪ `NO INICIADA`
**Depende de:** Fases 3 y 4
**ADRs:** [ADR-013](adr-mvp.md#adr-013), [ADR-014](adr-mvp.md#adr-014), [ADR-018](adr-mvp.md#adr-018)

### Objetivo
Tratar cada `.nupkg` como contenido no confiable y proteger el servicio frente a saturación.

### Entregables
- [ ] Límites configurables: tamaño comprimido, número de entradas ZIP, tamaño descomprimido por entrada y total, tamaño del `.nuspec`, profundidad XML, tiempo de inspección.
- [ ] Parser XML sin DTD ni entidades externas.
- [ ] Rechazo de rutas peligrosas (`..`, absolutas, nombres reservados) sin extraer a disco.
- [ ] Inspección con concurrencia limitada (semáforo) separada del pool que atiende descargas.
- [ ] Rate limiting por principal e IP; límite de subidas concurrentes (`max_concurrent_uploads`).
- [ ] Cuotas por feed (almacenamiento total, número de versiones).
- [ ] Estado `availability: available | blocked` independiente de `listed`; descargas bloqueadas devuelven error explícito.
- [ ] Auditoría de bloqueo/desbloqueo con motivo.
- [ ] Documentación: el registro no analiza malware ni valida confianza de firmas; un bloqueo no borra copias ya descargadas.

### Pruebas bloqueantes
| Escenario | Resultado esperado |
|---|---|
| ZIP bomb | Rechazo controlado dentro del presupuesto de memoria/tiempo. |
| XML bomb (billion laughs) | Rechazo controlado. |
| Entrada con path traversal | Rechazo; nada escrito fuera de staging. |
| Ráfaga de subidas | Degradación controlada (`429`/`503`), sin crecimiento ilimitado de tareas. |
| Versión bloqueada | `restore` falla con mensaje claro; sigue visible para administradores. |
| Cuota excedida | `403`/`413` con código específico; nada persistido. |

### Gate de salida
Suite `tests/security` verde en CI y revisión manual de seguridad documentada.

---

## Fase 6 — API administrativa y CLI `onepack`

**Estatus:** ⚪ `NO INICIADA`
**Depende de:** Fase 4 (paralelizable con Fase 5)
**ADRs:** [ADR-015](adr-mvp.md#adr-015), [ADR-016](adr-mvp.md#adr-016)

### Objetivo
Que el registro se opere por completo desde terminal, tanto por personas como por pipelines.

### Entregables — API `/api/v1`
- [ ] Endpoints de feeds, principals, tokens, grants, paquetes (listar, inspeccionar, unlist/relist, block/unblock), auditoría.
- [ ] Endpoint de capacidades y versión para negociación con el CLI.
- [ ] Errores con código estable, mensaje, acción sugerida y `request_id`.
- [ ] Paginación por cursor.
- [ ] Contrato definido en `api-client` y compartido por servidor y CLI.

### Entregables — CLI
- [ ] `onepack context add|list|use|remove`.
- [ ] `onepack login|logout` con entrada sin eco o `--token-stdin`; almacenamiento en keychain del SO; error explícito si no hay keychain (sin fallback silencioso a texto plano).
- [ ] `onepack feed create|list|show|configure`.
- [ ] `onepack principal create|list|disable`, `onepack token create|list|revoke`, `onepack grant add|remove|list`.
- [ ] `onepack package list|inspect|push|unlist|relist|block|unblock`, con `--skip-existing-identical`.
- [ ] `onepack audit list`.
- [ ] `onepack nuget init --dry-run`: genera/actualiza `NuGet.Config` con `packageSourceMapping`, preserva lo existente, muestra diff, no escribe secretos.
- [ ] `onepack exec --feed X -- <comando>`: inyecta `NuGetPackageSourceCredentials_*` solo en el entorno del proceso hijo.
- [ ] `onepack doctor`: DNS, TLS, autenticación, permisos, service index, `NuGet.Config`, source mapping.
- [ ] Transversal: `--json` estable, códigos de salida documentados, `--no-input`, timeouts, datos a stdout y diagnóstico a stderr, confirmación en operaciones destructivas, autocompletado de shell.
- [ ] Reintentos solo en operaciones idempotentes.

### Pruebas bloqueantes
| Escenario | Resultado esperado |
|---|---|
| Flujo completo sin UI: feed → principal CI → token → grant → push → restore | Éxito usando solo `onepack` y `dotnet`. |
| `onepack exec -- dotnet restore` | Restaura; el token no aparece en archivos ni en el entorno del shell padre. |
| `nuget init` sobre `NuGet.Config` existente | Conserva fuentes previas; diff mostrado antes de aplicar. |
| `doctor` con token sin permiso de publicación | Reporta `AUTH_SCOPE_MISSING` con acción requerida, sin mostrar el secreto. |
| CLI versión N contra servidor N-1 | Funciona o falla con mensaje de capacidad ausente. |
| Salida `--json` | Validada contra snapshot de esquema. |

### Gate de salida
Guía "de cero a restore en CI" en `docs/` ejecutada literalmente por alguien que no escribió el código.

---

## Fase 7 — Operación, recuperación y distribución

**Estatus:** ⚪ `NO INICIADA`
**Depende de:** Fases 5 y 6
**ADRs:** [ADR-017](adr-mvp.md#adr-017), [ADR-018](adr-mvp.md#adr-018), [ADR-020](adr-mvp.md#adr-020)

### Objetivo
Que el servicio se pueda instalar, actualizar, observar y recuperar de forma predecible.

### Entregables — Recuperación
- [ ] Modo mantenimiento: pausa mutaciones y limpieza de blobs.
- [ ] `onepackd backup`: SQLite Online Backup API + blobs referenciados + configuración + manifiesto con hashes.
- [ ] `onepackd restore` sobre directorio vacío, con verificación del manifiesto.
- [ ] `onepackd check`: integridad BD ↔ blobs (faltantes, huérfanos, hashes incorrectos).
- [ ] Migraciones: versión de esquema, comprobación previa, backup automático antes de migrar, forward-only.

### Entregables — Observabilidad
- [ ] Logs estructurados JSON con `request_id`.
- [ ] Endpoint de métricas (requests, latencias, subidas, rechazos, espacio en disco).
- [ ] `/healthz` (vivo) y `/readyz` (BD y almacenamiento accesibles).

### Entregables — Distribución
- [ ] Binarios servidor: Linux x86-64 y ARM64.
- [ ] Binarios CLI: Linux, macOS (x86-64/ARM64), Windows.
- [ ] Unidad systemd con hardening (`ProtectSystem`, `NoNewPrivileges`, usuario dedicado).
- [ ] Imagen de contenedor opcional (no root, volumen de datos).
- [ ] Ejemplos de reverse proxy (Nginx/Caddy) que **no** sirven blobs directamente.
- [ ] Checksums y firma de los artefactos de release.

### Entregables — Rendimiento
- [ ] Benchmark reproducible en 2 vCPU / 1 GB / SSD.
- [ ] Medición de objetivos: arranque < 2 s, RSS en reposo < 100 MiB, p95 metadatos < 50 ms, memoria acotada en transferencias.
- [ ] Reporte con resultados reales (los objetivos no cumplidos se documentan, no se ocultan).

### Pruebas bloqueantes
| Escenario | Resultado esperado |
|---|---|
| Backup → restore en otro servidor → `dotnet restore` | El proyecto restaura sus dependencias. |
| Blob eliminado manualmente | `onepackd check` lo detecta y lo reporta. |
| Actualización N-1 → N con migración | Datos intactos; backup previo generado. |
| Reinicio por systemd tras crash | Servicio vuelve a `ready` sin intervención. |

### Gate de salida
Prueba de recuperación ejecutada en infraestructura distinta de la original y documentada en `docs/runbooks/`.

---

## Fase 8 — Piloto y cierre del MVP

**Estatus:** ⚪ `NO INICIADA`
**Depende de:** Fase 7

### Objetivo
Validar con un equipo real que el producto resuelve el problema y ejecutar la prueba decisiva.

### Entregables
- [ ] Equipo piloto identificado (p. ej., un equipo de Hemia) con al menos una librería y un pipeline de CI.
- [ ] Instalación hecha por el equipo piloto siguiendo solo la documentación.
- [ ] Uso en builds y publicaciones habituales durante un periodo acordado (p. ej. 2–4 semanas).
- [ ] Ejercicio de incidente: revocar credencial de CI, bloquear una versión, restaurar desde backup.
- [ ] Registro de fricciones, errores y peticiones; clasificación en "bloqueante MVP" / "post-MVP".
- [ ] Corrección de todos los bloqueantes MVP.
- [ ] Retrospectiva y actualización del orden de evolución (credential provider, S3, proxy…).

### Criterios de cierre del MVP
- [ ] La prueba decisiva se ejecuta completa sin UI ni edición manual de BD.
- [ ] Cero incidencias abiertas de pérdida o corrupción de datos.
- [ ] Cero filtraciones entre feeds conocidas.
- [ ] Matriz de clientes y documentación de operación actualizadas.
- [ ] Release `v0.1.0` etiquetada con binarios firmados.

---

## Fuera del MVP (referencia)

Registrado para evitar que entre por la puerta de atrás. Cualquier inclusión requiere un ADR nuevo.

| Funcionalidad | Etapa prevista |
|---|---|
| Credential provider NuGet, importación de paquetes, almacenamiento S3 | Primera evolución |
| Proxy/caché de nuget.org con reglas de procedencia | Segunda evolución |
| npm | Tercera evolución |
| Símbolos, PyPI, SSO, interfaz web | Según demanda |
| PostgreSQL, multinodo, alta disponibilidad | Solo con necesidad demostrada |

---

## Registro de cambios de estatus

| Fecha | Fase | De | A | Nota / evidencia |
|---|---|---|---|---|
| 2026-10-03 | 0 | `NO INICIADA` | `EN DISEÑO` | Roadmap y ADRs iniciales redactados. |
| 2026-10-03 | 0 | `EN DISEÑO` | `EN PROGRESO` | ADR-001, 003 y 019 aceptados; binarios `onepackd`/`onepack`. |
| 2026-10-03 | 0 | `EN PROGRESO` | `EN VALIDACIÓN` | Entregables implementados; checks verdes en local. Pendiente: CI en GitHub. |
| 2026-10-04 | 0 | `EN VALIDACIÓN` | `COMPLETADA` | Gate superado: [CI run 37174401011](https://github.com/Khr0x/onepack/actions/runs/37174401011). Runner fijado en `ubuntu-24.04` (con `ubuntu-latest` los jobs no recibían runner). |
| 2026-10-04 | 1 | `NO INICIADA` | `EN DISEÑO` | Dependencia (Fase 0) completada. |
| 2026-10-04 | 1 | `EN DISEÑO` | `EN PROGRESO` | ADR-008 y ADR-009 aceptados. |
| 2026-10-04 | 1 | `EN PROGRESO` | `EN VALIDACIÓN` | Corpus sin divergencias y E2E verde en local. Pendiente: CI en GitHub. |
