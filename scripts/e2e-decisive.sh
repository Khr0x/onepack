#!/usr/bin/env bash
# Prueba decisiva del MVP (Fase 8), de punta a punta y en orden:
#
#   Un equipo instala el servidor, configura su proyecto, publica una librería y la restaura
#   desde CI; después revoca una credencial y recupera el servicio desde un backup, sin abrir
#   una interfaz web ni editar manualmente la base de datos.
#
# Solo usa `onepackd` (instalación, backup y restore), `onepack` y `dotnet`. Nunca `sqlite3`.
# Incluye el ejercicio de incidente del piloto: revocar la credencial de CI, bloquear una
# versión y restaurar desde un backup.
#
# Variables:
#   ONEPACK_BIN_DIR=DIR       usa los binarios `onepackd` y `onepack` de DIR (p. ej., los de
#                             una release descargada) en lugar de compilarlos.
#   ONEPACK_E2E_SDK=10.0.100  SDK para global.json (por defecto, el de tests/conformance-dotnet).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
conformance="$root/tests/conformance-dotnet"
port=${ONEPACK_E2E_DECISIVE_PORT:-5899}
base="http://127.0.0.1:$port"
feed=libs

work=$(mktemp -d)
data="$work/srv/data"
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
  echo "PRUEBA DECISIVA FALLÓ: $*" >&2
  echo "--- log de onepackd ---" >&2
  cat "$work"/server-*.log >&2 2>/dev/null || true
  exit 1
}
step() { echo "==> $*"; }
json() { python3 -c "import json, sys; print(json.load(sys.stdin)$1)"; }
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

# --- 1. Instalar el servidor -------------------------------------------------------------------

if [ -n "${ONEPACK_BIN_DIR:-}" ]; then
  bin=$(cd "$ONEPACK_BIN_DIR" && pwd)
  step "Usando los binarios de $bin"
else
  step "Compilando onepackd y onepack"
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server -p onepack-cli
  bin="$root/target/debug"
fi
[ -x "$bin/onepackd" ] && [ -x "$bin/onepack" ] || fail "faltan onepackd u onepack en $bin"
onepackd() { "$bin/onepackd" "$@"; }
onepack() { "$bin/onepack" "$@"; }
step "onepackd $("$bin/onepackd" --version | awk '{print $2}'), onepack $("$bin/onepack" --version | awk '{print $2}')"

# CLI aislado, sin keychain ni preguntas, como en un agente de CI.
export ONEPACK_CONFIG_DIR="$work/onepack-config" ONEPACK_KEYRING=off ONEPACK_NO_INPUT=1
unset ONEPACK_TOKEN

# Arranca onepackd siempre con la misma URL pública y espera a /readyz.
serve() {
  "$bin/onepackd" serve --data-dir "$data" --listen "127.0.0.1:$port" --public-url "$base" \
    --min-free-space-mib 0 >"$work/server-$1.log" 2>&1 &
  server_pid=$!
  for _ in $(seq 1 50); do
    curl -fs "$base/readyz" >/dev/null 2>&1 && return 0
    kill -0 "$server_pid" 2>/dev/null || fail "onepackd terminó al arrancar"
    sleep 0.2
  done
  fail "onepackd no llegó a ready"
}
stop() { kill "$server_pid"; wait "$server_pid" 2>/dev/null || true; server_pid=""; }

step "Instalando: init y arranque"
onepackd init --data-dir "$data" >/dev/null
ADMIN_TOKEN=$(tr -d '\n' <"$data/initial-admin-token")
export ADMIN_TOKEN
serve install

onepack context add registry --url "$base" >/dev/null
admin() { onepack --token-env ADMIN_TOKEN "$@"; }
[ "$(admin whoami --json | json '["administrator"]')" = "True" ] || fail "whoami del administrador"

step "Creando el feed, la cuenta de CI y su token"
admin feed create "$feed" >/dev/null
admin principal create ci --kind service >/dev/null
admin grant add --principal ci --feed "$feed" --role publisher --publish-pattern 'Onepack.Fixture.*' >/dev/null
new_ci_token() { admin token create --principal ci --name "$1" --expires-in-days 1 2>/dev/null; }
CI_TOKEN=$(new_ci_token pipeline)
export CI_TOKEN
[[ "$CI_TOKEN" == opk_* ]] || fail "token create no devolvió un token"

