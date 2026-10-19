# Especificación de conformidad npm — hosting privado v2 (OP-023)

> Tarea OP-023. Depende de OP-020 y OP-021 (verificadas). Entrega los contratos que OP-024
> (adaptador), OP-025 (CLI) y la documentación de cliente necesitan; no implementa nada.
>
> Fuentes: `roadmap/adr-v2.md` (ADR-021…026), `docs/security.md`, `docs/compatibility.md`,
> `docs/api.md` y el código de `crates/server/src` y `crates/core/src`.
>
> Baseline consultado: `88467a94eaee4fc7345bac54fdf6fa4449a913b8`.
> Fecha: 2026-10-19. Estado: propuesto para revisión; se acepta al cerrar OP-023.

## 1. Alcance

Habilitar **publish, install y ci** con npm real contra un feed privado `npm`, con paquetes
scoped y no scoped, dist-tags, autenticación por token, límites de inspección y errores
estables, reutilizando los roles, cuotas, auditoría y almacenamiento existentes.

**Fuera de alcance:** proxy o espejo de npmjs; pnpm, yarn y cualquier cliente no listado en la
matriz (no se anuncian); `npm login`; `npm unpublish`; `npm deprecate`; `npm search`; ejecutar
Node.js en el servidor; plugins dinámicos. Todo esto queda documentado como limitación con su
condición de revisión en la sección 10.

## 2. Matriz de conformidad inicial (AC-01)

Clientes candidatos; el patch exacto se fija al preparar OP-024 (AC-01) y una fila solo se
publica como verificada si pasa el E2E de la sección 11.

| Cliente | Node | Rol en la matriz |
|---|---|---|
| npm 10.x | Node 22 LTS | compatibilidad con la LTS anterior |
| npm 11.x | Node 24 LTS | línea principal |

Al preparar OP-024 se fijan las versiones patch y se añade la LTS vigente si ya existe; no se
anuncia un cliente npm que no haya pasado el E2E en CI. pnpm y yarn quedan explícitamente no
verificados (REQ-03) y `capabilities` solo anuncia el formato `npm`, no clientes.

| # | Flujo | Ejemplo | Qué comprueba |
|---|---|---|---|
| F1 | Publish no scoped | `npm publish` en `widgets` | Aceptación, transacción, `latest` |
| F2 | Publish scoped | `npm publish` en `@acme/widgets` | Namespace, prefijo de grant, tarball scoped |
| F3 | Install nuevo | `npm install widgets --registry ...` | Packument abreviado + tarball |
| F4 | CI con lockfile | `npm ci` con `package-lock.json` | `resolved` + `integrity` (sha512) |
| F5 | Consulta | `npm view @acme/widgets`, `... versions`, `... dist-tags` | Packument completo y rutas de versión |
| F6 | Dist-tags | `npm dist-tag add @acme/widgets@1.2.3 next`, `npm dist-tag rm ...` | Alta, movimiento y borrado |
| F7 | Auth adversa | sin token, token revocado, reader publicando, prefijo ajeno | 401/403/404 según la sección 4 |
| F8 | Aislamiento | token con grant solo en el feed A contra el feed B | 404 sin revelar |
| F9 | Versión bloqueada | `npm install` de una versión bloqueada | Tarball `410 PACKAGE_BLOCKED` |
| F10 | Integridad | `npm ci` con integridad correcta; lockfile alterado | Falla local del cliente, no del registro |
| F11 | Dependencia transitiva | `app` declara `dependencies: {"lib": "^1.0.0"}`; `npm install app` | Resolución de `lib` desde el mismo feed |

## 3. Superficie HTTP

Base: `{public_url}/npm/{feed}` (ADR-018). El formato del feed debe ser `npm`; si es de otro
formato, la ruta responde `404` como `FEED_NOT_FOUND` (ADR-024).

| Método | Ruta | Operación |
|---|---|---|
| GET | `/npm/{feed}/{name}` | Packument completo o abreviado según `Accept` |
| GET | `/npm/{feed}/{name}/{version}` | Manifiesto de una versión concreta |
| GET, HEAD | `/npm/{feed}/{name}/-/{file}.tgz` | Tarball |
| PUT | `/npm/{feed}/{name}` | Publicación |
| GET | `/npm/{feed}/-/package/{name}/dist-tags` | Listar dist-tags |
| PUT | `/npm/{feed}/-/package/{name}/dist-tags/{tag}` | Crear o mover un tag |
| DELETE | `/npm/{feed}/-/package/{name}/dist-tags/{tag}` | Borrar un tag |
| GET | `/npm/{feed}/-/whoami` | Identidad del token |
| GET | `/npm/{feed}/-/ping` | Sonda |
| — | `/npm/{feed}/-/v1/search` | No registrada: `npm search` recibe `404` |
| — | Login | No implementado: la credencial se configura en `.npmrc` |

