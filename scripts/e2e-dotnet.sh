#!/usr/bin/env bash
# Prueba de punta a punta con clientes .NET reales (ADR-019):
# dotnet pack -> dotnet nuget push -> onepackd -> search / restore / delete (unlist) -> run.
#
# Variables:
#   ONEPACK_E2E_SDK=10.0.100  SDK para global.json (por defecto, el de tests/conformance-dotnet).
#   ONEPACK_E2E_TLS=1         sirve onepackd detrás de nginx con TLS (solo Linux; usa sudo para
#                             confiar en una CA temporal).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
conformance="$root/tests/conformance-dotnet"
port=${ONEPACK_E2E_PORT:-5899}
tls_port=${ONEPACK_E2E_TLS_PORT:-8443}
tls=${ONEPACK_E2E_TLS:-0}
feed=e2e
if [ "$tls" = 1 ]; then base="https://localhost:$tls_port"; else base="http://127.0.0.1:$port"; fi
source_url="$base/nuget/$feed/v3/index.json"

work=$(mktemp -d)
src="$work/src"
client="$work/client"
mkdir -p "$src" "$client"
server_pid=""
cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [ -f "$work/nginx/nginx.pid" ]; then
    kill "$(cat "$work/nginx/nginx.pid")" 2>/dev/null || true
  fi
  if [ "$tls" = 1 ] && [ -f /usr/local/share/ca-certificates/onepack-e2e.crt ]; then
    sudo rm -f /usr/local/share/ca-certificates/onepack-e2e.crt
    sudo update-ca-certificates --fresh >/dev/null 2>&1 || true
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

# Ids de paquete presentes en una salida JSON de `dotnet package search`.
json_ids() {
  python3 -c '
import json, sys
def walk(node):
    if isinstance(node, dict):
        if isinstance(node.get("id"), str):
            print(node["id"])
        for value in node.values():
            walk(value)
    elif isinstance(node, list):
        for value in node:
            walk(value)
walk(json.load(sys.stdin))'
}

# --- SDK -------------------------------------------------------------------------------------

if [ -n "${ONEPACK_E2E_SDK:-}" ]; then
  printf '{ "sdk": { "version": "%s", "rollForward": "latestFeature" } }\n' "$ONEPACK_E2E_SDK" >"$work/global.json"
else
  cp "$conformance/global.json" "$work/global.json"
fi
cp -R "$conformance/fixtures" "$src/fixtures"
cp -R "$conformance/consumer" "$client/consumer"
rm -rf "$src"/fixtures/*/bin "$src"/fixtures/*/obj "$client"/consumer/bin "$client"/consumer/obj
sdk_version=$(cd "$work" && dotnet --version)
tfm="net${sdk_version%%.*}.0"
step "SDK .NET $sdk_version (consumidor: $tfm)$([ "$tls" = 1 ] && echo ', detrás de TLS')"

# --- Servidor --------------------------------------------------------------------------------

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

step "Iniciando onepackd (URL pública $base)"
# Trazas HTTP (método y ruta, sin cabeceras) para comprobar qué recursos usan los clientes.
RUST_LOG="info,tower_http=debug" "$root/target/debug/onepackd" serve \
  --listen "127.0.0.1:$port" \
  --data-dir "$work/data" \
  --public-url "$base" \
  --min-free-space-mib 0 >"$work/server.log" 2>&1 &
server_pid=$!

if [ "$tls" = 1 ]; then
  step "Configurando nginx con TLS en $base"
  [ "$(uname -s)" = Linux ] || fail "ONEPACK_E2E_TLS requiere Linux"
  command -v nginx >/dev/null || { sudo apt-get update -qq && sudo apt-get install -y -qq nginx >/dev/null; }
  pki="$work/pki"
  mkdir -p "$pki" "$work/nginx"
  openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=onepack-e2e-ca" \
    -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -keyout "$pki/ca.key" -out "$pki/ca.crt" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -subj "/CN=localhost" -keyout "$pki/tls.key" -out "$pki/tls.csr" 2>/dev/null
  printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n' >"$pki/ext"
  openssl x509 -req -in "$pki/tls.csr" -CA "$pki/ca.crt" -CAkey "$pki/ca.key" -CAcreateserial \
    -days 1 -extfile "$pki/ext" -out "$pki/tls.crt" 2>/dev/null
  sudo cp "$pki/ca.crt" /usr/local/share/ca-certificates/onepack-e2e.crt
  sudo update-ca-certificates >/dev/null
  cat >"$work/nginx/nginx.conf" <<EOF
