# Especificación de conformidad PyPI — hosting privado v2 (OP-027)

> Tarea OP-027. Depende de OP-020 y OP-021 (verificadas). Entrega los contratos que OP-033
> (adaptador), OP-034 (CLI/guías) y OP-035 (verificación mixta) necesitan; no implementa nada.
>
> Fuentes: `roadmap/adr-v2.md` (ADR-021…026), `roadmap/npm-v2.md` (referencia de estilo),
> `docs/security.md`, `docs/compatibility.md`, `docs/api.md` y el código de
> `crates/server/src` y `crates/core/src`.
>
> Baseline consultado: `88467a94eaee4fc7345bac54fdf6fa4449a913b8`.
> Fecha: 2026-10-19. Estado: propuesto para revisión; se acepta al cerrar OP-027.

## 1. Alcance

Habilitar publicación privada con **twine** y consumo con **pip** contra un feed `pypi`:
índice simple (HTML PEP 503 y JSON PEP 691), upload de wheels y sdists, múltiples archivos
por versión, integridad por SHA-256, autenticación por token, inspección sin ejecutar código y
errores estables, reutilizando roles, cuotas, auditoría y almacenamiento existentes.

**Fuera de alcance:** proxy o espejo de PyPI; ejecutar `setup.py`, backends PEP 517 o hooks de
build en el servidor; API JSON de PyPI; endpoints de *yank*; *search*; uploads solo de
metadatos; `.zip` como sdist y `.egg`; poetry, uv, PDM u otros clientes no listados (no se
verifican ni se anuncian). Documentado como limitación con condición de revisión en la
sección 11.

## 2. Matriz de conformidad inicial (AC-01)

Versiones candidatas; el patch exacto y la versión de Python se fijan al preparar OP-033 y una
fila solo se publica como verificada si pasa el E2E de la sección 12.

| Herramienta | Rol en la matriz |
|---|---|
| pip 25.x | consumo con `--no-cache-dir`, índice privado |
| pip 24.x | compatibilidad con la línea anterior |
| twine 6.x | publicación de wheel y sdist |

Al preparar OP-033 se fijan los patch y se añade la versión mayor vigente si ya existe; no se
anuncia una herramienta que no haya pasado el E2E en CI. `uv`, poetry y PDM no se verifican.

| # | Flujo | Ejemplo | Qué comprueba |
|---|---|---|---|
| F1 | Upload sdist | `twine upload --repository-url ... dist/hemia_core-1.0.0.tar.gz` | Multipart, metadatos, identidad |
| F2 | Upload wheel | `twine upload ... dist/hemia_core-1.0.0-py3-none-any.whl` | ZIP, `METADATA`, tags |
| F3 | Varios archivos | sdist + wheel de la misma versión | `version_file` múltiple |
| F4 | Archivo añadido después | wheel nueva en una versión ya publicada | Alta sin duplicar versión |
| F5 | Install wheel | `pip install hemia-core --index-url ... --no-cache-dir` | Índice JSON/HTML + descarga + hash |
| F6 | Install sdist | `pip install --no-binary :all: hemia-core ...` | Build local del sdist (no en el servidor) |
| F7 | Dependencia privada | `hemia-app` con `Requires-Dist: hemia-lib` | Resolución desde el mismo índice |
| F8 | Hashes | `requirements.txt` con `--hash=sha256:...` y `--require-hashes` | Integridad verificada por pip |
| F9 | Prerelease | `pip install hemia-core --pre` | Orden PEP 440 |
| F10 | Reintento idéntico | Segundo `twine upload` del mismo archivo | `200 OK` idempotente |
| F11 | Bytes distintos | Mismo filename, contenido cambiado | `409 VERSION_FILE_CONFLICT` + auditoría |
| F12 | Auth adversa | sin token, revocado, reader publicando, nombre fuera de prefijo | 401/403/404 según la sección 4 |
| F13 | Aislamiento | token con grant solo en el feed A contra el feed B | 404 sin revelar |
| F14 | Versión bloqueada | `pip install pkg==version_bloqueada` | Archivo fuera del índice y `410` |

## 3. Superficie HTTP

Base: `{public_url}/pypi/{feed}` (ADR-018). El formato del feed debe ser `pypi`; si es de otro
formato, la ruta responde `404` como `FEED_NOT_FOUND` (ADR-024).

