#!/usr/bin/env bash
# Prueba de recuperación de la Fase 7 (ADR-017): backup en un servidor, restore en otro y
# `dotnet restore` contra el restaurado.
#
#   e2e-recovery.sh backup  ARCHIVO.tar.gz   origen: publica paquetes y hace el backup en caliente
#   e2e-recovery.sh restore ARCHIVO.tar.gz   destino: restaura, comprueba y restaura un proyecto
#   e2e-recovery.sh                          ambas fases en esta máquina
#
# En CI cada fase corre en una máquina distinta (Linux -> macOS).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
conformance="$root/tests/conformance-dotnet"
mode=${1:-all}
archive=${2:-}
feed=e2e

work=$(mktemp -d)
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
  echo "E2E RECUPERACIÓN FALLÓ: $*" >&2
  cat "$work"/server-*.log >&2 2>/dev/null || true
  exit 1
}
step() { echo "==> $*"; }
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

step "Compilando onepackd y onepack"
cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server -p onepack-cli
onepackd() { "$root/target/debug/onepackd" "$@"; }
onepack() { "$root/target/debug/onepack" "$@"; }
export ONEPACK_CONFIG_DIR="$work/onepack-config" ONEPACK_KEYRING=off ONEPACK_NO_INPUT=1
unset ONEPACK_TOKEN

# Arranca onepackd y espera a /readyz. Uso: serve DIR PUERTO
serve() {
  # El binario directamente (no la función): `$!` debe ser el PID del servidor.
  "$root/target/debug/onepackd" serve --data-dir "$1" --listen "127.0.0.1:$2" --public-url "http://127.0.0.1:$2" \
    --ops-listen "127.0.0.1:$(($2 + 1000))" --min-free-space-mib 0 >"$work/server-$2.log" 2>&1 &
  server_pid=$!
  for _ in $(seq 1 50); do
    curl -fs "http://127.0.0.1:$2/readyz" >/dev/null 2>&1 && return 0
    kill -0 "$server_pid" 2>/dev/null || fail "onepackd terminó al arrancar"
    sleep 0.2
  done
  fail "onepackd no llegó a ready"
}
stop() { kill "$server_pid"; wait "$server_pid" 2>/dev/null || true; server_pid=""; }

