# De cero a restore en CI

> Guía de la Fase 6. Se ejecuta tal cual, de arriba abajo, en una terminal bash (Linux o macOS).
> Última actualización: 2026-10-04

Al terminar tendrás un registro en marcha, un feed privado, una cuenta de servicio para CI con un token limitado a sus paquetes, un paquete publicado y un proyecto que lo restaura como lo haría un agente de CI. Todo sin interfaz web: solo `onepack` y `dotnet`.

Tiempo estimado: 15 minutos (más la primera compilación).

## 0. Requisitos

- [rustup](https://rustup.rs) (la versión de Rust la fija el repositorio).
- SDK de .NET 8 o posterior (`dotnet --version`).
- `git` y `curl`.

## 1. Compilar los binarios

```bash
git clone https://github.com/Khr0x/onepack.git
cd onepack
cargo build --release -p onepack-server -p onepack-cli
export PATH="$PWD/target/release:$PATH"
onepack --version
onepackd --version
```

`onepackd` es el servidor; `onepack`, el CLI. En una estación de trabajo basta con `onepack`.

## 2. Arrancar el servidor

```bash
export DEMO="$HOME/onepack-demo"
mkdir -p "$DEMO"
onepackd init --data-dir "$DEMO/data"
```

`init` crea el directorio de datos y escribe la credencial administrativa inicial en `$DEMO/data/initial-admin-token`. En **otra terminal**, arranca el servidor y déjalo corriendo:

```bash
export DEMO="$HOME/onepack-demo"
export PATH="$PWD/target/release:$PATH"   # desde el directorio del repositorio
onepackd serve --data-dir "$DEMO/data" --public-url http://127.0.0.1:8080
```

> Esta guía usa HTTP en `127.0.0.1` para probar en local. En un servidor real, pon `onepackd` detrás de TLS y usa como `--public-url` la URL `https://` con la que llegarán los clientes.

Vuelve a la primera terminal.

## 3. Configurar el CLI como administrador

```bash
onepack context add local --url http://127.0.0.1:8080
onepack login --token-stdin < "$DEMO/data/initial-admin-token"
```

`login` valida el token y lo guarda en el keychain del sistema. Si responde `KEYCHAIN_UNAVAILABLE` (p. ej. en un servidor sin sesión gráfica), pasa el token por una variable de entorno:

```bash
export ONEPACK_TOKEN="$(cat "$DEMO/data/initial-admin-token")"
```

Comprueba la identidad y, ya guardada, borra el archivo de la credencial inicial:

```bash
onepack whoami
rm "$DEMO/data/initial-admin-token"   # solo si usaste `login`; con ONEPACK_TOKEN, guárdala antes en un gestor de secretos
```

`whoami` debe mostrar `admin (user, administrador)`.

## 4. Crear el feed y la cuenta de CI

```bash
onepack feed create payments
onepack principal create ci-payments --kind service
onepack grant add --principal ci-payments --feed payments --role publisher --publish-pattern 'Hemia.Payments.*'
CI_TOKEN="$(onepack token create --principal ci-payments --name github-actions --expires-in-days 90)"
export CI_TOKEN
```

El token se muestra una sola vez; en un caso real se guardaría directamente como secreto del pipeline. Con `--publish-pattern`, la cuenta de CI solo puede publicar ids `Hemia.Payments.*` en `payments`.

Comprueba qué puede hacer:

```bash
onepack --token-env CI_TOKEN doctor --feed payments --require publish
```

Todas las comprobaciones deben salir `ok`, salvo `nuget-config`, que avisa (`WARN`) porque aún no hay `NuGet.Config`; el resultado final es `Sin fallos.` El token nunca aparece en la salida: solo su id (`opk_<id>_…`).

## 5. Publicar un paquete como lo haría el pipeline

```bash
mkdir -p "$DEMO/src"
dotnet new classlib -n Hemia.Payments.Core -o "$DEMO/src/Hemia.Payments.Core"
dotnet pack "$DEMO/src/Hemia.Payments.Core" -c Release -o "$DEMO/out" -p:Version=1.0.0
onepack --token-env CI_TOKEN package push --feed payments "$DEMO"/out/*.nupkg --skip-existing-identical
onepack package list --feed payments
```

`--skip-existing-identical` hace que reintentar el pipeline con el mismo paquete no falle. Con una versión ya publicada y contenido distinto, falla (salida 6): las versiones son inmutables.

## 6. Preparar el proyecto consumidor

```bash
dotnet new console -n Consumer -o "$DEMO/consumer"
cd "$DEMO/consumer"
onepack nuget init --feed payments --pattern 'Hemia.Payments.*' --dry-run
onepack nuget init --feed payments --pattern 'Hemia.Payments.*' --yes
cat NuGet.Config
dotnet add package Hemia.Payments.Core --version 1.0.0 --no-restore
```

`--dry-run` muestra el diff sin escribir. El `NuGet.Config` resultante declara la fuente `onepack_payments` y un `packageSourceMapping`: los ids `Hemia.Payments.*` solo se resuelven desde onepack y el resto sigue en nuget.org. **No contiene ningún secreto**, así que se puede subir al repositorio.

## 7. Restaurar en local

```bash
onepack exec --feed payments -- dotnet restore
dotnet build --no-restore
```

`onepack exec` define la credencial solo en el entorno de `dotnet restore`. Al terminar no queda ni en el shell ni en disco:

```bash
env | grep NuGetPackageSourceCredentials || echo "sin credenciales en el shell"
grep -c Password NuGet.Config || true
```

## 8. Restaurar como un agente de CI

En CI no se usa `onepack`: NuGet lee la credencial de la variable `NuGetPackageSourceCredentials_<fuente>`, que el pipeline rellena desde un secreto. Simúlalo con una caché vacía y solo el token de CI:

```bash
cd "$DEMO/consumer"
rm -rf obj bin
env -u ONEPACK_TOKEN \
  NUGET_PACKAGES="$(mktemp -d)" \
  NUGET_HTTP_CACHE_PATH="$(mktemp -d)" \
  NuGetPackageSourceCredentials_onepack_payments="Username=ci;Password=$CI_TOKEN" \
  dotnet restore
dotnet build --no-restore
```

El restore descarga `Hemia.Payments.Core 1.0.0` desde onepack con la credencial de CI.

En GitHub Actions, con el servidor accesible por HTTPS y el token guardado como secreto `ONEPACK_TOKEN`:

```yaml
jobs:
  build:
    runs-on: ubuntu-24.04
    env:
      NuGetPackageSourceCredentials_onepack_payments: "Username=ci;Password=${{ secrets.ONEPACK_TOKEN }}"
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-dotnet@v4
        with:
          dotnet-version: 8.0.x
      - run: dotnet restore
      - run: dotnet build --no-restore
```

Para publicar desde el pipeline sirve tanto `dotnet nuget push out/*.nupkg --source onepack_payments --api-key "$ONEPACK_TOKEN"` como `onepack --url https://packages.example.com package push --feed payments out/*.nupkg --skip-existing-identical` (`onepack` lee `ONEPACK_TOKEN`).

## 9. Comprobar y auditar

```bash
cd "$DEMO/consumer"
onepack --token-env CI_TOKEN doctor --feed payments --require publish
onepack audit list --limit 10
```

Ahora `doctor` sale sin fallos, también en `nuget-config` y `source-mapping`. La auditoría muestra la creación del feed, del principal, del token, el grant y la publicación, con el actor de cada una.

## 10. Revocar y limpiar

```bash
onepack token list --principal ci-payments
onepack token revoke <ID> --yes            # el ID es la primera columna
onepack --token-env CI_TOKEN whoami; echo "salida: $?"   # 3: credencial revocada
```

Para terminar, detén `onepackd` (Ctrl+C en su terminal) y borra el directorio de la prueba:

```bash
onepack logout                      # solo si usaste `login` (con ONEPACK_TOKEN: unset ONEPACK_TOKEN)
onepack context remove local --yes
rm -rf "$DEMO"
```

## Si algo falla

- `onepack doctor --feed payments` indica qué comprobación falla y qué hacer.
- Cada error del CLI incluye un código estable, una acción sugerida y un `request_id` que también aparece en el log de `onepackd`. Los códigos de salida están en [docs/cli.md](cli.md#códigos-de-salida).