| Método | Ruta | Operación |
|---|---|---|
| GET | `/pypi/{feed}/simple/` | Raíz del índice: todos los proyectos (HTML o JSON) |
| GET | `/pypi/{feed}/simple/{project}/` | Página de proyecto (HTML PEP 503 o JSON PEP 691) |
| GET, HEAD | `/pypi/{feed}/packages/{filename}` | Descarga de un archivo de distribución |
| POST | `/pypi/{feed}/` y `/pypi/{feed}` | Upload de twine (multipart) |
| — | `/pypi/{feed}/pypi/{project}/json` | No implementado (API JSON) |
| — | Yank | No implementado: sin PEP 592 en el primer corte |
| — | Search | No implementado |

Detalles de ruta:

- El `{project}` de la URL se acepta en cualquier forma que normalice al mismo proyecto
  (PEP 503); la respuesta usa siempre el nombre normalizado. La barra final es la forma
  canónica y también se acepta sin ella.
- Se registran ambas variantes del POST para que twine no dependa de redirecciones.
- `{filename}` es único dentro del feed porque codifica nombre normalizado y versión; aun así
  el handler resuelve por proyecto + versión + archivo, no por texto libre.
- Sin compresión de respuesta en el primer corte (`identity`), igual que NuGet y npm.

## 4. Autenticación y autorización (AC-02)

La superficie PyPI acepta **solo** `Authorization: Basic` con el token en la contraseña; el
usuario es informativo (`__token__` por convención de las herramientas). pip y twine usan este
mecanismo; no se acepta `X-NuGet-ApiKey` ni `Bearer` en esta superficie. Esto implica añadir
`Surface::PyPi` a la clasificación de rutas, a `credential()` y al renderizado de errores, y
la etiqueta `pypi` en métricas (ADR-024).

Todas las peticiones siguen autenticadas; no hay lectura anónima. Configuración del cliente:

```ini
# ~/.config/pip/pip.conf
[global]
index-url = https://__token__:${ONEPACK_TOKEN}@packages.example.com/pypi/internal/simple/
```

```ini
# ~/.pypirc
[distutils]
index-servers = onepack

[onepack]
repository = https://packages.example.com/pypi/internal/
username = __token__
password = ${ONEPACK_TOKEN}
```

| Caso | Resultado |
|---|---|
| Token ausente, expirado, revocado o mal formado | `401 AUTH_REQUIRED` + `WWW-Authenticate: Basic realm="onepack"` |
| Token válido sin grant en el feed | `404 FEED_NOT_FOUND` (no revela feed, proyecto ni archivo; ADR-012) |
| `reader` intenta publicar | `403 AUTH_SCOPE_MISSING` |
| `publisher` publica un nombre fuera de sus prefijos | `403 AUTH_PREFIX_DENIED` |
| `reader`/`publisher`/`maintainer` leen | Permitido según rol acumulativo |
| `maintainer` o admin | Publicación y bloqueo vía API administrativa |

Reglas adicionales:

- Lectura: `Access::Read` del feed. Índice, página de proyecto y descarga comparten decisión;
  un error nunca incluye nombres de proyectos ni de archivos de otros feeds.
- Publicación: `Access::Publish` + patrón del grant sobre el `package_key` (nombre
  normalizado; ADR-022). Ejemplos: `hemia-*` cubre `hemia-core`, `hemia.logging` normaliza a
  `hemia-logging`.
- La revocación surte efecto en la siguiente petición; pip puede guardar credenciales en
  netrc/keyring, así que la documentación recordará refrescarlas tras revocar.
- TLS: pip y twine envían credenciales por HTTPS. El despliegue exige HTTPS en el proxy
  inverso y `public_url` HTTPS; el servidor no confía en `X-Forwarded-For` (ADR-018).

## 5. Identidad, versiones y conflictos

Reglas del adaptador (ADR-022); la implementación reproducirá PEP 503, PEP 508 y PEP 440 con
corpus de conformidad.

| Regla | Detalle |
|---|---|
| Normalización de nombre | PEP 503: minúsculas y runs de `-`, `_`, `.` → `-` |
| Forma válida del nombre | PEP 508: letras, dígitos y `-_.` en el interior, empieza y termina alfanumérico; ≤ 100 caracteres por política del registro |
| `package_key` | Nombre normalizado |
| Versión | PEP 440 canónica: epoch, release, pre (`a`/`b`/`rc`), post, dev y local (`+local`) |
| Identidad de versión (`version_key`) | Cadena canónica completa, incluido epoch y local; dos textos que comparan igual pero difieren (`1.0` y `1.0.0`) son identidades distintas |
| Orden | PEP 440; `is_prerelease` para pre y dev |
| Archivos | No crean identidad: se agregan a la versión; `{filename}` es su clave dentro del feed |