worker_processes 1;
pid $work/nginx/nginx.pid;
error_log $work/nginx/error.log;
events {}
http {
  access_log off;
  client_body_temp_path $work/nginx/body;
  proxy_temp_path $work/nginx/proxy;
  fastcgi_temp_path $work/nginx/fastcgi;
  uwsgi_temp_path $work/nginx/uwsgi;
  scgi_temp_path $work/nginx/scgi;
  client_max_body_size 200m;
  server {
    listen 127.0.0.1:$tls_port ssl;
    ssl_certificate $pki/tls.crt;
    ssl_certificate_key $pki/tls.key;
    location / {
      proxy_pass http://127.0.0.1:$port;
      proxy_set_header Host \$host;
      proxy_set_header X-Forwarded-Proto https;
      proxy_request_buffering off;
    }
  }
}
EOF
  nginx -p "$work/nginx" -c "$work/nginx/nginx.conf" -e "$work/nginx/error.log"
fi

for _ in $(seq 1 50); do
  curl -fsS -H "X-NuGet-ApiKey: $token" "$source_url" >/dev/null 2>&1 && break
  kill -0 "$server_pid" 2>/dev/null || fail "onepackd terminó al arrancar"
  sleep 0.2
done
curl -fsS -H "X-NuGet-ApiKey: $token" "$source_url" >"$work/index.json" || fail "onepackd no respondió en $source_url"

step "Comprobando que sin credenciales se rechaza (401)"
[ "$(curl -s -o /dev/null -w '%{http_code}' "$source_url")" = "401" ] || fail "el feed respondió sin credenciales"

step "Comprobando que todas las URLs del service index usan la URL pública"
python3 - "$work/index.json" "$base/nuget/$feed/" <<'EOF' || fail "el service index tiene URLs que no parten de la URL pública"
import json, sys
resources = json.load(open(sys.argv[1]))["resources"]
bad = [r["@id"] for r in resources if not r["@id"].startswith(sys.argv[2])]
assert resources and not bad, bad
EOF

# --- Paquetes --------------------------------------------------------------------------------

step "Empaquetando fixtures con dotnet pack"
pack() { (cd "$work" && dotnet pack "src/fixtures/$1" -c Release -o "$work/nupkgs" --nologo -v quiet "${@:2}") || fail "pack de $1"; }
pack Onepack.Fixture.Basic
pack Onepack.Fixture.Basic -p:Version=1.1.0
pack Onepack.Fixture.Basic -p:Version=2.0.0
pack Onepack.Fixture.Dependent
pack Onepack.Fixture.Rich
pack Onepack.Fixture.Ranged "-p:RestoreAdditionalProjectSources=$work/nupkgs"

# Configuración aislada: solo onepackd como origen y sin secretos (ADR-016).
insecure=""
[ "$tls" = 1 ] || insecure=' allowInsecureConnections="true"'
cat >"$client/NuGet.Config" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="onepack" value="$source_url" protocolVersion="3"$insecure />
  </packageSources>
</configuration>
EOF
export NuGetPackageSourceCredentials_onepack="Username=ci;Password=$token"
export NUGET_PACKAGES="$work/packages"
export NUGET_HTTP_CACHE_PATH="$work/http-cache"