# --- 2. Configurar el proyecto -----------------------------------------------------------------

if [ -n "${ONEPACK_E2E_SDK:-}" ]; then
  printf '{ "sdk": { "version": "%s", "rollForward": "latestFeature" } }\n' "$ONEPACK_E2E_SDK" >"$work/global.json"
else
  cp "$conformance/global.json" "$work/global.json"
fi
mkdir -p "$work/lib" "$work/app"
cp -R "$conformance/fixtures" "$work/lib/fixtures"
cp -R "$conformance/consumer" "$work/app/consumer"
rm -rf "$work"/lib/fixtures/*/bin "$work"/lib/fixtures/*/obj "$work"/app/consumer/bin "$work"/app/consumer/obj
sdk_version=$(cd "$work" && dotnet --version)
tfm="net${sdk_version%%.*}.0"

step "Configurando el proyecto con onepack nuget init (SDK $sdk_version)"
printf '<?xml version="1.0" encoding="utf-8"?>\n<configuration>\n  <packageSources>\n    <clear />\n  </packageSources>\n</configuration>\n' \
  >"$work/app/NuGet.Config"
(cd "$work/app" && onepack nuget init --feed "$feed" --pattern '*' --yes) >/dev/null 2>&1 || fail "nuget init"

# --- 3. Publicar una librería ------------------------------------------------------------------