| Publicación | Nombre / versión | Resultado |
|---|---|---|
| `Zope.Interface` `5.0` | `zope-interface` / `5.0` | Aceptada con nombre normalizado |
| `hemia.core` `1.0rc1` | `hemia-core` / `1.0rc1` | Aceptada, prerelease |
| `hemia-core` `1.0.post1` | `hemia-core` / `1.0.post1` | Distinta de `1.0`; aceptada |
| `hemia-core` `1!2.0` | `hemia-core` / `1!2.0` | Epoch canónico; aceptada |
| `hemia-core` `1.0+ubuntu.1` | `hemia-core` / `1.0+ubuntu.1` | Distinta de `1.0`; aceptada |
| `hemia-core` `v1.0` | `hemia-core` / `1.0` | Aceptada; el prefijo `v` se normaliza |
| `hemia-core` `1.0.0` tras `1.0` | `hemia-core` / `1.0.0` | Identidad distinta; aceptada (comparan igual, no son la misma versión) |
| `Hemía` | — | Rechazada: nombre no ASCII |

## 6. Publicación (upload)

twine envía `POST` multipart a la URL del repositorio:

- `:action = file_upload` y `protocol_version = 1`; cualquier otra acción se rechaza con
  `400 PUBLISH_BODY_INVALID`.
- `name`, `version`, `filetype` (`sdist` o `bdist_wheel`) y `pyversion`, validados contra el
  archivo. Los campos de core metadata (resumen, clasificadores, `requires_dist`,
  `requires_python`, etc.) se aceptan como entrada, pero **la fuente autoritativa es la
  metadata dentro del archivo** (`METADATA` o `PKG-INFO`); los campos del formulario solo se
  validan y los desconocidos se ignoran.
- `content` con el archivo y su nombre. `sha256_digest` opcional: si viene, debe coincidir con
  el calculado; `md5_digest` y demás se ignoran.

Procesamiento acotado:

1. El multipart se recorre en streaming hacia `staging/`; el archivo `content` usa el límite
   comprimido compartido `max_package_bytes` (100 MiB) y los campos de texto un tope de 1 MiB
   en conjunto.
2. La inspección de la sección 8 valida el archivo y extrae la metadata.
3. El blob entra por SHA-256 y se persiste una fila de `version_file` con `kind = wheel` o
   `sdist` y el filename original (ADR-023). No se necesitan tablas nuevas.

Reglas de archivos:

- Una versión se crea con su primer archivo; los siguientes se agregan a la misma versión si
  su filename no existe.
- **Reintento idéntico:** mismo filename y mismo SHA-256 → `200 OK` idempotente, sin escritura
  nueva y con la misma semántica que ADR-007. Es el caso de un `twine upload` reintentado.
- **Conflicto:** mismo filename con bytes distintos → `409 VERSION_FILE_CONFLICT`, auditado
  (`package.publish`, `denied`). twine puede evitar el reintento con `--skip-existing`, que
  consulta el índice; el registro no delega la decisión en el cliente.
- La cuota y la auditoría se aplican como en NuGet/npm; el `created_at` de la versión no
  cambia al añadir archivos.
- No se ejecuta `setup.py`, backend PEP 517 ni hook alguno; el registro solo lee y valida.

## 7. Lectura

Página de proyecto en HTML (PEP 503):

```html
<a href="{public_url}/pypi/{feed}/packages/{filename}#sha256={hex}"
   data-requires-python="&gt;=3.9">{filename}</a>
```

- Un ancla por archivo, con `#sha256=` obligatorio para que pip verifique.
- `data-requires-python` se copia escapado de la metadata si existe; `data-yanked` no se usa
  en el primer corte.
- La raíz `/simple/` lista los proyectos (nombre normalizado) enlazando a su página.

Página de proyecto en JSON (PEP 691, `Accept: application/vnd.pypi.simple.v1+json`):

```json
{
  "meta": { "api-version": "1.0" },
  "name": "hemia-core",
  "files": [
    {
      "filename": "hemia_core-1.0.0-py3-none-any.whl",
      "url": "https://packages.example.com/pypi/internal/packages/hemia_core-1.0.0-py3-none-any.whl",
      "hashes": { "sha256": "…" },
      "requires-python": ">=3.9",
      "yanked": false
    }
  ]
}
```

pip negocia JSON y cae a HTML si no lo entiende; ambos se sirven desde el mismo estado.

- Descarga: `application/octet-stream`, soporte de `HEAD`, sin rangos, sin compresión.
- Versión bloqueada (ADR-013): sus archivos **se omiten del índice** y la descarga directa
  responde `410 PACKAGE_BLOCKED`. Es una decisión distinta de NuGet a propósito: en pip, un
  yanked seguiría instalándose con un pin exacto, y el bloqueo debe impedirlo. La versión
  sigue visible por la API administrativa.
