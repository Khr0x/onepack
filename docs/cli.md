# CLI `onepack`

> Fase 6 ([ADR-015](../roadmap/adr-mvp.md#adr-015), [ADR-016](../roadmap/adr-mvp.md#adr-016)).
> Última actualización: 2026-10-04

`onepack` opera el registro desde la terminal: lo usan personas y pipelines. Es un binario distinto del servidor (`onepackd`) y no lo necesita instalado.

```bash
cargo build --release -p onepack-cli   # target/release/onepack
```

## Contextos

Un contexto es un servidor con nombre. La configuración (`config.json`) nunca contiene secretos.

```bash
onepack context add prod --url https://packages.example.com --use
onepack context add lab --url https://lab.example.test --ca-cert ./ca.pem
onepack context list
onepack context use lab
onepack context remove lab --yes
```

Ubicación: `$ONEPACK_CONFIG_DIR`, o `$XDG_CONFIG_HOME/onepack`, o `~/.config/onepack` (`%APPDATA%\onepack` en Windows). `--context` o `ONEPACK_CONTEXT` eligen otro contexto para un comando; `--url` o `ONEPACK_URL` apuntan a un servidor sin contexto.

## Credenciales

El token se resuelve en este orden:

1. `--token-env VAR`: el token está en la variable `VAR`. Es la opción para pipelines.
2. `ONEPACK_TOKEN`.
3. El keychain del sistema (macOS Keychain, Windows Credential Manager, Secret Service en Linux), guardado con `onepack login`.

```bash
onepack login                  # pide el token sin eco
printf %s "$TOKEN" | onepack login --token-stdin
onepack logout
onepack whoami
```

`login` valida el token contra el servidor antes de guardarlo. **No hay fallback a texto plano**: si no hay keychain, `login` falla con `KEYCHAIN_UNAVAILABLE` (salida 9) y sugiere `--token-env`. `ONEPACK_KEYRING=off` desactiva el keychain a propósito (contenedores, CI).

## Comandos

| Comando | Qué hace | Requiere |
|---|---|---|
| `feed create\|list\|show` | Feeds visibles para la credencial. | lectura (crear: admin) |
| `feed configure NAME [--max-storage-mib N] [--max-versions N]` | Cuotas; lo no indicado se mantiene, `0` es sin límite. | admin |
| `principal create\|list\|disable` | Usuarios y cuentas de servicio. `disable` corta sus tokens y no permite desactivar al último administrador. | admin |
| `token create --principal P [--name N] [--expires-in-days D]` | Emite un token. En texto solo el token va a stdout; no se vuelve a mostrar. | admin |
| `token list [--principal P]`, `token revoke ID` | Sin secretos; `ID` son los 16 caracteres tras `opk_`. | admin |
| `grant add --principal P --feed F --role R [--publish-pattern X]…` | Asigna o reemplaza el rol. | admin |
| `grant remove`, `grant list [--principal] [--feed]` | | admin |
| `package list --feed F`, `package inspect --feed F ID [VERSION]` | Paquetes, versiones y estado. El motivo de un bloqueo solo lo ve Maintainer o admin. | lectura |
| `package push --feed F FILE… [--skip-existing-identical]` | Publica por la misma ruta que `dotnet nuget push`. Valida el paquete en local antes de subirlo. Con la opción, una versión existente con el **mismo** contenido (SHA-256) no es error; con otro contenido sí. | publicación |
| `package unlist\|relist --feed F ID VERSION` | Visibilidad en búsqueda; no afecta a la descarga. | publicación |
| `package block --feed F ID VERSION --reason R`, `package unblock …` | Impide la descarga ([seguridad](security.md#bloqueo-de-versiones)). | Maintainer |
| `audit list [--feed F] [--action PREFIJO] [--limit N]` | Del más reciente al más antiguo. | admin |
| `nuget init --feed F --pattern P [--config PATH] [--dry-run]` | Ver [NuGet.Config](#nugetconfig-y-credenciales-de-nuget). | — |
| `exec --feed F [--source-name N] -- CMD…` | Ver [NuGet.Config](#nugetconfig-y-credenciales-de-nuget). | lectura |
| `doctor [--feed F] [--require read\|publish\|maintain]` | Ver [Diagnóstico](#diagnóstico). | — |
| `completion bash\|zsh\|fish\|powershell\|elvish` | Script de autocompletado. | — |

Las operaciones destructivas (`token revoke`, `principal disable`, `grant remove`, `package block`, `context remove`) piden confirmación. Sin terminal o con `--no-input` (`ONEPACK_NO_INPUT=1`) exigen `--yes`.

## NuGet.Config y credenciales de NuGet

Guardar un token en el CLI no hace que `dotnet restore` lo use ([ADR-016](../roadmap/adr-mvp.md#adr-016)). Hay dos piezas:

**`onepack nuget init`** añade cada feed a `NuGet.Config` con la clave `onepack_<feed>` (los `-` pasan a `_`) y un `packageSourceMapping` con los patrones indicados:

```bash
onepack nuget init --feed internal --pattern 'Hemia.*' --dry-run   # muestra el diff
onepack nuget init --feed internal --pattern 'Hemia.*' --yes
```

- Conserva todo lo que ya había (comentarios, otras fuentes, otras secciones); lo que no cambia se reescribe byte a byte.
- **Nunca escribe credenciales.** Si encuentra una `ClearTextPassword`, avisa.
- Si el archivo no tenía `packageSourceMapping`, mapea el resto de sus fuentes a `*` para no romper lo que ya restauraba (los patrones de onepack son más específicos y ganan). Si el archivo no empieza con `<clear />`, también mapea `nuget.org`, que suele heredarse de la configuración del usuario. Cualquier otra fuente heredada hay que mapearla a mano.
- Con una URL `http://` añade `allowInsecureConnections="true"` y avisa: úsalo solo en local.
- `packageSourceMapping` reduce el riesgo de *dependency confusion*, pero no garantiza que los ids internos no se consulten en otras fuentes. Las pruebas de procedencia deben usar cachés limpias.

**`onepack exec`** ejecuta un comando con `NuGetPackageSourceCredentials_<clave>` definida **solo en el entorno del proceso hijo**. El token no se escribe en disco ni queda en el shell. El código de salida es el del comando.

Antes de lanzarlo, `exec` comprueba la credencial (`GET /api/v1/whoami`): sin esa comprobación, NuGet solo diría `NU1301: no se puede cargar el índice de servicio`. Si el token no es válido (revocado, caducado o de un principal desactivado), falla con salida 3; si no tiene acceso a alguno de los feeds, con salida 4 y `AUTH_SCOPE_MISSING`. En ambos casos el comando no se ejecuta. Si el servidor no responde, avisa y ejecuta el comando igualmente: puede bastar la caché local.

```bash
onepack exec --feed internal -- dotnet restore
onepack exec --feed internal --feed customer-a -- dotnet build
```

En CI no hace falta `exec`: define la variable directamente con el secreto del pipeline (ver la [guía](guide-zero-to-ci.md)).

## Diagnóstico

`onepack doctor --feed F` comprueba en orden la URL, el DNS, la conexión TCP, el TLS (o avisa de HTTP sin TLS fuera de loopback), la credencial (muestra solo el id del token y su origen), la autenticación y la caducidad, el servidor y sus capacidades, los permisos en el feed (`--require`), el service index (que sus URLs partan de la del contexto, es decir, que `--public-url` del servidor coincida), y el `NuGet.Config` del directorio actual o de sus padres: fuente, credenciales en texto plano y source mapping.

Cada comprobación da `ok`, `warn`, `fail` o `skip`. Cuando falla, incluye un código y la acción a tomar. Sale con 10 si alguna falla.

## Contrato de salida

- Los datos van a **stdout** y el diagnóstico (avisos, progreso, errores) a **stderr**.
- `--json` escribe un documento JSON por comando. Su forma (claves y tipos) está fijada por un snapshot en `crates/cli/tests/snapshots/json-schema.json`. Un cambio incompatible exige versión mayor del CLI. Los listados son arrays con todas las páginas.
- Los errores con `--json` van a stderr como `{"error": {"code", "message", "action", "request_id"}}`. Sin `--json`:

  ```text
  error: AUTH_SCOPE_MISSING: la credencial es válida, pero no tiene el permiso packages:publish en este feed
    acción: pide a un administrador un rol suficiente: `onepack grant add --principal <principal> --feed <feed> --role <rol>`
    request_id: 3f9c0a1b2c3d4e5f
  ```

  El `request_id` aparece también en el log del servidor. Los mensajes nunca incluyen el token.

### Códigos de salida

| Código | Significado |
|---|---|
| 0 | Éxito. |
| 1 | Error no clasificado (E/S local, operación cancelada, …). |
| 2 | Uso incorrecto: argumentos, entrada inválida, contexto inexistente o confirmación requerida con `--no-input`. |
| 3 | Sin credencial válida: ausente, caducada o revocada (`401`). |
| 4 | Credencial válida sin permiso (`403`). |
| 5 | No encontrado (`404`). |
| 6 | Conflicto: ya existe (`409`); p. ej. una versión publicada con otro contenido. |
| 7 | No disponible y reintentable: red, DNS, TLS, timeout, `429` o `5xx`. |
| 8 | Servidor incompatible: no ofrece una capacidad que el comando necesita. |
| 9 | No hay keychain del sistema. |
| 10 | `doctor` encontró fallos. |

`exec` devuelve el código del comando ejecutado (128 + señal si terminó por una señal).

## Red y compatibilidad

- `--timeout SECS` (`ONEPACK_TIMEOUT`): 30 s por petición; 600 s en `package push`.
- Solo se reintentan las peticiones idempotentes (`GET`, `PUT`, `DELETE`), hasta 3 intentos, ante errores de conexión, timeouts y `429`/`502`/`503`/`504`, respetando `Retry-After`. `package push` y las operaciones `POST` no se reintentan.
- TLS con las raíces de confianza del sistema; `--ca-cert` (`ONEPACK_CA_CERT`) usa una CA propia en PEM.
- Antes de cada comando administrativo, el CLI consulta `GET /api/v1/capabilities`. Si falta la capacidad que necesita, falla con `CAPABILITY_MISSING` (salida 8) e indica la versión del servidor. Un servidor sin ese endpoint (anterior a la Fase 6) se trata como uno sin capacidades administrativas. La API es la [v1](api.md).