Detalles de ruta:

- `{name}` admite `@scope/name` y `name`. Se aceptan las formas percent-encoded `%40` y
  `%2F`; se decodifica una sola vez y se rechaza el doble encoding, segmentos vacíos, `..`,
  bytes nulos y caracteres de control.
- El parser es un catch-all por método; las rutas estáticas `/-/...` tienen prioridad. La
  implementación concreta (patrón Axum y extractores) se fija en OP-024 con pruebas de ambas
  formas de scoped.
- Sin compresión de respuesta en el primer corte (`identity`), igual que las rutas NuGet.

## 4. Autenticación y autorización (AC-02)

La superficie npm acepta **solo** `Authorization: Bearer <token>` (mecanismo de npm); no
acepta `X-NuGet-ApiKey` ni `Basic`. La superficie NuGet y la administrativa no cambian. Esto
implica añadir `Surface::Npm` a la clasificación de rutas, a `credential()` y al renderizado
de errores, y la etiqueta `npm` en métricas (ADR-024).

Todas las peticiones siguen autenticadas; no hay lectura anónima en este corte. La
credencial se configura así:

```ini
; .npmrc
@acme:registry=https://packages.example.com/npm/internal
//packages.example.com/npm/internal/:_authToken=${ONEPACK_TOKEN}
```

```bash
onepack token create --principal ci-web --name npm-publish
```

| Caso | Resultado |
|---|---|
| Token ausente, expirado, revocado o mal formado | `401 AUTH_REQUIRED` + `WWW-Authenticate: Bearer realm="onepack"` |
| Token válido sin grant en el feed | `404 FEED_NOT_FOUND` (no revela que el feed o el paquete existen; ADR-012) |
| `reader` intenta publicar o mover tags | `403 AUTH_SCOPE_MISSING` |
| `publisher` publica un id fuera de sus prefijos | `403 AUTH_PREFIX_DENIED` |
| `reader`/`publisher`/`maintainer` leen | Permitido según rol acumulativo |
| `maintainer` o admin | Publicación, tags y bloqueo vía API administrativa |

Reglas adicionales:

- Lectura: `Access::Read` del feed. Packument, manifiesto y tarball comparten decisión.
- Publicación: `Access::Publish` + patrón del grant sobre el `package_key` (ADR-022);
  `@acme/*` cubre `@acme/widgets`.
- Dist-tags: `Access::Publish` + patrón del paquete; mover o crear un tag es una operación de
  publicación, borrarlo también. `latest` no se puede borrar.
- `/-/whoami` y `/-/ping` requieren token válido, sin exigir grant: no exponen datos del feed.
- La revocación surte efecto en la siguiente petición; npm puede reutilizar credenciales en
  caché, así que la documentación recordará refrescar `.npmrc` tras revocar.
- TLS: npm rechaza enviar credenciales por HTTP salvo localhost. El despliegue exige HTTPS en
  el proxy inverso y `public_url` HTTPS; el servidor no confía en `X-Forwarded-For` (ADR-018).

## 5. Identidad, versiones y conflictos

Reglas del adaptador (ADR-022); la implementación reproducirá las reglas de
`validate-npm-package-name` y SemVer con un corpus de conformidad.

| Regla | Detalle |
|---|---|
| Nombre no scoped | Minúsculas, ≤ 214 caracteres, sin espacios ni caracteres no seguros, sin empezar por `.` ni `_` |
| Nombre scoped | `@scope/name`; scope y name con las mismas reglas |
| `package_key` | Nombre completo en minúsculas, incluido `@scope/` |
| Versión | SemVer 2.0.0 estricta; `pre` y `build` según SemVer |
| Identidad de versión (`version_key`) | Sin build metadata; `full_version` conserva el valor publicado |
| Dist-tags | No son versiones y no crean identidad; un tag no puede ser SemVer ni un rango válido |
| Orden | Precedencia SemVer, en el adaptador |