- `data-dist-info-metadata` (PEP 658) no se implementa; pip cae a descargar la wheel para leer
  la metadata, que es el comportamiento correcto sin esa extensión.

## 8. Validación de ZIP/tar y metadatos (AC-03)

La inspección corre como las de NuGet/npm: en el pool bloqueante, con semáforo propio y plazo
(ADR-014). Se reutilizan las reglas de rutas ya probadas, adaptadas al tipo de archivo.

| Límite | Por defecto |
|---|---|
| Tamaño comprimido | `max_package_bytes` compartido (100 MiB) |
| Tamaño descomprimido total | 512 MiB |
| Entradas ZIP (wheel) | 20 000 |
| Entradas tar (sdist) | 50 000 |
| `METADATA` / `PKG-INFO` | 1 MiB |
| Tiempo de inspección | 30 s |
| Inspecciones simultáneas | las de NuGet |

**Wheel (ZIP):**

- Mismas reglas que el `.nupkg` (Fase 5): número de entradas, tamaños declarados, rutas
  peligrosas, duplicados sin distinguir mayúsculas, enlaces simbólicos y entradas cifradas.
- Debe existir exactamente un `{name}-{version}.dist-info/METADATA` que corresponda al
  filename y a la URL; la metadata se parsea con topes y sin ejecutar nada.
- El filename cumple PEP 427 (distribución escapada con `_`, versión PEP 440, tags); los
  componentes se validan con corpus en OP-033.

**Sdist (`*.tar.gz`):**

- Mismas reglas que el tarball npm: gzip de un solo miembro, solo archivos y directorios, sin
  symlinks, hardlinks, dispositivos, FIFOs, sparse ni entradas cifradas.
- Un único directorio raíz `{name}-{version}/` con `PKG-INFO` dentro; se rechaza cualquier
  ruta absoluta, `..`, con caracteres de control o fuera de ese directorio.

**Metadatos:**

- `name` y `version` normalizados/canonicalizados deben coincidir con el filename, la URL y
  los campos del formulario; `Requires-Python` alimenta el índice y `Summary`/clasificadores
  alimentan `search_text`.
- Los artefactos peligrosos se rechazan sin persistir y se auditan con su código.

Fixtures adversos previstos (OP-033): wheel con traversal, ZIP bomb, entradas duplicadas,
symlink y entrada cifrada; sdist con traversal, symlink, hardlink, gzip bomb y 50 001
entradas; filename o metadata que no coinciden con nombre/versión; `METADATA` ausente,
sobredimensionada o con JSON/RFC 822 inválido; `sha256_digest` que miente; archivo mayor que
el límite; acción distinta de `file_upload`.

## 9. Errores

Formato de error de la superficie PyPI: JSON `{"error": "<CODE>: <mensaje>", "reason":
"<mensaje>", "code": "<CODE>"}`; el prefijo estable es el mismo `CODE` de las demás
superficies y twine/pip muestran el texto.

| Código | HTTP | Cuándo |
|---|---|---|
| `AUTH_REQUIRED` | 401 | Token ausente, inválido, expirado o revocado |
| `FEED_NOT_FOUND` | 404 | Feed inexistente, sin grant o de otro formato |
| `AUTH_SCOPE_MISSING` | 403 | Rol insuficiente para la operación |
| `AUTH_PREFIX_DENIED` | 403 | Nombre fuera de los prefijos del grant |
| `PACKAGE_NOT_FOUND` | 404 | Proyecto, versión o archivo inexistente con acceso al feed |
| `PUBLISH_BODY_INVALID` | 400 | Acción, campos o formulario que no cumplen la sección 6 |
| `FILENAME_MISMATCH` | 400 | Filename que no corresponde al nombre/versión |
| `WHEEL_INVALID` | 400 | ZIP inválido, `METADATA` ausente o incoherente |
| `SDIST_INVALID` | 400 | tar.gz inválido o estructura inesperada |
| `PACKAGE_UNSAFE_PATH` | 400 | Ruta o entrada peligrosa o duplicada |
| `METADATA_INVALID` | 400 | Metadata ilegible, incoherente o sobredimensionada |
| `VERSION_FILE_CONFLICT` | 409 | Mismo filename con bytes distintos |
| `PACKAGE_LIMIT_EXCEEDED` | 413 | Se supera un límite de inspección o el plazo |
| `PACKAGE_TOO_LARGE` | 413 | Se supera el tamaño comprimido |
| `FEED_QUOTA_VERSIONS` | 403 | Cuota de versiones superada |
| `FEED_QUOTA_STORAGE` | 413 | Cuota de almacenamiento superada |
| `PACKAGE_BLOCKED` | 410 | Descarga de un archivo de versión bloqueada |
| `UPLOADS_BUSY` | 503 | Sin plazas de subida |
| `UPLOAD_TIMEOUT` | 408 | Subida agotó el plazo |
| `RATE_LIMITED` | 429 | Límite por principal o IP |
| `INTERNAL` | 500 | Error no clasificado |