step "Publicando la librería desde CI"
pack() { (cd "$work" && dotnet pack "lib/fixtures/$1" -c Release -o "$work/nupkgs" --nologo -v quiet "${@:2}") >/dev/null || fail "pack de $1"; }
pack Onepack.Fixture.Basic
pack Onepack.Fixture.Basic -p:Version=1.1.0
pack Onepack.Fixture.Dependent
pack Onepack.Fixture.Rich
pack Onepack.Fixture.Ranged "-p:RestoreAdditionalProjectSources=$work/nupkgs"
onepack --token-env CI_TOKEN package push --feed "$feed" "$work"/nupkgs/*.nupkg >/dev/null 2>"$work/push.log" \
  || { cat "$work/push.log"; fail "package push"; }

# --- 4. Restaurar desde CI ---------------------------------------------------------------------

# Cada restore simula un agente de CI limpio: sin caché de NuGet ni obj/ previos.
# Uso: ci_restore NOMBRE  (deja el log en $work/restore-NOMBRE.log)
ci_restore() {
  rm -rf "$work/app/consumer/obj" "$work/app/consumer/bin"
  export NUGET_PACKAGES="$work/cache-$1/packages" NUGET_HTTP_CACHE_PATH="$work/cache-$1/http"
  (cd "$work/app/consumer" && onepack --token-env CI_TOKEN exec --feed "$feed" -- \
    dotnet restore --nologo "-p:ConsumerTargetFramework=$tfm") >"$work/restore-$1.log" 2>&1
}
run_app() {
  output=$(cd "$work/app/consumer" && dotnet run --no-restore --nologo "-p:ConsumerTargetFramework=$tfm")
  expected=$'dependent -> hello from onepack fixture\nranged -> hello from onepack fixture\nrich'
  [ "$output" = "$expected" ] || fail "salida inesperada: '$output'"
}

step "Restaurando desde CI con caché vacía"
ci_restore first || { cat "$work/restore-first.log"; fail "dotnet restore desde CI"; }
run_app

# --- 5. Incidente: revocar la credencial de CI -------------------------------------------------

step "Incidente 1: el token de CI se filtra y se revoca"
token_id=$(admin token list --principal ci --json | json '[0]["id"]')
admin token revoke "$token_id" --yes >/dev/null
set +e
ci_restore revoked
code=$?
set -e
# onepack exec comprueba la credencial antes de lanzar dotnet: error claro, sin NU1301.
[ "$code" = "3" ] || { cat "$work/restore-revoked.log"; fail "código $code con el token revocado (esperado 3)"; }
grep -q "AUTH_" "$work/restore-revoked.log" && ! grep -q "NU1301" "$work/restore-revoked.log" \
  || { cat "$work/restore-revoked.log"; fail "el error no explica que la credencial es inválida"; }

step "Rotando: token nuevo para el pipeline"
CI_TOKEN=$(new_ci_token pipeline-rotated)
export CI_TOKEN
ci_restore rotated || { cat "$work/restore-rotated.log"; fail "restore con el token rotado"; }

# --- 6. Incidente: bloquear una versión --------------------------------------------------------

step "Incidente 2: Onepack.Fixture.Basic 1.1.0 tiene una vulnerabilidad y se bloquea"
admin package block --feed "$feed" Onepack.Fixture.Basic 1.1.0 --reason "drill: CVE ficticio" --yes >/dev/null
if ci_restore blocked; then fail "el restore descargó una versión bloqueada"; fi
grep -q "PACKAGE_BLOCKED" "$work/restore-blocked.log" \
  || { cat "$work/restore-blocked.log"; fail "el error no explica el bloqueo"; }
admin package unblock --feed "$feed" Onepack.Fixture.Basic 1.1.0 --reason "drill: parche publicado" >/dev/null
ci_restore unblocked || { cat "$work/restore-unblocked.log"; fail "restore tras desbloquear"; }

# --- 7. Incidente: perder el servidor y recuperar desde backup ---------------------------------

step "Incidente 3: backup en caliente y pérdida total del directorio de datos"
onepackd backup --data-dir "$data" --output "$work/backup" >/dev/null || fail "backup"
(cd "$work/nupkgs" && for f in *.nupkg; do echo "$(sha "$f") $f"; done) >"$work/expected.sha256"
stop
rm -rf "$data"
if curl -fs "$base/readyz" >/dev/null 2>&1; then fail "el servidor sigue respondiendo"; fi

step "Recuperando: restore, check y arranque con la misma URL"
onepackd restore --from "$work/backup" --data-dir "$data" >/dev/null || fail "restore"
onepackd check --data-dir "$data" >/dev/null || fail "check tras restaurar"
serve recovered

step "Verificando el servicio recuperado"
ci_restore recovered || { cat "$work/restore-recovered.log"; fail "restore desde CI tras recuperar"; }
run_app
checked=0
while read -r hash file; do
  found=$(find "$NUGET_PACKAGES" -name "$(echo "$file" | tr '[:upper:]' '[:lower:]')" | head -1)
  [ -n "$found" ] || continue
  [ "$(sha "$found")" = "$hash" ] || fail "$file recuperado no coincide con el publicado"
  checked=$((checked + 1))
done <"$work/expected.sha256"
[ "$checked" -ge 4 ] || fail "solo se compararon $checked paquetes"

# El estado de seguridad sobrevive al restore: el token revocado sigue revocado.
revoked=$(admin token list --principal ci --json | python3 -c '
import json, sys
print(sum(1 for t in json.load(sys.stdin) if t.get("revoked_at")))')
[ "$revoked" = "1" ] || fail "el token revocado no sigue revocado tras el restore ($revoked)"

step "Comprobando la auditoría del incidente"
actions=$(admin audit list --feed "$feed" --limit 100 --json | python3 -c '
import json, sys
print(" ".join(sorted({e["action"] for e in json.load(sys.stdin)})))')
for a in package.block package.unblock; do
  [[ " $actions " == *" $a "* ]] || fail "falta $a en la auditoría ($actions)"
done
admin audit list --action token.revoke --json | grep -q token.revoke || fail "falta token.revoke en la auditoría"

step "Comprobando que ningún secreto quedó en disco ni en logs"
for token in "$CI_TOKEN" "$ADMIN_TOKEN"; do
  secret=${token##*_}
  grep -rqF "$secret" "$work/app" "$ONEPACK_CONFIG_DIR" && fail "un token quedó escrito en el proyecto o en la configuración"
  grep -qF "$secret" "$work"/server-*.log && fail "un token apareció en el log del servidor"
done
stop

echo "PRUEBA DECISIVA OK (SDK $sdk_version): instalar, configurar, publicar, restaurar desde CI,"
echo "  revocar y rotar la credencial, bloquear una versión y recuperar desde backup;"
echo "  $checked paquetes idénticos byte a byte, sin UI ni acceso a la base de datos."
