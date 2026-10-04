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

onepackd() { "$root/target/debug/onepackd" "$@" --data-dir "$work/data"; }

step "Inicializando el directorio de datos y una cuenta de servicio de CI"
onepackd init >/dev/null
onepackd feed create "$feed" >/dev/null
onepackd principal create ci --kind service >/dev/null
onepackd grant set --principal ci --feed "$feed" --role publisher --publish-pattern "Onepack.Fixture.*" >/dev/null
token=$(onepackd token create --principal ci --expires-in-days 1 2>/dev/null)
token_id=${token:4:16}

step "Iniciando onepackd en $base"
"$root/target/debug/onepackd" serve \
  --listen "127.0.0.1:$port" \
  --data-dir "$work/data" \
  --public-url "$base" \
  --min-free-space-mib 0 >"$work/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 50); do
  curl -fsS -H "X-NuGet-ApiKey: $token" "$source_url" >/dev/null 2>&1 && break
  kill -0 "$server_pid" 2>/dev/null || fail "onepackd terminó al arrancar"
  sleep 0.2
done
curl -fsS -H "X-NuGet-ApiKey: $token" "$source_url" >/dev/null || fail "onepackd no respondió en $source_url"

step "Comprobando que sin credenciales se rechaza (401)"
[ "$(curl -s -o /dev/null -w '%{http_code}' "$source_url")" = "401" ] || fail "el feed respondió sin credenciales"

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

# Credenciales por variable de entorno, sin escribir secretos en NuGet.Config (ADR-016).
export NuGetPackageSourceCredentials_onepack="Username=ci;Password=$token"

step "Publicando con dotnet nuget push"
for pkg in "$work"/nupkgs/*.nupkg; do
  (cd "$work" && dotnet nuget push "$pkg" --source onepack --api-key "$token") || fail "push de $(basename "$pkg")"
done

step "Comprobando que una versión duplicada se rechaza (409)"
dup="$work/nupkgs/Onepack.Fixture.Basic.1.0.0.nupkg"
if (cd "$work" && dotnet nuget push "$dup" --source onepack --api-key "$token" >"$work/dup.log" 2>&1); then
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

step "Revocando la credencial de CI"
onepackd token revoke "$token_id" >/dev/null
[ "$(curl -s -o /dev/null -w '%{http_code}' -H "X-NuGet-ApiKey: $token" "$source_url")" = "401" ] \
  || fail "el token revocado sigue funcionando"
if (cd "$work" && dotnet nuget push "$dup" --source onepack --api-key "$token" >"$work/revoked.log" 2>&1); then
  fail "el push con un token revocado debió fallar"
fi

grep -q "$token" "$work/server.log" && fail "el token apareció en el log del servidor"

echo "E2E OK: 401 sin credenciales, push con cuenta de servicio, 409 en duplicado, restore transitivo"
echo "        autenticado con caché vacía, bytes idénticos y token revocado rechazado."