## 10. Procedencia y dependency confusion (REQ-03)

pip no tiene registries por scope como npm: resuelve todos los proyectos contra los índices
configurados. Si se combina `--extra-index-url` con PyPI, un nombre privado colisionable puede
resolverse desde el índice público (o al revés). Reglas del primer corte:

- La guía oficial usa **solo** `index-url` apuntando al feed privado; para dependencias
  públicas se decide de forma explícita (publicarlas en el feed o combinarlas a conciencia).
- El registro no puede impedir una configuración insegura del cliente; la documentación
  (`docs/compatibility.md`, `docs/security.md`) advertirá el riesgo y mostrará la
  configuración recomendada.
- Los patrones de publicación por prefijo y la auditoría de uploads reducen la apropiación de
  nombres dentro del feed; no sustituyen la política de nombres del usuario.
- `pip` no consulta ningún índice público por sí solo: si el usuario no configura
  `extra-index-url`, la resolución es solo privada.

## 11. Estados, retirada y yank (decisión)

- **Sin `yank` (PEP 592) en el primer corte.** No se implementa el endpoint ni
  `data-yanked`. La retirada efectiva es el bloqueo administrativo existente (ADR-013), que
  omite archivos del índice y responde `410`.
- **No hay borrado de archivos ni versiones** (ADR-005): la inmutabilidad se aplica por
  filename, no por versión completa. Añadir otro archivo a una versión existente es válido;
  repetir un filename con bytes distintos se rechaza y se audita.
- La decisión de idempotencia del reintento idéntico es la de ADR-007 aplicada por archivo;
  no se traslada la inmutabilidad de versión NuGet a la versión Python, que puede tener varias
  distribuciones.
- Condición de revisión: si el piloto pide yank, se abre una tarea de evolución con su
  migración antes de ampliar el alcance.

## 12. Pruebas y handoff

- **Corpus de identidad:** nombres PEP 508/503 y versiones PEP 440 (epoch, pre, post, dev,
  local, `v` prefijo, comparaciones `1.0`/`1.0.0`), con casos límite de filename de wheel.
- **Unitarias del adaptador:** multipart, extracción de `METADATA`/`PKG-INFO`, integridad,
  orden, render HTML y JSON del índice.
- **Integración de servidor:** auth por superficie, 401/403/404, reintento idéntico,
  `VERSION_FILE_CONFLICT`, cuotas y auditoría, aislamiento entre feeds.
- **Seguridad:** fixtures adversos de la sección 8, sin persistencia.
- **E2E real:** guion `scripts/e2e-pypi.sh` (patrón de `scripts/e2e-dotnet.sh`) y job de CI con
  la matriz de la sección 2: F1–F14 con pip y twine reales, caché vacía y dependencia privada.
- **Docs a actualizar al implementar:** `docs/security.md` (límites, errores y fixtures
  Python), `docs/compatibility.md` (matriz pip/twine y advertencia de dependency confusion),
  `docs/cli.md` o la guía de uso (`.pypirc`, `pip.conf`). La spec no las modifica todavía.
- **Handoff:** OP-033 crea `onepack-pypi` (ADR-026), registra las rutas aprobadas y reutiliza
  `version_file` sin migración nueva; OP-034 decide los comandos y guías; OP-035 verifica el
  registro mixto. Ninguna dependencia de Python entra en `core`, `storage` ni el runtime.

## 13. Verificación de esta especificación

- AC-01: contrato de upload, índice y descargas de las secciones 3, 6 y 7; fixtures de wheel,
  sdist, prerelease y dependencia privada en las secciones 2 y 12.
- AC-02: decisión de archivos de la sección 11 (añadir otro archivo; bytes distintos
  rechazados y auditados) y auth sin filtración de metadatos de la sección 4.
- AC-03: plan con pip desde caché vacía y fixtures adversos de la sección 8, incluidos los
  accesos sin permiso de la sección 4.
- AC-CIERRE: pendiente de la aceptación de la spec, con nota y backlog sincronizados.

Evidencia esperada al cerrar: ruta de este documento, SHA-256, commit local y revisión
registrada en Obsidian.