| Publicación | Clave | Resultado |
|---|---|---|
| `@acme/widgets` `1.2.3-beta.1+build.5` | `@acme/widgets` / `1.2.3-beta.1` | Aceptada; `full_version` conserva `+build.5` |
| `@acme/widgets` `1.2.3` tras la anterior | `1.2.3` | Distinta identidad: aceptada |
| `Widgets` | — | Rechazada: npm exige minúsculas |
| `@acme/widgets` `1.2.3` repetida, mismos bytes | `1.2.3` | `409 VERSION_EXISTS` |
| `@acme/widgets` `1.2.3` repetida, otros bytes | `1.2.3` | `409 VERSION_EXISTS` |
| `@acme/widgets` `latest` como versión | — | Rechazada: no es SemVer válida |

Un reintento de `npm publish` tras un fallo de red devuelve `409`; npm no tiene publicación
idempotente y la spec lo asume explícitamente (ADR-007).

## 6. Publicación

El cliente envía `PUT /npm/{feed}/{name}` con `Content-Type: application/json`. El cuerpo es
el documento estilo CouchDB de npm:

- `versions` con **exactamente una** versión nueva; cualquier otro caso es
  `400 PUBLISH_BODY_INVALID`.
- `_attachments` con **exactamente** un adjunto. La clave puede ser
  `{name}-{version}.tgz` (con scope) o `{unscoped}-{version}.tgz`; el registro canoniza el
  nombre servido como `{unscoped}-{version}.tgz`, que es el que emite en `dist.tarball`.
  `content_type: application/octet-stream` y `data` en base64.
- `name` del documento y `versions[version].name` deben coincidir con la URL; `version` debe
  ser SemVer y coincidir con el nombre del adjunto.
- El `package.json` dentro del tarball es la **fuente autoritativa** del manifiesto; el
  manifiesto del cuerpo solo se valida y no se persiste. `dist` lo genera el registro.

Procesamiento acotado (no se materializa el JSON completo en memoria):

1. El cuerpo va a `staging/` con el límite
   `max_package_bytes × 4/3 + 1 MiB` (base64 + metadatos).
2. Se recorre el JSON en streaming; `_attachments[*].data` se decodifica con un lector base64
   hacia un archivo de staging, aplicando el límite comprimido mientras se decodifica.
3. El resto de campos se recogen con topes de tamaño; los campos desconocidos se ignoran.

Persistencia, reutilizando el flujo actual de subida → inspección → blob → transacción:

- Blob por SHA-256 y fila en `version_file` con `kind = tarball` y el nombre del adjunto
  (ADR-023).
- `package_version.metadata` guarda un JSON del adaptador con el manifiesto normalizado, la
  integridad y el nombre del tarball; `search_text` sale de descripción y `keywords`.
- `listed = true`, `is_semver2 = false` (señal NuGet; ADR-022) e `is_prerelease` según SemVer.
- Dist-tags en una tabla nueva `package_tag` (PK `feed_id + package_key + tag`, FK al
  paquete), en la **misma transacción** que la versión: un fallo no deja un tag apuntando a
  una versión inexistente. La migración es la siguiente disponible después de la de OP-022
  (p. ej. `0007`).
- Integridad calculada en la inspección: `sha512` SRI (`sha512-<base64>`) y `sha1` hexadecimal
  heredado, ambos persistidos para el packument.
- Cuota: se aplica la suma de tamaños de la versión como hoy (ADR-014).
- Auditoría `package.publish` con actor y token, también en rechazos.

Dist-tags del cuerpo: se aplican los tags enviados; si el paquete no tenía ningún tag y el
cuerpo no trae, se asigna `latest` a la primera versión. No se inventa `latest` cuando el
cuerpo declara otros tags (`npm publish --tag next` no mueve `latest`). Respuesta `201` con
`{"ok": true}`. `PUT`/`DELETE` de tags:

- `PUT .../dist-tags/{tag}` con un string JSON de versión; crea o mueve; `201`.
- `DELETE .../dist-tags/{tag}`; `latest` → `400 DIST_TAG_PROTECTED`; tag inexistente → `404`;
  versión inexistente → `404 VERSION_NOT_FOUND`; tag inválido → `400 DIST_TAG_INVALID`.

## 7. Lectura

Packument completo (`GET .../{name}`, `Accept: application/json`):

| Campo | Contenido |
|---|---|
| `_id`, `name` | Nombre tal como figura en el feed |
| `dist-tags` | Tags actuales |
| `versions` | Manifiesto por versión con `dist.tarball`, `dist.integrity`, `dist.shasum` |
| `time` | `created`, `modified` y una entrada por versión (RFC 3339 UTC con milisegundos, como el resto del registro) |
| `description`, `keywords`, `license`, `repository` | Del último manifiesto publicado, si existen |
| `maintainers` | Vacío u omitido; el registro no inventa identidades npm |