origin() {
  local out=$1 port=5896
  local data="$work/origin"
  cp "$conformance/global.json" "$work/global.json"
  [ -n "${ONEPACK_E2E_SDK:-}" ] && printf '{ "sdk": { "version": "%s", "rollForward": "latestFeature" } }\n' "$ONEPACK_E2E_SDK" >"$work/global.json"
  cp -R "$conformance/fixtures" "$work/fixtures"
  rm -rf "$work"/fixtures/*/bin "$work"/fixtures/*/obj

  step "Origen: inicializando y publicando paquetes"
  onepackd init --data-dir "$data" >/dev/null
  ADMIN_TOKEN=$(tr -d '\n' <"$data/initial-admin-token")
  export ADMIN_TOKEN
  serve "$data" $port
  onepack context add origin --url "http://127.0.0.1:$port" >/dev/null 2>&1
  admin() { onepack --context origin --token-env ADMIN_TOKEN "$@"; }
  admin feed create $feed >/dev/null
  admin principal create ci --kind service >/dev/null
  admin grant add --principal ci --feed $feed --role publisher >/dev/null
  CI_TOKEN=$(admin token create --principal ci --expires-in-days 1 2>/dev/null)
  export CI_TOKEN
  pack() { (cd "$work" && dotnet pack "fixtures/$1" -c Release -o "$work/nupkgs" --nologo -v quiet "${@:2}") >/dev/null || fail "pack de $1"; }
  pack Onepack.Fixture.Basic
  pack Onepack.Fixture.Basic -p:Version=1.1.0
  pack Onepack.Fixture.Dependent
  pack Onepack.Fixture.Rich
  pack Onepack.Fixture.Ranged "-p:RestoreAdditionalProjectSources=$work/nupkgs"
  onepack --context origin --token-env CI_TOKEN package push --feed $feed "$work"/nupkgs/*.nupkg >/dev/null 2>&1 \
    || fail "push en el origen"

  step "Origen: backup con el servidor en marcha"
  onepackd backup --data-dir "$data" --output "$work/backup" || fail "backup"
  curl -fs "http://127.0.0.1:$((port + 1000))/metrics" | grep -q '^onepack_maintenance 0' \
    || fail "el mantenimiento no terminó tras el backup"
  admin feed create after-backup >/dev/null || fail "el servidor no acepta mutaciones tras el backup"
  onepackd check --data-dir "$data" >/dev/null || fail "check en el origen"
  stop

  # Lo que el destino necesita para comprobar: el backup, los hashes publicados y el token de
  # CI (de prueba, caduca en un día) para demostrar que las credenciales sobreviven.
  (cd "$work/nupkgs" && for f in *.nupkg; do echo "$(sha "$f") $f"; done) >"$work/backup/expected.sha256"
  printf %s "$CI_TOKEN" >"$work/backup/ci-token.txt"
  tar -czf "$out" -C "$work" backup
  echo "    backup: $(du -h "$out" | cut -f1) en $out"
}

target() {
  local in=$1 port=5897
  local data="$work/restored"
  mkdir -p "$work/in"
  tar -xzf "$in" -C "$work/in"
  local backup="$work/in/backup"

  step "Destino: restaurando en un directorio vacío"
  onepackd restore --from "$backup" --data-dir "$data" || fail "restore"
  onepackd check --data-dir "$data" || fail "check tras restaurar"
  if onepackd restore --from "$backup" --data-dir "$data" >/dev/null 2>&1; then
    fail "restore debió negarse a escribir sobre un directorio con datos"
  fi

  step "Destino: arrancando con otra URL pública"
  serve "$data" $port
  CI_TOKEN=$(cat "$backup/ci-token.txt")
  export CI_TOKEN
  onepack --url "http://127.0.0.1:$port" --token-env CI_TOKEN whoami >/dev/null \
    || fail "la credencial de CI no sobrevivió al restore"

  step "Destino: dotnet restore de un proyecto contra el servidor restaurado"
  mkdir -p "$work/client"
  cp "$conformance/global.json" "$work/global.json"
  [ -n "${ONEPACK_E2E_SDK:-}" ] && printf '{ "sdk": { "version": "%s", "rollForward": "latestFeature" } }\n' "$ONEPACK_E2E_SDK" >"$work/global.json"
  cp -R "$conformance/consumer" "$work/client/consumer"
  rm -rf "$work/client/consumer/bin" "$work/client/consumer/obj"
  sdk_version=$(cd "$work" && dotnet --version)
  tfm="net${sdk_version%%.*}.0"
  printf '<?xml version="1.0" encoding="utf-8"?>\n<configuration>\n  <packageSources>\n    <clear />\n  </packageSources>\n</configuration>\n' \
    >"$work/client/NuGet.Config"
  (cd "$work/client" && onepack --url "http://127.0.0.1:$port" nuget init --feed $feed --pattern '*' --yes) >/dev/null 2>&1 \
    || fail "nuget init"
  export NUGET_PACKAGES="$work/packages" NUGET_HTTP_CACHE_PATH="$work/http-cache"
  (cd "$work/client/consumer" && onepack --url "http://127.0.0.1:$port" --token-env CI_TOKEN exec --feed $feed -- \
    dotnet restore --nologo "-p:ConsumerTargetFramework=$tfm") >"$work/restore.log" 2>&1 \
    || { cat "$work/restore.log"; fail "dotnet restore contra el servidor restaurado"; }
  output=$(cd "$work/client/consumer" && dotnet run --no-restore --nologo "-p:ConsumerTargetFramework=$tfm")
  expected=$'dependent -> hello from onepack fixture\nranged -> hello from onepack fixture\nrich'
  [ "$output" = "$expected" ] || fail "salida inesperada: '$output'"

  step "Destino: los paquetes descargados son idénticos a los publicados en el origen"
  while read -r hash file; do
    lower=$(echo "$file" | tr '[:upper:]' '[:lower:]')
    found=$(find "$NUGET_PACKAGES" -name "$lower" | head -1)
    [ -n "$found" ] || continue
    [ "$(sha "$found")" = "$hash" ] || fail "$file restaurado no coincide con el original"
    checked=$((${checked:-0} + 1))
  done <"$backup/expected.sha256"
  [ "${checked:-0}" -ge 4 ] || fail "solo se compararon ${checked:-0} paquetes"
  stop
  echo "    $checked paquetes idénticos byte a byte"
}

case "$mode" in
  backup) [ -n "$archive" ] || fail "uso: $0 backup ARCHIVO.tar.gz"; origin "$archive" ;;
  restore) [ -n "$archive" ] || fail "uso: $0 restore ARCHIVO.tar.gz"; target "$archive" ;;
  all)
    origin "$work/backup.tar.gz"
    rm -rf "$work/origin" "$work/backup" "$work/nupkgs"
    target "$work/backup.tar.gz"
    ;;
  *) fail "modo desconocido: $mode" ;;
esac
echo "E2E RECUPERACIÓN OK ($mode)"
