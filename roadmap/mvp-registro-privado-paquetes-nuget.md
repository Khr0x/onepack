# Registro privado de paquetes — MVP NuGet

> Propuesta de producto, alcance, arquitectura, stack tecnológico, seguridad, CLI, operación y evolución.

## Resumen ejecutivo

La idea tiene sentido, pero la enfocaría como un producto muy concreto:

> **Un registro privado de paquetes, self-hosted y administrado desde un CLI, que empiece con compatibilidad NuGet y pueda ampliarse después a otros ecosistemas.**

No intentaría crear un sustituto de `dotnet` ni un nuevo resolvedor de dependencias. El producto almacenaría, protegería y distribuiría paquetes; los desarrolladores seguirían utilizando sus herramientas habituales. NuGet ya define un protocolo de servidor que permite esa separación. [Referencia: service index de NuGet](https://learn.microsoft.com/en-us/nuget/api/service-index).

**Mi propuesta técnica inicial sería: Rust + Tokio + Axum + SQLite + almacenamiento local, con servidor y CLI en un mismo workspace.** Sin interfaz web, sin Kubernetes y sin servicios externos obligatorios.

Lo importante es que “ligero” no signifique “incompleto”: autenticación, publicación consistente, permisos y recuperación deben formar parte del MVP.

---

## 1. Definición del producto

### Qué sería

Un servicio que una empresa instala en su infraestructura para publicar y consumir sus librerías privadas.

Por ejemplo, en Hemia podrías tener:

```text
Registro privado
│
├── Feed: internal
│   ├── Hemia.Logging
│   ├── Hemia.Authentication
│   └── Hemia.Observability
│
├── Feed: customer-a
│   ├── CustomerA.Contracts
│   └── CustomerA.Integrations
│
└── Feed: experimental
    └── Hemia.Agents.Preview
```

Cada **feed** sería un repositorio lógico con paquetes, configuración y permisos propios.

Una versión publicada tendría una identidad inequívoca dentro del feed:

```text
feed + package_id + normalized_version
```

El sistema debería permitir que un desarrollador publique una librería, que un pipeline la restaure y que un administrador pueda responder:

> ¿Quién publicó esta versión, cuándo, con qué credencial y cuál es el archivo exacto que estamos distribuyendo?

### A quién lo dirigiría

Como hipótesis inicial, lo enfocaría en equipos pequeños y medianos que necesitan paquetes privados, pero quieren administrar el servicio sin una plataforma compleja.

También encaja con consultoras que separan librerías por cliente y organizaciones que necesitan mantener sus artefactos dentro de su infraestructura.

No empezaría intentando cubrir grandes instalaciones multinodo ni empresas con requisitos avanzados de federación de identidad.

### Dónde estaría la diferenciación

La categoría ya tiene alternativas: **BaGet y BaGetter son servidores NuGet ligeros**, y BaGetter documenta ejecución multiplataforma, soporte ARM64 y capacidades de espejo. Por tanto, “NuGet self-hosted y ligero” no basta como diferenciador. Referencias: [BaGet](https://github.com/loic-sharma/BaGet) y [BaGetter](https://github.com/bagetter/BaGetter).

La propuesta que validaría sería:

> **Un registro privado que se instala fácilmente y se opera completamente desde terminal, con credenciales acotadas, versiones inmutables y recuperación predecible.**

El CLI tendría que resolver problemas operativos reales, no limitarse a envolver llamadas HTTP.

---

## 2. Alcance recomendado del MVP

Para mantenerlo construible, asumiría **una instalación para una organización, con varios feeds privados y un único nodo servidor**.

| Área | Incluiría en el MVP |
|---|---|
| Compatibilidad | Publicación, restauración, búsqueda y metadatos NuGet V3. |
| Feeds | Crear, consultar y configurar varios feeds privados. |
| Identidades | Administradores, usuarios y cuentas de servicio para pipelines. |
| Credenciales | Tokens con expiración, revocación y permisos acotados. |
| Autorización | Roles por feed y restricciones opcionales de publicación por prefijo. |
| Paquetes | Versiones inmutables; listar, consultar, descargar, ocultar y volver a listar. |
| Incidentes | Bloqueo administrativo de una versión. |
| Almacenamiento | SQLite para metadatos y disco local para artefactos. |
| Operación | Diagnóstico, auditoría, comprobación de integridad y backup/restore. |
| Distribución | Binarios del servidor y CLI; imagen de contenedor opcional. |

**Dejaría fuera inicialmente:** interfaz web, proxy de nuget.org, feeds virtuales, S3, PostgreSQL, alta disponibilidad, SSO, soporte npm/PyPI, servidor de símbolos y análisis antimalware.

No son malas funcionalidades. El problema es que cada una abre un frente distinto de compatibilidad, seguridad u operación.

Especialmente importante: **el MVP alojaría paquetes privados; no sería todavía un proxy universal de dependencias**. Los proyectos podrían usar simultáneamente el feed privado y nuget.org, con una configuración explícita de procedencia.

---

## 3. Qué significa realmente “compatible con NuGet”

No basta con implementar un endpoint para subir `.nupkg` y otro para descargarlos.

El cliente descubre los recursos del servidor mediante un **service index**. Esos recursos tienen contratos y versiones independientes. [Referencia: service index](https://learn.microsoft.com/en-us/nuget/api/service-index).

### Recursos que implementaría

Las rutas siguientes son una propuesta propia; los nombres de recursos corresponden al protocolo:

| Recurso | Función | Ruta ilustrativa |
|---|---|---|
| Service index | Descubrir capacidades y endpoints. | `/nuget/{feed}/v3/index.json` |
| `PackageBaseAddress/3.0.0` | Consultar versiones y descargar `.nupkg` y `.nuspec`. | `/nuget/{feed}/v3/flat/` |
| `RegistrationsBaseUrl/3.6.0` | Exponer metadatos, dependencias y versiones. | `/nuget/{feed}/v3/registration/` |
| `SearchQueryService` | Buscar paquetes y navegar resultados. | `/nuget/{feed}/v3/query` |
| `SearchAutocompleteService` | Completar identificadores y consultar versiones. | `/nuget/{feed}/v3/autocomplete` |
| `PackagePublish/2.0.0` | Publicar, ocultar y volver a listar versiones. | `/nuget/{feed}/v2/package` |

Estas capacidades están documentadas en los recursos oficiales de contenido, registros, búsqueda y autocompletado. [Referencia: contenido de paquetes](https://learn.microsoft.com/en-us/nuget/api/package-base-address-resource).

**El `2.0.0` de `PackagePublish` no implica implementar toda la API antigua de NuGet.** Es el recurso de publicación utilizado también desde la API V3. Su publicación usa `PUT`, contenido `multipart/form-data` y la cabecera `X-NuGet-ApiKey`. [Referencia: publicación de paquetes](https://learn.microsoft.com/en-us/nuget/api/package-publish-resource).

### Los detalles que no simplificaría

**Normalización de identidad y versiones.** NuGet tiene particularidades que no cubre automáticamente una librería SemVer genérica. Por ejemplo, admite un cuarto segmento numérico y considera equivalentes determinadas representaciones como `1.0` y `1.0.0`. Estas reglas deben aplicarse también al detectar duplicados. [Referencia: versionado de paquetes](https://learn.microsoft.com/en-us/nuget/concepts/package-versioning).

**Metadatos de dependencias.** Hay que conservar correctamente grupos por framework y rangos de versiones. El servidor expone esa información; no necesita resolver el grafo de dependencias del proyecto. [Referencia: registros de paquetes](https://learn.microsoft.com/en-us/nuget/api/registration-base-url-resource).

**Comportamiento de búsqueda.** Deben respetarse paginación, versiones preliminares y `semVerLevel`. No anunciaría una versión de recurso sin implementar sus requisitos. [Referencia: búsqueda de paquetes](https://learn.microsoft.com/en-us/nuget/api/search-query-service-resource).

**Estados del paquete.** Ocultar una versión no es bloquearla: NuGet contempla que paquetes no listados sigan disponibles para descarga. En particular, el flat container incluye versiones listadas y no listadas. [Referencia: contenido de paquetes](https://learn.microsoft.com/en-us/nuget/api/package-base-address-resource).

Mi criterio sería publicar una **matriz de clientes comprobados**, no prometer “compatibilidad total” sin pruebas.

---

## 4. Stack: qué elegiría y por qué

### Elección principal: Rust

Para la dirección del producto —self-hosted, pocos componentes, CLI nativo y control de recursos— elegiría Rust.

Su modelo de propiedad permite gestionar memoria sin un recolector de basura; eso es una característica útil para esta arquitectura, no una garantía automática de mayor rendimiento o seguridad del producto completo. [Referencia: ownership en Rust](https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html).

| Componente | Elección propuesta |
|---|---|
| Lenguaje del servidor | Rust. |
| Ejecución asíncrona | Tokio. |
| HTTP y routing | Axum. |
| Middleware | Tower / tower-http. |
| Persistencia | SQLite con WAL. |
| Consultas y migraciones | SQLx. |
| Artefactos | Filesystem local mediante una abstracción pequeña. |
| CLI | Rust + clap. |
| Cliente HTTP del CLI | reqwest. |
| Serialización y configuración | serde, JSON y TOML. |
| Inspección de paquetes | Bibliotecas ZIP y XML con límites explícitos. |
| Observabilidad | Logs estructurados, métricas y request IDs. |
| Transporte seguro | HTTPS directo o terminación TLS en reverse proxy. |

Axum está diseñado para trabajar con Tokio y se integra con Tower; SQLx ofrece soporte para SQLite y migraciones embebidas; clap proporciona la base para definir comandos y argumentos. Referencias: [Axum](https://docs.rs/axum/latest/axum/), [SQLx](https://docs.rs/sqlx/latest/sqlx/) y [clap](https://docs.rs/clap/latest/clap/).

No introduciría Redis, Elasticsearch ni una cola externa en esta versión.

### La alternativa seria: C# / ASP.NET Core

**C# sería una elección muy defendible cuando la prioridad principal sea reducir el riesgo de implementar NuGet.**

El SDK oficial ofrece `NuGet.Packaging`, `NuGet.Versioning`, `NuGet.Frameworks` y `NuGet.Protocol`, entre otras bibliotecas. Reutilizarlas evita rehacer parte de las reglas del ecosistema. [Referencia: NuGet Client SDK](https://learn.microsoft.com/en-us/nuget/reference/nuget-client-sdk).

La decisión, para mí, sería:

| Prioridad dominante | Elección |
|---|---|
| Producto nativo, CLI central y futura expansión multiformato | Rust. |
| Reducir trabajo específico de NuGet y aprovechar experiencia .NET | C# / ASP.NET Core. |
| Equipo mucho más productivo en Go | Go sería viable, pero seguiría requiriendo trabajo específico de compatibilidad NuGet. |

**Mantendría Rust como recomendación para tu idea**, con una condición: construir primero una prueba de compatibilidad NuGet. No descubrir después de desarrollar administración, permisos y branding que el adaptador se comporta distinto a los clientes reales.

Usaría pequeños programas .NET como referencia en las pruebas, **sin convertir .NET en una dependencia del servidor desplegado**.

---

## 5. Arquitectura: monolito modular

No lo dividiría en microservicios.

Propondría un proceso servidor con dos superficies HTTP: una compatible con NuGet y otra administrativa para el CLI.

```text
dotnet / clientes NuGet                 CLI administrativo
           │                                   │
           ▼                                   ▼
   API compatible NuGet                 API de gestión /api/v1
           │                                   │
           └────────────────┬──────────────────┘
                            ▼
                 Autenticación y permisos
                            │
                            ▼
                  Casos de uso del dominio
             ┌──────────────┼──────────────┐
             ▼              ▼              ▼
           Feeds         Paquetes       Identidades
             │              │              │
             └──────────────┼──────────────┘
                            ▼
                   Persistencia y auditoría
                    │                  │
                    ▼                  ▼
                  SQLite           Blob storage
                                      │
                                      ▼
                                Disco local
```

El servidor tendría tareas internas acotadas para limpiar temporales, comprobar operaciones incompletas y gestionar mantenimiento. No necesitaría un worker independiente.

### Separación interna importante

**Dominio:** qué significa publicar una versión, quién puede hacerlo y cuándo está disponible.

**Adaptador NuGet:** cómo leer el paquete y traducir esas operaciones al protocolo.

**Persistencia:** cómo guardar metadatos y bytes.

**API administrativa:** cómo exponer operaciones al CLI.

Esto permite añadir otro formato sin convertir el dominio entero en un conjunto de reglas NuGet.

Una estructura razonable:

```text
private-registry/
├── crates/
│   ├── core/
│   ├── nuget/
│   ├── storage/
│   ├── api-client/
│   ├── server/
│   └── cli/
├── migrations/
├── tests/
│   ├── integration/
│   ├── conformance-dotnet/
│   ├── security/
│   └── recovery/
├── packaging/
│   ├── systemd/
│   └── container/
└── docs/
```

No crearía todavía un sistema de plugins dinámicos. Módulos y adaptadores compilados serían suficientes.

---

## 6. Persistencia y publicación consistente

### SQLite para metadatos

Para este MVP, SQLite evita exigir otra instalación y permite mantener la operación en un único nodo.

La limitación debe ser explícita: en WAL puede haber lectores concurrentes, pero solo un escritor a la vez; además, WAL no está pensado para compartir la base mediante un filesystem de red entre máquinas. [Referencia: SQLite WAL](https://www.sqlite.org/wal.html).

Por tanto:

> **Un servidor activo, disco local y transacciones cortas. No varias réplicas compartiendo el mismo archivo SQLite.**

Propondría `synchronous=FULL` como configuración inicial orientada a durabilidad, evaluando su coste en las pruebas. SQLite documenta que `NORMAL` y `FULL` tienen diferencias relevantes ante pérdida de energía. [Referencia: SQLite WAL](https://www.sqlite.org/wal.html).

### Archivos fuera de la base de datos

Guardaría el `.nupkg` original como un objeto inmutable identificado por su hash:

```text
/var/lib/pkgd/
├── metadata.sqlite
├── blobs/
│   └── sha256/
│       └── ab/
│           └── cd/
│               └── <hash>
└── staging/
```

El hash serviría para integridad y posible deduplicación. **Nunca sería una credencial ni permitiría descargar saltándose los permisos del feed.**

Conservaría exactamente los bytes recibidos. No descomprimiría y volvería a empaquetar el artefacto para almacenarlo.

### Modelo de datos inicial

| Entidad | Información principal |
|---|---|
| Feed | Nombre, formato, cuotas y configuración. |
| Principal | Usuario o cuenta de servicio. |
| Token | Verificador del secreto, propietario, expiración y revocación. |
| Grant | Acciones permitidas sobre un feed y restricciones de publicación. |
| Package | Identificador original y normalizado, asociado al feed. |
| PackageVersion | Versión, metadatos, autor de publicación y estado. |
| Blob | Hash, tamaño y ubicación física. |
| AuditEvent | Actor, acción, recurso, fecha y resultado. |

La restricción de unicidad debe existir en la base, no únicamente en código:

```text
UNIQUE(feed_id, normalized_package_id, normalized_version)
```

### Flujo de publicación

```text
Autenticar
   ↓
Comprobar permiso y cuota
   ↓
Recibir por streaming en staging
   ↓
Calcular hash e inspeccionar paquete con límites
   ↓
Validar identidad, versión y metadatos
   ↓
Persistir durablemente el blob
   ↓
Confirmar metadatos + auditoría en una transacción
   ↓
Hacer visible la versión y responder
```

**La base de datos y el filesystem no forman una única transacción.** Diseñaría la recuperación alrededor de esa realidad.

Si el proceso cae después de guardar el blob pero antes de confirmar la base, puede quedar un archivo huérfano recuperable o eliminable. Lo que evitaría es confirmar una versión que todavía no tiene un artefacto durable.

No mantendría una transacción SQLite abierta durante toda la subida.

### Versiones inmutables

Una versión publicada no se reemplaza. Para corregir contenido se publica otra versión.

El endpoint NuGet devolvería `409` ante una identidad ya existente, comportamiento previsto en el protocolo. [Referencia: publicación de paquetes](https://learn.microsoft.com/en-us/nuget/api/package-publish-resource).

El CLI podría ofrecer `--skip-existing-identical`, pero solo consideraría éxito un duplicado después de comprobar que el contenido coincide. Un conflicto con bytes diferentes debe seguir siendo un error.

---

## 7. Seguridad desde la primera versión

La seguridad tendría que cubrir tanto **quién accede** como **qué ocurre cuando recibe un paquete malicioso o defectuoso**.

### Autenticación: tres contextos distintos

| Contexto | Mecanismo propuesto |
|---|---|
| API administrativa | Token mediante `Authorization: Bearer`. |
| Descarga y restauración NuGet | Autenticación Basic sobre HTTPS, con token como contraseña. |
| Publicación NuGet | `X-NuGet-ApiKey`, con permisos de publicación. |

Los clientes NuGet pueden buscar credenciales después de una respuesta `401`, mediante configuración, variables de entorno o un credential provider. No basta con proteger el endpoint de publicación. [Referencia: feeds autenticados](https://learn.microsoft.com/en-us/nuget/consume-packages/consuming-packages-authenticated-feeds).

No usaría una única API key global para toda la instalación.

### Tokens y permisos

Propondría tokens aleatorios de alta entropía, mostrados una sola vez, con expiración y revocación. El servidor almacenaría un verificador del secreto, no el token recuperable.

Para este MVP elegiría tokens opacos verificables contra la base, en lugar de JWT de larga duración.

Los roles iniciales podrían ser:

| Rol | Permisos |
|---|---|
| Reader | Buscar, consultar y descargar. |
| Publisher | Leer y publicar nuevas versiones. |
| Maintainer | Gestionar visibilidad y bloqueos del feed. |
| Administrator | Administrar identidades, permisos y configuración. |

Una cuenta CI debería poder publicar únicamente en su feed y, cuando corresponda, solo determinados prefijos:

```text
Principal: ci-payments
Feed: internal
Acciones: read, publish
Prefijo permitido: Hemia.Payments.*
```

Ese prefijo sería una política del registro, no un namespace especial del protocolo NuGet.

El arranque inicial generaría una credencial administrativa local bajo permisos restrictivos. No expondría un registro público de administradores ni utilizaría una contraseña predeterminada.

### Protección de endpoints

La autorización debe ejecutarse también para búsquedas, metadatos, `.nuspec`, `HEAD`, respuestas condicionales y descargas. No solo para el archivo principal.

Aplicaría HTTPS, límites de solicitudes, cuotas y redacción de secretos en logs. Son controles alineados con las recomendaciones de seguridad de APIs de OWASP. [Referencia: OWASP REST Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html).

No permitiría que Nginx sirva directamente el directorio de blobs mediante una ruta pública que eluda la autorización.

### Validación de paquetes

Trataría cada `.nupkg` como contenido no confiable. Establecería límites para tamaño comprimido, entradas del ZIP, metadatos XML, tiempo de inspección y recursos consumidos.

No ejecutaría scripts, ensamblados ni herramientas incluidos en el paquete. Tampoco descargaría automáticamente recursos indicados por sus metadatos.

OWASP identifica expresamente riesgos como ZIP bombs, XML bombs, vulnerabilidades del parser y sobrescritura de archivos durante cargas. [Referencia: OWASP File Upload Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/File_Upload_Cheat_Sheet.html).

La inspección debería evitar extracción indiscriminada, rechazar rutas peligrosas y trabajar con concurrencia limitada.

**Comprobar un hash no demuestra que un paquete sea benigno.** El MVP no debería anunciar análisis antimalware ni validación de confianza de firmas si únicamente almacena los bytes.

### Ocultar no es bloquear

Usaría dos conceptos separados:

```text
listed: true | false
availability: available | blocked
```

`listed=false` lo retira de descubrimiento; `blocked` impide nuevas descargas desde el servidor. Esta separación evita confundir el unlist de NuGet con una respuesta a incidentes. [Referencia: publicación y listado de paquetes](https://learn.microsoft.com/en-us/nuget/api/package-publish-resource).

Ningún bloqueo del registro puede borrar copias que un cliente ya haya descargado. Esa limitación debe quedar documentada.

---

## 8. El CLI sería la interfaz principal del producto

Utilizaré `pkgd` y `pkgctl` como nombres ilustrativos, no como una marca validada.

**`pkgd` sería el servidor; `pkgctl`, el cliente administrativo.** Compartirían contratos, pero instalar el CLI no obligaría a desplegar el servidor.

### Experiencia propuesta

```bash
# Inicialización local del servidor
pkgd init --data-dir /var/lib/pkgd
pkgd serve --config /etc/pkgd/config.toml

# Conexión desde una estación de trabajo
pkgctl context add hemia \
  --url https://packages.hemia.example

pkgctl login --context hemia

# Administración
pkgctl feed create internal --format nuget
pkgctl principal create ci-payments --type service

# Preparar un proyecto, mostrando primero los cambios
pkgctl nuget init \
  --feed internal \
  --source-name Hemia \
  --prefix "Hemia.*" \
  --dry-run

# Operación cotidiana
pkgctl package list --feed internal
pkgctl package inspect Hemia.Logging@1.2.0 --feed internal
pkgctl package push ./Hemia.Logging.1.2.0.nupkg --feed internal

# Diagnóstico
pkgctl doctor --feed internal
```

Todos son comandos propuestos para el producto, no herramientas ya implementadas.

### Qué debe hacer bien

**Automatización.** Salida `--json` estable, códigos de salida documentados, `--no-input`, timeouts y paginación. Los datos irían a stdout; progreso y diagnóstico, a stderr.

**Interacción.** Autocompletado, ayuda local, confirmaciones para operaciones destructivas y errores que indiquen una acción concreta.

**Credenciales.** Entrada sin eco o por stdin; almacenamiento mediante el gestor de credenciales del sistema cuando esté disponible. No caer silenciosamente a un archivo de texto plano.

**Resiliencia.** Cancelación correcta, reintentos limitados y ninguna repetición automática ciega de operaciones no idempotentes.

**Compatibilidad.** Negociación de capacidades entre CLI y servidor, sin exigir que ambos se actualicen siempre a la vez.

### `doctor` como función diferenciadora

Debería comprobar resolución del servidor, TLS, autenticación, permisos, service index y configuración NuGet.

Un error útil sería:

```text
AUTH_SCOPE_MISSING

La credencial es válida, pero no permite publicar en "internal".

Acción requerida: packages:publish
Principal: ci-payments
Request ID: req_...
```

No mostraría el secreto ni volcaría todas las variables de entorno para diagnosticar.

### El punto delicado: login del CLI y login de NuGet

Guardar un token en el CLI **no hace que `dotnet restore` pueda leerlo automáticamente**.

En el MVP propondría dos vías:

**CI:** credenciales inyectadas mediante mecanismos compatibles con NuGet.

**Desarrollo local:** un comando que ejecute el cliente estándar con credenciales disponibles únicamente en el entorno del proceso hijo:

```bash
pkgctl exec --feed internal -- dotnet restore
```

El resolvedor seguiría siendo `dotnet`; el CLI solo prepararía su autenticación.

NuGet admite credenciales mediante `NuGetPackageSourceCredentials_{nombre}`. Además, el cifrado de contraseñas en `NuGet.Config` tiene limitaciones de plataforma: está soportado en Windows y ligado a usuario y máquina. [Referencia: feeds autenticados](https://learn.microsoft.com/en-us/nuget/consume-packages/consuming-packages-authenticated-feeds).

Un **credential provider** propio sería una evolución posterior para mejorar la experiencia con IDEs. No prometería que guardar credenciales en el CLI resuelve también esa integración. [Referencia: plugin de autenticación multiplataforma de NuGet](https://learn.microsoft.com/en-us/nuget/reference/extensibility/nuget-cross-platform-authentication-plugin).

---

## 9. Paquetes públicos y privados: configuración segura

El asistente `pkgctl nuget init` debería generar o proponer una configuración del proyecto sin secretos, preservando lo existente y mostrando los cambios antes de aplicarlos.

Un ejemplo:

```xml
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add
      key="Hemia"
      value="https://packages.hemia.example/nuget/internal/v3/index.json"
      protocolVersion="3" />
    <add
      key="nuget.org"
      value="https://api.nuget.org/v3/index.json"
      protocolVersion="3" />
  </packageSources>

  <packageSourceMapping>
    <clear />
    <packageSource key="Hemia">
      <package pattern="Hemia.*" />
    </packageSource>
    <packageSource key="nuget.org">
      <package pattern="*" />
    </packageSource>
  </packageSourceMapping>
</configuration>
```

El patrón más específico tiene precedencia: aquí los paquetes `Hemia.*` se asignan al feed privado. También deben quedar cubiertas las dependencias transitivas. No confiaría en el orden de los feeds para conseguirlo. [Referencia: Package Source Mapping](https://learn.microsoft.com/en-us/nuget/consume-packages/package-source-mapping).

Hay dos límites importantes. Los paquetes ya presentes en la caché global pueden evitar una nueva consulta al origen; por eso las pruebas de procedencia deben usar cachés limpias o aisladas. [Referencia: Package Source Mapping](https://learn.microsoft.com/en-us/nuget/consume-packages/package-source-mapping).

Además, el mapping **no garantiza confidencialidad absoluta de los identificadores**: determinadas consultas de metadatos pueden dirigirse a todas las fuentes configuradas. Para requisitos estrictos de aislamiento habría que restringir fuentes y salida de red, no depender únicamente del mapping. [Referencia: configuración de NuGet](https://learn.microsoft.com/en-us/nuget/reference/nuget-config-file).

---

## 10. Despliegue, mantenimiento y recuperación

### Despliegue inicial

Propondría servidor Linux en x86-64 y ARM64, con CLI para Linux, macOS y Windows. El soporte de servidor Windows podría añadirse después de validar demanda.

La instalación mínima sería:

```text
1 binario servidor
1 archivo de configuración
1 directorio de datos
1 servicio del sistema
```

Ejemplo de configuración propuesta:

```toml
[server]
listen = "127.0.0.1:8080"
public_url = "https://packages.hemia.example"

[database]
path = "/var/lib/pkgd/metadata.sqlite"

[storage]
driver = "filesystem"
path = "/var/lib/pkgd/blobs"

[security]
anonymous_read = false

[limits]
max_package_size_mib = 100
max_concurrent_uploads = 4
```

Los límites serían configurables; no restricciones universales de NuGet.

`public_url` sería explícito para generar correctamente URLs detrás del reverse proxy. No lo derivaría de cualquier cabecera `Host` recibida.

Docker sería otra presentación del mismo producto, no un requisito.

### Backup y restore dentro del MVP

Empezaría con **backup consistente en modo mantenimiento**, pausando mutaciones y limpieza de blobs durante la operación.

El respaldo debe contener metadatos, blobs referenciados, configuración necesaria y un manifiesto de integridad. Un respaldo de la base sin los artefactos no recupera el registro.

SQLite ofrece mecanismos específicos de backup; copiar sin coordinación únicamente el archivo principal de una base activa no es un procedimiento suficiente para este diseño. [Referencia: SQLite Online Backup API](https://www.sqlite.org/backup.html).

Las actualizaciones incluirían versión de esquema, comprobaciones previas y migraciones controladas. No asumiría que cambiar al binario anterior revierte automáticamente una migración de datos.

**La prueba de recuperación termina al restaurar un proyecto desde una instalación reconstruida**, no al comprobar que se generó un archivo de backup.

---

## 11. Rendimiento: objetivos medibles

No vendería “rápido porque está escrito en Rust”. Definiría un presupuesto de recursos y un escenario de evaluación.

Por ejemplo, para un entorno de prueba de **2 vCPU, 1 GB de RAM y SSD local**, propondría estos objetivos iniciales, todavía pendientes de medición:

| Indicador | Objetivo de diseño |
|---|---|
| Inicio del servidor | Menos de 2 segundos con una base pequeña. |
| Memoria en reposo | Menos de 100 MiB de RSS. |
| Consulta autenticada de metadatos | p95 inferior a 50 ms en red local y caché caliente. |
| Transferencias | Memoria acotada, no proporcional al tamaño completo del paquete. |
| Concurrencia | Degradación controlada bajo saturación, sin crecimiento ilimitado de tareas. |
| Publicación | Disponible para lectura al confirmar la operación. |

Son objetivos, no resultados ni mínimos garantizados.

Para alcanzarlos priorizaría streaming, índices adecuados, paginación, cachés acotadas y límites de concurrencia. La inspección ZIP/XML tendría su propio presupuesto para no bloquear la atención de descargas.

Evitaría convertir cada descarga en varias escrituras SQLite. Las mutaciones importantes tendrían auditoría durable; los contadores de acceso podrían agregarse sin bloquear cada lectura.

---

## 12. Plan de construcción y criterios de aceptación

Lo desarrollaría por hitos verificables, no comenzando por una plataforma genérica enorme.

| Hito | Qué debe demostrar |
|---|---|
| Compatibilidad | Un paquete creado con herramientas oficiales se publica y restaura mediante clientes reales. |
| Persistencia | La versión conserva exactamente sus bytes y sobrevive a reinicios e interrupciones. |
| Seguridad | Feeds aislados, tokens revocables y permisos comprobados en todos los endpoints. |
| CLI | Configuración, publicación, consulta y diagnóstico sin interfaz web. |
| Operación | Backup, restauración, actualización y recuperación comprobados. |
| Piloto | Un equipo lo utiliza en sus builds y publicaciones habituales. |

Los paquetes de prueba deberían cubrir distintas versiones, frameworks, dependencias y metadatos. Las herramientas oficiales de creación deben ser la referencia; Microsoft recomienda usarlas para producir paquetes bien formados. [Referencia: NuGet Client SDK](https://learn.microsoft.com/en-us/nuget/reference/nuget-client-sdk).

### Pruebas que consideraría bloqueantes

| Escenario | Resultado esperado |
|---|---|
| Dos publicaciones simultáneas de la misma identidad | Solo una versión confirmada, sin sobrescritura. |
| Caída durante una subida | No aparece una versión parcialmente disponible. |
| Disco lleno | Error controlado, sin metadatos apuntando a un blob inexistente. |
| Token revocado | Las nuevas solicitudes dejan de autorizarse. |
| Lectura entre feeds sin permiso | Ninguna filtración mediante búsqueda, metadatos o contenido. |
| ZIP o XML diseñado para agotar recursos | Rechazo controlado dentro de los límites. |
| Versión no listada | No se descubre en búsqueda, pero continúa disponible según su estado. |
| Restauración de backup en otro servidor | El proyecto vuelve a restaurar sus dependencias. |

También probaría proxies, certificados, configuración heredada de NuGet y cachés vacías. Probar únicamente con el CLI propio sería insuficiente: podría ocultar incompatibilidades compartidas entre cliente y servidor.

---

## 13. Evolución del producto

Después de estabilizar el núcleo, seguiría este orden:

| Etapa | Ampliación |
|---|---|
| Primera evolución | Credential provider, importación de paquetes y almacenamiento S3 compatible. |
| Segunda evolución | Proxy/cache de NuGet con reglas explícitas de procedencia. |
| Tercera evolución | npm, conservando sus propias reglas de identidad y protocolo. |
| Según demanda | Símbolos, PyPI, SSO, interfaz web y funciones empresariales. |
| Solo con necesidad demostrada | PostgreSQL, múltiples nodos y alta disponibilidad. |

El proxy merece tratarse como un proyecto de seguridad propio: introduce acceso saliente, credenciales upstream, colisiones de identidad y políticas de caché. No lo implementaría como “si no existe localmente, descargar de cualquier URL”.

Tampoco asumiría que todos los formatos comparten exactamente las reglas NuGet. Compartiría almacenamiento, permisos y auditoría; mantendría validación y normalización en cada adaptador.

Para un posible modelo comercial, dejaría **el núcleo self-hosted, permisos esenciales y recuperación plenamente utilizables**, y evaluaría monetizar servicio administrado, soporte, integración empresarial y operación avanzada. Primero validaría que los equipos eligen realmente esta experiencia frente a alternativas existentes.

---

## Recomendación final

Construiría un **registro privado NuGet de un solo nodo**, con:

**Servidor Rust, CLI Rust, SQLite, disco local, autenticación obligatoria, permisos por feed, versiones inmutables y recuperación incorporada.**

No empezaría con S3, proxy, microservicios ni interfaz web. La mayor inversión inicial debería ir a **compatibilidad con clientes NuGet, gestión de credenciales y consistencia de publicación**.

La prueba decisiva del MVP sería esta:

> Un equipo instala el servidor, configura su proyecto, publica una librería y la restaura desde CI; después revoca una credencial y recupera el servicio desde un backup, sin abrir una interfaz web ni editar manualmente la base de datos.

**Ese sería un MVP pequeño en infraestructura, pero completo en su función.**