Packument abreviado (`Accept: application/vnd.npm.install-v1+json`): `name`, `dist-tags` y
`versions` con los campos que npm necesita para instalar (`name`, `version`,
`dependencies`, `optionalDependencies`, `peerDependencies`, `peerDependenciesMeta`,
`bundleDependencies`, `bin`, `engines`, `os`, `cpu`, `dist`, `deprecated`). El resto se omite.
`npm ci` obtiene `dist.integrity` de aquí.

- `dist.tarball` es absoluto y estable: `{public_url}/npm/{feed}/{@scope/name}/-/{file}.tgz`;
  un cambio de `public_url` invalida lockfiles, y la documentación lo advertirá.
- Versión bloqueada (ADR-013): permanece en el packument completo y abreviado; el tarball
  responde `410 PACKAGE_BLOCKED`. No se usa el campo `deprecated` para bloquear en el primer
  corte.
- `listed` no aplica a npm: las versiones se listan siempre; la retirada no existe (sección
  10).
- Tarball: `application/octet-stream`, soporte de `HEAD`, sin rangos. Sin compresión.
- `/-/whoami` → `{"username": "<principal.name>"}`; `/-/ping` → `{}`.

## 8. Validación de tar/gzip (AC-03)

La inspección corre como la de NuGet: en el pool bloqueante, con semáforo propio y plazo
(ADR-014).

| Límite | Por defecto |
|---|---|
| Tamaño comprimido | `max_package_bytes` compartido (100 MiB) |
| Tamaño descomprimido total | 512 MiB |
| Entradas tar | 50 000 |
| `package.json` | 1 MiB |
| Profundidad del JSON de `package.json` | 64 |
| Tiempo de inspección | 30 s |
| Inspecciones simultáneas | las de NuGet |

Reglas (espejo de las NuGet, adaptadas a tar):

- gzip de un solo miembro; se rechaza el gzip inválido o truncado. Los topes se aplican
  mientras se descomprime, no sobre tamaños declarados.
- Solo entradas regulares y directorios. Se rechazan symlinks, hardlinks, dispositivos, FIFOs,
  entradas sparse, cifradas o de un tipo que la librería no interprete; las extensiones
  normales de tar (pax, GNU long name) se procesan como metadatos, no como entradas.
- Cada ruta debe ser relativa y empezar por `package/`; se rechazan rutas absolutas, `..`,
  caracteres de control o `:`, nombres reservados de Windows, terminaciones en punto o
  espacio, entradas duplicadas (sin distinguir mayúsculas) y cualquier ruta fuera de
  `package/`.
- Se exige `package/package.json`; un JSON válido con `name` y `version` coincidentes con la
  URL y el adjunto, nombre y versión válidos según la sección 5.
- Los `scripts` no se ejecutan ni se materializan; `bin` se sirve como metadato.
- Un rechazo no persiste nada y se audita con su código.

Fixtures adversos previstos (OP-024): traversal (`package/../..`), ruta absoluta, symlink,
hardlink, entrada duplicada con distinta capitalización, gzip bomb, 50 001 entradas,
`package.json` ausente, sobredimensionado o con JSON inválido, nombre/versión que no coinciden
con la URL, base64 inválido, `length` del adjunto que miente, tarball mayor que el límite y
cuerpo con dos versiones o dos adjuntos.

## 9. Errores

Formato de error de la superficie npm: JSON `{"error": "<CODE>: <mensaje>", "reason":
"<mensaje>", "code": "<CODE>"}`; el prefijo estable es el mismo `CODE` que usan las demás
superficies, y `reason` queda para lectura humana. npm muestra el texto y su propio código
`E<status>`.