step "Publicando con dotnet nuget push"
for pkg in "$work"/nupkgs/*.nupkg; do
  (cd "$client" && dotnet nuget push "$pkg" --source onepack --api-key "$token") >/dev/null || fail "push de $(basename "$pkg")"
done

step "Comprobando que una versión duplicada se rechaza (409)"
dup="$work/nupkgs/Onepack.Fixture.Basic.1.0.0.nupkg"
if (cd "$client" && dotnet nuget push "$dup" --source onepack --api-key "$token" >"$work/dup.log" 2>&1); then
  fail "el push duplicado debió fallar"
fi
grep -q "409" "$work/dup.log" || fail "el push duplicado no devolvió 409: $(cat "$work/dup.log")"

# --- Búsqueda --------------------------------------------------------------------------------

search() { (cd "$client" && dotnet package search "$@" --source onepack --format json); }

step "Buscando con dotnet package search (sin y con prerelease)"
stable=$(search Onepack.Fixture | json_ids | sort -u | tr '\n' ' ')
[ "$stable" = "Onepack.Fixture.Basic Onepack.Fixture.Dependent Onepack.Fixture.Ranged " ] \
  || fail "búsqueda sin prerelease inesperada: $stable"
prerelease=$(search Onepack.Fixture --prerelease | json_ids | sort -u | tr '\n' ' ')
[ "$prerelease" = "Onepack.Fixture.Basic Onepack.Fixture.Dependent Onepack.Fixture.Ranged Onepack.Fixture.Rich " ] \
  || fail "búsqueda con prerelease inesperada: $prerelease"

step "Buscando por id exacto (registros)"
exact=$(search Onepack.Fixture.Basic --exact-match)
for v in 1.0.0 1.1.0 2.0.0; do
  grep -q "\"$v\"" <<<"$exact" || fail "--exact-match no muestra $v: $exact"
done

# --- Restore ---------------------------------------------------------------------------------

step "Restaurando el consumidor con caché vacía"
(cd "$client/consumer" && dotnet restore --configfile "$client/NuGet.Config" --nologo "-p:ConsumerTargetFramework=$tfm") \
  >"$work/restore.log" || { cat "$work/restore.log"; fail "dotnet restore"; }

step "Comprobando la resolución por rango (Basic 1.1.0 entre 1.0.0, 1.1.0 y 2.0.0)"
resolved=$(python3 -c '
import json, sys
libs = json.load(open(sys.argv[1]))["libraries"]
print(" ".join(sorted(k for k in libs if k.startswith("Onepack.Fixture.Basic/"))))' "$client/consumer/obj/project.assets.json")
[ "$resolved" = "Onepack.Fixture.Basic/1.1.0" ] || fail "Basic resuelto como '$resolved'"

step "Verificando bytes restaurados"
for pkg in onepack.fixture.basic/1.1.0 onepack.fixture.dependent/1.0.0 onepack.fixture.ranged/1.0.0 onepack.fixture.rich/2.0.0-beta.1; do
  id=${pkg%/*}
  version=${pkg#*/}
  pushed=$(ls "$work"/nupkgs/*.nupkg | grep -i "/$id.$version.nupkg")
  restored="$NUGET_PACKAGES/$id/$version/$id.$version.nupkg"
  [ -f "$restored" ] || fail "$pkg no se restauró"
  cmp -s "$pushed" "$restored" || fail "$pkg restaurado no coincide byte a byte con el publicado"
done

step "Ejecutando el consumidor"
output=$(cd "$client/consumer" && dotnet run --no-restore --nologo "-p:ConsumerTargetFramework=$tfm")
expected=$'dependent -> hello from onepack fixture\nranged -> hello from onepack fixture\nrich'
[ "$output" = "$expected" ] || fail "salida inesperada: '$output'"

# --- Unlist / relist -------------------------------------------------------------------------

step "Ocultando Basic 2.0.0 con dotnet nuget delete (unlist)"
(cd "$client" && dotnet nuget delete Onepack.Fixture.Basic 2.0.0 --source onepack --api-key "$token" --non-interactive) \
  >/dev/null || fail "dotnet nuget delete"
latest=$(search Onepack.Fixture.Basic)
grep -q '"1.1.0"' <<<"$latest" && ! grep -q '"2.0.0"' <<<"$latest" \
  || fail "tras el unlist la búsqueda debería mostrar 1.1.0: $latest"
curl -fsS -H "X-NuGet-ApiKey: $token" -o "$work/unlisted.nupkg" \
  "$base/nuget/$feed/v3/flat/onepack.fixture.basic/2.0.0/onepack.fixture.basic.2.0.0.nupkg" \
  || fail "la versión no listada debería seguir descargable"
cmp -s "$work/nupkgs/Onepack.Fixture.Basic.2.0.0.nupkg" "$work/unlisted.nupkg" || fail "bytes de la versión no listada"

step "Volviendo a listar Basic 2.0.0 (relist)"
[ "$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "X-NuGet-ApiKey: $token" \
  "$base/nuget/$feed/v2/package/Onepack.Fixture.Basic/2.0.0")" = "200" ] || fail "relist"
grep -q '"2.0.0"' <<<"$(search Onepack.Fixture.Basic)" || fail "tras el relist la búsqueda debería mostrar 2.0.0"

# --- Bloqueo ---------------------------------------------------------------------------------

admin_token=$(cat "$work/data/initial-admin-token")
availability() {
  curl -s -o "$work/availability.json" -w '%{http_code}' -X POST \
    -H "Authorization: Bearer $admin_token" -H 'Content-Type: application/json' \
    -d "{\"reason\": \"$2\"}" "$base/api/v1/feeds/$feed/packages/onepack.fixture.basic/1.1.0/$1"
}

step "Bloqueando Basic 1.1.0 por la API administrativa"
[ "$(availability block 'prueba e2e')" = "200" ] || fail "block: $(cat "$work/availability.json")"

step "Comprobando que restore falla con un mensaje claro (caché vacía)"
export NUGET_PACKAGES="$work/packages-blocked" NUGET_HTTP_CACHE_PATH="$work/http-cache-blocked"
rm -rf "$client/consumer/obj"
if (cd "$client/consumer" && dotnet restore --configfile "$client/NuGet.Config" --nologo "-p:ConsumerTargetFramework=$tfm") \
  >"$work/restore-blocked.log" 2>&1; then
  fail "restore debió fallar con Basic 1.1.0 bloqueada"
fi
grep -q "410" "$work/restore-blocked.log" && grep -q "PACKAGE_BLOCKED" "$work/restore-blocked.log" \
  || { cat "$work/restore-blocked.log"; fail "el error de restore no explica el bloqueo"; }
grep -m1 "PACKAGE_BLOCKED" "$work/restore-blocked.log" | sed 's/^ */    /'

step "Desbloqueando Basic 1.1.0 y restaurando de nuevo"
[ "$(availability unblock 'fin de la prueba')" = "200" ] || fail "unblock: $(cat "$work/availability.json")"
(cd "$client/consumer" && dotnet restore --configfile "$client/NuGet.Config" --nologo "-p:ConsumerTargetFramework=$tfm") \
  >"$work/restore-unblocked.log" 2>&1 || { cat "$work/restore-unblocked.log"; fail "restore tras desbloquear"; }

# --- Revocación ------------------------------------------------------------------------------

step "Revocando la credencial de CI"
onepackd token revoke "$token_id" >/dev/null
[ "$(curl -s -o /dev/null -w '%{http_code}' -H "X-NuGet-ApiKey: $token" "$source_url")" = "401" ] \
  || fail "el token revocado sigue funcionando"
if (cd "$client" && dotnet nuget push "$dup" --source onepack --api-key "$token" >"$work/revoked.log" 2>&1); then
  fail "el push con un token revocado debió fallar"
fi

grep -q "$token" "$work/server.log" && fail "el token apareció en el log del servidor"
grep -q "$admin_token" "$work/server.log" && fail "el token administrativo apareció en el log del servidor"

step "Comprobando qué recursos usaron los clientes"
for resource in /v3/index.json /v3/flat/ /v3/registration/ /v3/query /v2/package; do
  grep -q "uri=/nuget/$feed$resource" "$work/server.log" || fail "ningún cliente usó $resource"
done

echo "E2E OK (SDK $sdk_version$([ "$tls" = 1 ] && echo ', TLS')): 401 sin credenciales, push, 409, búsqueda con y sin"
echo "        prerelease, búsqueda exacta, restore con rango y caché vacía, unlist/relist, bloqueo"
echo "        y revocación."
