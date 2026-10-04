#!/usr/bin/env bash
# Prueba de punta a punta de la Fase 6 (ADR-015, ADR-016): registro operado sin UI.
# Tras el arranque (`onepackd init`), todo se hace con `onepack` y `dotnet`:
# feed -> principal de CI -> token -> grant -> push -> nuget init -> exec dotnet restore -> run.
#
# Variables:
#   ONEPACK_E2E_SDK=10.0.100  SDK para global.json (por defecto, el de tests/conformance-dotnet).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
conformance="$root/tests/conformance-dotnet"
port=${ONEPACK_E2E_CLI_PORT:-5898}
base="http://127.0.0.1:$port"
feed=e2e

work=$(mktemp -d)
client="$work/client"
mkdir -p "$work/src" "$client"
server_pid=""
cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

fail() {
  echo "E2E CLI FALLÓ: $*" >&2
  echo "--- log de onepackd ---" >&2
  cat "$work/server.log" >&2 || true
  exit 1
}

step() { echo "==> $*"; }
json() { python3 -c "import json, sys; print(json.load(sys.stdin)$1)"; }

if [ -n "${ONEPACK_E2E_SDK:-}" ]; then
  printf '{ "sdk": { "version": "%s", "rollForward": "latestFeature" } }\n' "$ONEPACK_E2E_SDK" >"$work/global.json"
else
  cp "$conformance/global.json" "$work/global.json"
fi
cp -R "$conformance/fixtures" "$work/src/fixtures"
cp -R "$conformance/consumer" "$client/consumer"
rm -rf "$work"/src/fixtures/*/bin "$work"/src/fixtures/*/obj "$client"/consumer/bin "$client"/consumer/obj
sdk_version=$(cd "$work" && dotnet --version)
tfm="net${sdk_version%%.*}.0"
step "SDK .NET $sdk_version (consumidor: $tfm)"

step "Compilando onepackd y onepack"
cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server -p onepack-cli
onepack() { "$root/target/debug/onepack" "$@"; }

# CLI aislado: configuración propia y sin keychain (como en un agente de CI).
export ONEPACK_CONFIG_DIR="$work/onepack-config" ONEPACK_KEYRING=off ONEPACK_NO_INPUT=1
export NUGET_PACKAGES="$work/packages" NUGET_HTTP_CACHE_PATH="$work/http-cache"
unset ONEPACK_TOKEN

# --- Arranque del servidor (única vez que se usa onepackd) -----------------------------------

step "Inicializando y arrancando onepackd"
"$root/target/debug/onepackd" init --data-dir "$work/data" >/dev/null
ADMIN_TOKEN=$(tr -d '\n' <"$work/data/initial-admin-token")
export ADMIN_TOKEN
"$root/target/debug/onepackd" serve --listen "127.0.0.1:$port" --data-dir "$work/data" \
  --public-url "$base" --min-free-space-mib 0 >"$work/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 50); do
  curl -s -o /dev/null "$base/api/v1/capabilities" && break
  kill -0 "$server_pid" 2>/dev/null || fail "onepackd terminó al arrancar"
  sleep 0.2
done

# --- Administración solo con onepack ---------------------------------------------------------

step "Configurando el contexto y comprobando la identidad"
onepack context add local --url "$base" >/dev/null
admin() { onepack --token-env ADMIN_TOKEN "$@"; }
[ "$(admin whoami --json | json '["administrator"]')" = "True" ] || fail "whoami del administrador"

step "Creando feed, cuenta de servicio de CI, token y grant"
admin feed create "$feed" >/dev/null
admin principal create ci --kind service >/dev/null
CI_TOKEN=$(admin token create --principal ci --name e2e --expires-in-days 1 2>/dev/null)
export CI_TOKEN
[[ "$CI_TOKEN" == opk_* ]] || fail "token create no devolvió un token"
admin grant add --principal ci --feed "$feed" --role publisher --publish-pattern 'Onepack.Fixture.*' >/dev/null
admin principal create dev --kind user >/dev/null
DEV_TOKEN=$(admin token create --principal dev --expires-in-days 1 2>/dev/null)
export DEV_TOKEN
admin grant add --principal dev --feed "$feed" --role reader >/dev/null
ci() { onepack --token-env CI_TOKEN "$@"; }

step "Empaquetando fixtures con dotnet pack"
pack() { (cd "$work" && dotnet pack "src/fixtures/$1" -c Release -o "$work/nupkgs" --nologo -v quiet "${@:2}") >/dev/null || fail "pack de $1"; }
pack Onepack.Fixture.Basic
pack Onepack.Fixture.Basic -p:Version=1.1.0
pack Onepack.Fixture.Basic -p:Version=2.0.0
pack Onepack.Fixture.Dependent
pack Onepack.Fixture.Rich
pack Onepack.Fixture.Ranged "-p:RestoreAdditionalProjectSources=$work/nupkgs"