| Código | HTTP | Cuándo |
|---|---|---|
| `AUTH_REQUIRED` | 401 | Token ausente, inválido, expirado o revocado |
| `FEED_NOT_FOUND` | 404 | Feed inexistente, sin grant o de otro formato |
| `AUTH_SCOPE_MISSING` | 403 | Rol insuficiente para la operación |
| `AUTH_PREFIX_DENIED` | 403 | Prefijo de publicación no autorizado |
| `PACKAGE_NOT_FOUND` | 404 | Paquete o versión inexistente con acceso al feed |
| `PUBLISH_BODY_INVALID` | 400 | Cuerpo que no cumple la sección 6 |
| `VERSION_EXISTS` | 409 | Identidad de versión ya publicada |
| `TARBALL_INVALID` | 400 | gzip/tar inválido o estructura inesperada |
| `PACKAGE_UNSAFE_PATH` | 400 | Ruta o entrada peligrosa o duplicada |
| `PACKAGE_LIMIT_EXCEEDED` | 413 | Se supera un límite de inspección o el plazo |
| `PACKAGE_TOO_LARGE` | 413 | Se supera el tamaño comprimido |
| `DIST_TAG_INVALID` | 400 | Nombre de tag inválido |
| `DIST_TAG_PROTECTED` | 400 | Intento de borrar `latest` |
| `VERSION_NOT_FOUND` | 404 | Tag hacia versión inexistente |
| `FEED_QUOTA_VERSIONS` | 403 | Cuota de versiones superada |
| `FEED_QUOTA_STORAGE` | 413 | Cuota de almacenamiento superada |
| `PACKAGE_BLOCKED` | 410 | Tarball de una versión bloqueada |
| `UPLOADS_BUSY` | 503 | Sin plazas de subida |
| `UPLOAD_TIMEOUT` | 408 | Subida agotó el plazo |
| `RATE_LIMITED` | 429 | Límite por principal o IP |
| `INTERNAL` | 500 | Error no clasificado |

## 10. Retirada y búsqueda (decisión pendiente de OP-023)

Decisión para el primer corte:

- **Sin `npm unpublish`.** El registro nunca borra bytes (ADR-005); la ruta de publicación
  rechaza cualquier intento de modificar o quitar versiones existentes con `409` o
  `PUBLISH_BODY_INVALID`. La retirada real se hará con el bloqueo administrativo existente.
- **Sin `npm deprecate`.** npm implementa `deprecate` con un `PUT` que modifica el packument;
  el contrato aditivo de publicación lo rechaza. Queda como evolución con su propia migración
  de metadatos si se prioriza.
- **Sin `npm search`.** `/-/v1/search` no se registra; `npm search` recibe `404`. La búsqueda
  del registro sigue siendo la administrativa y la de NuGet.
- **Bloqueo como retirada:** `410` en el tarball, versión visible en el packument; bloquea y
  desbloquea Maintainer o admin con motivo y auditoría, igual que NuGet.

Condición de revisión: si el piloto o los usuarios piden retirada o búsqueda npm, se abre una
tarea de evolución antes de ampliar este alcance.

## 11. Pruebas y handoff

- **Corpus de identidad:** nombres válidos e inválidos (scoped, mayúsculas, bordes de 214
  caracteres) y versiones SemVer, incluido build metadata; sin equivalentes NuGet.
- **Unitarias del adaptador:** parseo del body, extracción de `package.json`, integridad,
  dist-tags, render del packument completo y abreviado.
- **Integración de servidor:** auth por superficie, 401/403/404, `VERSION_EXISTS`,
  `DIST_TAG_*`, cuotas y auditoría, transacción de versión + tag.
- **Seguridad:** los fixtures adversos de la sección 8, sin persistencia.
- **E2E real:** guion `scripts/e2e-npm.sh` (patrón de `scripts/e2e-dotnet.sh`) y job de CI con
  la matriz de la sección 2: F1–F11, incluidas revocación, prefijo ajeno, aislamiento entre
  feeds y bloqueo.
- **Docs a actualizar al implementar:** `docs/security.md` (límites, tabla de errores y
  fixtures npm), `docs/compatibility.md` (matriz npm verificada), `docs/cli.md`
  (`.npmrc` y `onepack token create`). La spec no las modifica todavía.
- **Handoff:** OP-024 crea `onepack-npm` (ADR-026), registra las rutas aprobadas aquí y añade
  la migración de `package_tag`; OP-025 decide los comandos de CLI. Ninguna dependencia de
  npm entra en `core` ni en `storage`.

## 12. Verificación de esta especificación

- AC-01: matriz de la sección 2 con publish/install/ci, scoped/no scoped, dist-tags y la regla
  de fijación de versiones en OP-024.
- AC-02: contratos de auth y errores de las secciones 4 y 9, probables con token
  ausente/revocado, rol insuficiente y namespace ajeno sin revelar paquetes.
- AC-03: plan adverso de la sección 8 (traversal, links, expansión, colisiones) y metadatos
  que anuncian solo lo implementado (sección 3).
- AC-CIERRE: pendiente de la aceptación de la spec, con nota y backlog sincronizados.

Evidencia esperada al cerrar: ruta de este documento, SHA-256, commit local y revisión
registrada en Obsidian.
