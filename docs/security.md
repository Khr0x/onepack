# Seguridad frente a paquetes y abuso

> Fase 5 ([ADR-013](../roadmap/adr-mvp.md#adr-013), [ADR-014](../roadmap/adr-mvp.md#adr-014), [ADR-018](../roadmap/adr-mvp.md#adr-018)).
> Última actualización: 2026-10-04

## Qué hace y qué no hace el registro

- Trata cada `.nupkg` como **contenido no confiable**: lo inspecciona con un presupuesto acotado, nunca lo extrae a disco y nunca ejecuta ni descarga nada que el paquete referencie.
- **No analiza malware.** Un paquete que supere la inspección puede contener código malicioso.
- **No valida la confianza de las firmas.** Los paquetes firmados se guardan y sirven byte a byte, sin comprobar el certificado.
- **Bloquear una versión no borra las copias ya descargadas** en cachés de clientes (`~/.nuget/packages`, cachés HTTP, agentes de CI) ni en espejos. Tras un incidente hay que limpiar esas cachés por separado.

## Inspección de paquetes

Cada subida pasa por estas comprobaciones antes de guardar nada. Todas se configuran en `onepackd serve` (flag o variable de entorno):

| Límite | Flag | Por defecto |
|---|---|---|
| Tamaño comprimido | `--max-package-size-mib` | 100 MiB |
| Número de entradas del ZIP | `--max-zip-entries` | 20 000 |
| Tamaño descomprimido por entrada | `--max-entry-size-mib` | 512 MiB |
| Tamaño descomprimido total | `--max-uncompressed-size-mib` | 2048 MiB |
| Tamaño del `.nuspec` | `--max-nuspec-size-kib` | 1024 KiB |
| Profundidad XML del `.nuspec` | `--max-xml-depth` | 32 |
| Tiempo de inspección | `--inspection-timeout-secs` | 30 s |
| Inspecciones simultáneas | `--max-concurrent-inspections` | la mitad de los núcleos |

Cómo se aplican:

- El número de entradas se lee del registro de fin del ZIP (también en ZIP64) **antes** de cargar el directorio central, así que un ZIP que declara millones de entradas no reserva memoria para ellas.
- Los tamaños descomprimidos se comprueban con los que declara el directorio central, sin descomprimir. Solo se descomprime el `.nuspec`, y su lectura se corta en el límite aunque el tamaño declarado mienta.
- El `.nuspec` se analiza **sin DTD**: cualquier `<!DOCTYPE>` se rechaza, así que no hay entidades definidas por el documento (billion laughs) ni externas (XXE). Solo se aceptan las cinco entidades predefinidas de XML y las referencias numéricas.
- Se rechazan rutas que serían peligrosas al extraerlas en un cliente: absolutas (`/x`, `\\x`, `C:/x`), con segmentos `.` o `..`, con caracteres de control o `:`, con nombres reservados de Windows (`CON`, `PRN`, `AUX`, `NUL`, `COM0-9`, `LPT0-9`, con o sin extensión), terminadas en punto o espacio, y con escapes `%XX` incompletos. Cada ruta se comprueba tal cual y con el escape `%XX` resuelto, porque los clientes NuGet lo deshacen al extraer.
- También se rechazan entradas duplicadas (sin distinguir mayúsculas ni escapes), enlaces simbólicos y entradas cifradas.
- La inspección corre en el pool de hilos bloqueantes con un semáforo propio: nunca ocupa los hilos que atienden descargas.

## Errores

Las respuestas de error llevan un código estable al inicio del texto (`CÓDIGO: mensaje`) en las rutas NuGet, que los clientes muestran tal cual, y `{"error": {"code", "message"}}` en `/api/v1`.

| Código | Estado | Cuándo |
|---|---|---|
| `PACKAGE_INVALID` | 400 | ZIP o `.nuspec` inválido, DTD, entidad desconocida, entrada cifrada o enlace simbólico. |
| `PACKAGE_UNSAFE_PATH` | 400 | Ruta peligrosa o entrada duplicada. |
| `PACKAGE_LIMIT_EXCEEDED` | 413 | Se supera un límite de inspección o el tiempo máximo. |
| `PACKAGE_TOO_LARGE` | 413 | Se supera el tamaño comprimido. |
| `FEED_QUOTA_VERSIONS` | 403 | El feed alcanzó su cuota de versiones. |
| `FEED_QUOTA_STORAGE` | 413 | El feed alcanzó su cuota de almacenamiento. |
| `PACKAGE_BLOCKED` | 410 | Descarga de una versión bloqueada. |
| `UPLOADS_BUSY` | 503 + `Retry-After` | No quedan plazas de subida. |
| `UPLOAD_TIMEOUT` | 408 | La subida superó `--upload-timeout-secs`. |
| `RATE_LIMITED` | 429 + `Retry-After` | Límite de peticiones por principal o por IP. |

Los rechazos de publicación se auditan (`package.publish`, `denied`, con el código).

## Saturación

| Límite | Flag | Por defecto |
|---|---|---|
| Subidas simultáneas | `--max-concurrent-uploads` | 8 |
| Tiempo máximo de una subida | `--upload-timeout-secs` | 600 s |
| Peticiones por principal | `--principal-rate-limit` / `--principal-rate-burst` | 50/s, ráfaga de 1000 |
| Peticiones por IP | `--ip-rate-limit` / `--ip-rate-burst` | 100/s, ráfaga de 2000 |

- La plaza de subida se toma **antes de leer el cuerpo**: una ráfaga recibe `503` sin crear archivos en staging ni tareas pendientes.
- El límite por IP se aplica **antes de autenticar**, así que también frena la fuerza bruta de tokens. El límite por principal se aplica después.
- La IP es la del socket. Detrás de un reverse proxy todas las peticiones comparten la IP del proxy, porque el servidor no confía en `X-Forwarded-For` ([ADR-018](../roadmap/adr-mvp.md#adr-018)). En ese despliegue conviene limitar por IP en el proxy, y subir `--ip-rate-limit` o desactivarlo con `0`.
- Un `restore` de una solución grande hace varias peticiones por paquete. Las ráfagas por defecto lo absorben; ajústalas si los agentes de CI comparten credencial.
- Los contadores viven en memoria y se pierden al reiniciar. Su tamaño está acotado: con más de 100 000 claves se olvidan los clientes inactivos.

## Cuotas por feed

```bash
onepackd feed quota internal --max-storage-mib 10240 --max-versions 5000 --data-dir ./data
onepackd feed show internal --data-dir ./data
```

`0` significa sin límite. El uso es la suma lógica de los tamaños de las versiones: dos versiones con el mismo contenido cuentan dos veces, aunque en disco se guarde una sola copia. Una cuota nueva no afecta a lo ya publicado aunque lo supere. Si una publicación superaría la cuota, no se guarda nada y se audita con el código.

## Bloqueo de versiones

`availability` es independiente de `listed` ([ADR-013](../roadmap/adr-mvp.md#adr-013)):

| | Búsqueda | Flat container y registros | Descarga |
|---|---|---|---|
| `listed=false` | no | sí (`listed: false`) | sí |
| `blocked` | no | sí, anunciada como obsoleta | `410 PACKAGE_BLOCKED` |

La versión bloqueada se mantiene en los metadatos a propósito: así `restore` falla con un error explícito, en vez de resolver en silencio otra versión o decir que no existe. Los clientes .NET muestran la frase de estado:

```text
Response status code does not indicate success: 410 (PACKAGE_BLOCKED - version blocked by the registry).
```

Bloquear y desbloquear requiere rol Maintainer (o administrador) y un motivo, que queda en la auditoría (`package.block`, `package.unblock`). El motivo no se publica en los metadatos NuGet; solo lo ve quien puede mantener el feed:

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"reason": "CVE-2026-0001"}' \
  https://packages.example.com/api/v1/feeds/internal/packages/hemia.core/1.0.0/block
curl -H "Authorization: Bearer $TOKEN" \
  https://packages.example.com/api/v1/feeds/internal/packages/hemia.core/1.0.0
```

Con el CLI:

```bash
onepack package block --feed internal Hemia.Core 1.0.0 --reason "CVE-2026-0001"
onepack package inspect --feed internal Hemia.Core 1.0.0
onepack package unblock --feed internal Hemia.Core 1.0.0 --reason "parche publicado"
```

Sin servidor en marcha, o con acceso solo al directorio de datos:

```bash
onepackd package block --feed internal --id Hemia.Core --version 1.0.0 --reason "CVE-2026-0001" --data-dir ./data
onepackd package unblock --feed internal --id Hemia.Core --version 1.0.0 --reason "parche publicado" --data-dir ./data
```

## Suite de seguridad

`tests/security` genera en cada ejecución los artefactos maliciosos y comprueba que se rechazan sin persistir nada:

```bash
cargo test -p onepack-server --test security
```

| Escenario | Prueba |
|---|---|
| ZIP bomb con tamaños falseados (~4 GiB por entrada) | `zip_bomb_with_forged_sizes_is_rejected_without_decompressing` |
| ZIP bomb real (12 MiB de ceros en menos de 200 KiB) | `zip_bomb_with_real_compression_is_rejected` |
| `.nuspec` de 16 MiB, con tamaño honesto y falseado | `oversized_nuspec_is_cut_at_the_limit` |
| Demasiadas entradas | `too_many_entries_are_rejected` |
| Billion laughs | `billion_laughs_is_rejected` |
| XXE y entidades sin definir | `external_entities_are_never_resolved` |
| 10 000 niveles de anidamiento | `deeply_nested_xml_is_rejected` |
| Path traversal, rutas absolutas, nombres reservados, ADS | `path_traversal_is_rejected_and_nothing_is_written_outside_staging` |
| Entradas duplicadas | `duplicate_entries_are_rejected` |
| Ráfaga de subidas | `upload_burst_degrades_with_503_without_queueing` |
| Subida que no termina | `stalled_upload_times_out` |
| Límite por principal y por IP | `requests_are_rate_limited_per_principal`, `requests_are_rate_limited_per_ip_before_authentication` |
| Versión bloqueada | `blocked_version_cannot_be_downloaded_but_stays_visible` y el paso de bloqueo de `scripts/e2e-dotnet.sh` con `dotnet restore` real |
| Cuotas | `version_quota_rejects_with_specific_code`, `storage_quota_rejects_with_specific_code` |

## Revisión manual de seguridad (Fase 5)

Revisión hecha sobre el código de la rama `feature/phase-5-hardening`, el 2026-10-04.

| Área | Comprobación | Resultado |
|---|---|---|
| Lectura del ZIP | Solo se descomprime el `.nuspec`, con `take(límite + 1)`; el resto se valida con el directorio central (`by_index_raw`). | Correcto. |
| Memoria del directorio central | El número de entradas se valida antes de `ZipArchive::new`. El tamaño del directorio central está acotado por el tamaño comprimido. | Correcto. |
| Escritura en disco | La subida va a `staging/` con un nombre que genera el servidor y pasa a `blobs/sha256/...` por hash. Ningún nombre de entrada del ZIP llega al sistema de archivos. | Correcto. |
| XML | `DocType` se rechaza antes de procesar entidades. Las entidades generales solo resuelven las predefinidas. Profundidad y tamaño acotados. | Correcto. |
| Segmentos de URL | Id y versión se normalizan con los parsers NuGet antes de usarse en consultas; los valores inválidos son `404`. | Correcto (Fase 3). |
| Tiempo | La inspección comprueba un plazo en cada entrada y en cada evento XML. La subida tiene su propio plazo. | Correcto. Riesgo residual: una sola lectura bloqueante lenta del disco no se interrumpe. |
| Concurrencia | Plazas de subida con `try_acquire` (sin cola) y semáforo de inspección. Los rechazos no crean archivos. | Correcto. |
| Rate limiting | Cubo de fichas en memoria con número de claves acotado. Por IP antes de autenticar. | Correcto. Riesgo residual: detrás de un proxy el límite por IP es global (documentado). |
| Bloqueo | Comprobado en el handler de descarga para `.nupkg` y `.nuspec`, para cualquier rol. El motivo no se expone en NuGet. | Correcto. |
| Cuotas | Comprobación previa y dentro de la transacción del único escritor, así que dos publicaciones concurrentes no superan la cuota juntas. | Correcto. Si la cuota se supera entre la comprobación previa y la transacción, el blob queda huérfano y lo recoge la limpieza. |
| Secretos en logs | Los motivos de bloqueo y los códigos se registran; los tokens no (E2E lo comprueba también con el token administrativo). | Correcto. |
| Fuera de alcance | Antimalware, verificación de firmas, cuarentena automática, límites por IP de proxies de confianza. | Documentado arriba. |