step "Publicando con onepack package push"
ci package push --feed "$feed" "$work"/nupkgs/*.nupkg 2>"$work/push.log" >/dev/null \
  || { cat "$work/push.log"; fail "package push"; }
skipped=$(ci package push --feed "$feed" "$work"/nupkgs/*.nupkg --skip-existing-identical --json \
  | python3 -c 'import json, sys; print(sum(r["status"] == "skipped" for r in json.load(sys.stdin)))')
[ "$skipped" = "6" ] || fail "--skip-existing-identical debió omitir los 6 paquetes ($skipped)"
[ "$(ci package list --feed "$feed" --json | json '.__len__()')" = "4" ] || fail "package list"

# --- Configuración de NuGet y restore --------------------------------------------------------

step "Generando NuGet.Config con onepack nuget init (sin secretos)"
cat >"$client/NuGet.Config" <<'EOF'
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <!-- configuración previa del equipo -->
  <packageSources>
    <clear />
    <add key="nuget.org" value="https://api.nuget.org/v3/index.json" protocolVersion="3" />
  </packageSources>
</configuration>
EOF
(cd "$client" && onepack nuget init --feed "$feed" --pattern 'Onepack.Fixture.*' --dry-run) >"$work/diff.txt"
grep -q '^+.*onepack_e2e' "$work/diff.txt" || fail "el diff no muestra la fuente nueva"
(cd "$client" && onepack nuget init --feed "$feed" --pattern 'Onepack.Fixture.*' --yes) >/dev/null 2>&1
grep -q 'configuración previa del equipo' "$client/NuGet.Config" || fail "nuget init perdió contenido previo"
grep -q 'api.nuget.org' "$client/NuGet.Config" || fail "nuget init perdió la fuente previa"
grep -q '<package pattern="Onepack.Fixture.\*" />' "$client/NuGet.Config" || fail "falta el source mapping"

step "Diagnóstico con onepack doctor"
(cd "$client" && onepack --token-env CI_TOKEN doctor --feed "$feed" --require publish) >"$work/doctor.txt" \
  || { cat "$work/doctor.txt"; fail "doctor con la credencial de CI"; }
if (cd "$client" && onepack --token-env DEV_TOKEN doctor --feed "$feed" --require publish --json) >"$work/doctor-dev.json"; then
  fail "doctor debió fallar sin permiso de publicación"
fi
grep -q AUTH_SCOPE_MISSING "$work/doctor-dev.json" || fail "doctor no reporta AUTH_SCOPE_MISSING"
grep -q -- "--role publisher" "$work/doctor-dev.json" || fail "doctor no sugiere la acción"
grep -q "${DEV_TOKEN##*_}" "$work/doctor-dev.json" && fail "doctor mostró el secreto"

step "Restaurando con onepack exec -- dotnet restore (caché vacía)"
(cd "$client/consumer" && onepack --token-env DEV_TOKEN exec --feed "$feed" -- \
  dotnet restore --nologo "-p:ConsumerTargetFramework=$tfm") >"$work/restore.log" 2>&1 \
  || { cat "$work/restore.log"; fail "exec dotnet restore"; }
resolved=$(python3 -c '
import json, sys
libs = json.load(open(sys.argv[1]))["libraries"]
print(" ".join(sorted(k for k in libs if k.startswith("Onepack.Fixture.Basic/"))))' "$client/consumer/obj/project.assets.json")
[ "$resolved" = "Onepack.Fixture.Basic/1.1.0" ] || fail "Basic resuelto como '$resolved'"

step "Ejecutando el consumidor"
output=$(cd "$client/consumer" && dotnet run --no-restore --nologo "-p:ConsumerTargetFramework=$tfm")
expected=$'dependent -> hello from onepack fixture\nranged -> hello from onepack fixture\nrich'
[ "$output" = "$expected" ] || fail "salida inesperada: '$output'"

step "Comprobando que el token no quedó en archivos ni en el entorno"
env | grep -q '^NuGetPackageSourceCredentials_' && fail "la credencial quedó en el entorno del shell"
for token in "$DEV_TOKEN" "$CI_TOKEN" "$ADMIN_TOKEN"; do
  secret=${token##*_}
  grep -rqF "$secret" "$client" "$ONEPACK_CONFIG_DIR" && fail "un token quedó escrito en disco"
  grep -qF "$secret" "$work/server.log" && fail "un token apareció en el log del servidor"
done

step "Revocando el token de CI"
token_id=$(admin token list --principal ci --json | json '[0]["id"]')
admin token revoke "$token_id" --yes >/dev/null
if ci whoami >/dev/null 2>&1; then fail "el token revocado sigue funcionando"; fi
set +e
ci whoami >/dev/null 2>&1
code=$?
set -e
[ "$code" = "3" ] || fail "código de salida $code con token revocado (esperado 3)"

echo "E2E CLI OK (SDK $sdk_version): feed, principal, token, grant, push, skip-existing-identical,"
echo "            nuget init, doctor, exec dotnet restore con source mapping, run y revocación."
