#!/usr/bin/env bash
# Prueba de punta a punta con clientes .NET reales (Fase 1, ADR-019):
# dotnet pack -> dotnet nuget push -> onepackd -> dotnet restore (caché aislada) -> dotnet run.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
conformance="$root/tests/conformance-dotnet"
port=${ONEPACK_E2E_PORT:-5899}
feed=e2e
base="http://127.0.0.1:$port"
source_url="$base/nuget/$feed/v3/index.json"

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
  echo "E2E FALLÓ: $*" >&2
  echo "--- log de onepackd ---" >&2
  cat "$work/server.log" >&2 || true
  exit 1
}

step() { echo "==> $*"; }

step "Compilando onepackd"
cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server

step "Inicializando el directorio de datos"
"$root/target/debug/onepackd" migrate --data-dir "$work/data" >/dev/null
"$root/target/debug/onepackd" feed create "$feed" --data-dir "$work/data" >/dev/null

step "Iniciando onepackd en $base"
"$root/target/debug/onepackd" serve \
  --listen "127.0.0.1:$port" \
  --data-dir "$work/data" \
  --public-url "$base" \
  --min-free-space-mib 0 >"$work/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 50); do
  curl -fsS "$source_url" >/dev/null 2>&1 && break
  kill -0 "$server_pid" 2>/dev/null || fail "onepackd terminó al arrancar"
  sleep 0.2
done
curl -fsS "$source_url" >/dev/null || fail "onepackd no respondió en $source_url"

step "Empaquetando fixtures con dotnet pack"
for proj in "$conformance"/fixtures/*/*.csproj; do
  dotnet pack "$proj" -c Release -o "$work/nupkgs" --nologo -v quiet
done

# Configuración aislada: solo onepackd como origen. HTTP solo porque es loopback de prueba.
cp "$conformance/global.json" "$work/global.json"
cat >"$work/NuGet.Config" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="onepack" value="$source_url" protocolVersion="3" allowInsecureConnections="true" />
  </packageSources>
</configuration>
EOF

step "Publicando con dotnet nuget push"
for pkg in "$work"/nupkgs/*.nupkg; do
  (cd "$work" && dotnet nuget push "$pkg" --source onepack --api-key e2e-spike) || fail "push de $(basename "$pkg")"
done

step "Comprobando que una versión duplicada se rechaza (409)"
dup="$work/nupkgs/Onepack.Fixture.Basic.1.0.0.nupkg"
if (cd "$work" && dotnet nuget push "$dup" --source onepack --api-key e2e-spike >"$work/dup.log" 2>&1); then
  fail "el push duplicado debió fallar"
fi
grep -q "409" "$work/dup.log" || fail "el push duplicado no devolvió 409: $(cat "$work/dup.log")"

step "Restaurando el consumidor con caché vacía"
cp -R "$conformance/consumer" "$work/consumer"
export NUGET_PACKAGES="$work/packages"
export NUGET_HTTP_CACHE_PATH="$work/http-cache"
(cd "$work/consumer" && dotnet restore --configfile "$work/NuGet.Config" --nologo) || fail "dotnet restore"

step "Verificando bytes restaurados"
for id in onepack.fixture.basic onepack.fixture.dependent; do
  pushed=$(ls "$work"/nupkgs/*.nupkg | grep -i "/$id.1.0.0.nupkg")
  restored="$NUGET_PACKAGES/$id/1.0.0/$id.1.0.0.nupkg"
  [ -f "$restored" ] || fail "$id no se restauró"
  cmp -s "$pushed" "$restored" || fail "$id restaurado no coincide byte a byte con el publicado"
done

step "Ejecutando el consumidor"
output=$(cd "$work/consumer" && dotnet run --no-restore --nologo)
expected="dependent -> hello from onepack fixture"
[ "$output" = "$expected" ] || fail "salida inesperada: '$output'"

echo "E2E OK: push, 409 en duplicado, restore transitivo con caché vacía y bytes idénticos."
