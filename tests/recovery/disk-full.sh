#!/usr/bin/env bash
# Prueba bloqueante de la Fase 2: disco lleno durante una subida.
# Esperado: error controlado (507), nada en staging, ningún metadato apunta a un blob
# inexistente y el servicio sigue funcionando cuando hay espacio.
#
# Requiere Linux y sudo (monta un tmpfs pequeño como directorio de datos).
set -euo pipefail

if [ "$(uname -s)" != "Linux" ]; then
  echo "disk-full: se omite (requiere Linux para montar tmpfs)"
  exit 0
fi

root=$(cd "$(dirname "$0")/../.." && pwd)
bin="$root/target/debug/onepackd"
port=${ONEPACK_DISK_FULL_PORT:-5898}
base="http://127.0.0.1:$port"

work=$(mktemp -d)
mnt="$work/mnt"
mkdir -p "$mnt"
server_pid=""
cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  sudo umount "$mnt" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

fail() {
  echo "disk-full FALLÓ: $*" >&2
  cat "$work/server.log" >&2 || true
  exit 1
}

cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server

sudo mount -t tmpfs -o size=8m tmpfs "$mnt"
sudo chown "$(id -u):$(id -g)" "$mnt"
data="$mnt/data"

"$bin" init --data-dir "$data" >/dev/null
"$bin" feed create internal --data-dir "$data" >/dev/null
token=$(tr -d '\n' <"$data/initial-admin-token")
auth=(-H "X-NuGet-ApiKey: $token")
"$bin" serve --data-dir "$data" --listen "127.0.0.1:$port" --public-url "$base" \
  --min-free-space-mib 0 >"$work/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 50); do
  curl -fsS "${auth[@]}" "$base/nuget/internal/v3/index.json" >/dev/null 2>&1 && break
  sleep 0.2
done

# Paquetes con contenido incompresible: uno no cabe en el tmpfs y otro sí.
python3 - "$work" <<'EOF'
import os, sys, zipfile
work = sys.argv[1]
for name, size in [("Big", 12 * 1024 * 1024), ("Small", 64 * 1024)]:
    with zipfile.ZipFile(f"{work}/{name}.nupkg", "w", zipfile.ZIP_STORED) as z:
        z.writestr(f"{name}.nuspec",
                   f"<package><metadata><id>{name}</id><version>1.0.0</version></metadata></package>")
        z.writestr("content.bin", os.urandom(size))
EOF

push() {
  # El servidor puede responder antes de recibir todo el cuerpo (p. ej. 507); curl informa
  # entonces un error de envío aunque haya recibido el código, por eso se ignora su salida.
  curl -s -o "$work/push.out" -w '%{http_code}' -X PUT "${auth[@]}" \
    -F "package=@$1;type=application/octet-stream" "$base/nuget/internal/v2/package" || true
}
status() { curl -s -o /dev/null -w '%{http_code}' "${auth[@]}" "$1"; }

code=$(push "$work/Big.nupkg")
[ "$code" = "507" ] || fail "subida con disco lleno devolvió $code (esperado 507): $(cat "$work/push.out")"
echo "ok: disco lleno -> 507"

[ -z "$(ls -A "$data/staging")" ] || fail "quedaron archivos en staging: $(ls "$data/staging")"
echo "ok: staging vacío"

[ "$(status "$base/nuget/internal/v3/flat/big/index.json")" = "404" ] || fail "la versión fallida es visible"
echo "ok: la versión fallida no es visible"

code=$(push "$work/Small.nupkg")
[ "$code" = "201" ] || fail "tras liberar espacio la publicación devolvió $code: $(cat "$work/push.out")"
curl -fsS "${auth[@]}" -o "$work/Small.downloaded" "$base/nuget/internal/v3/flat/small/1.0.0/small.1.0.0.nupkg"
cmp -s "$work/Small.nupkg" "$work/Small.downloaded" || fail "el paquete descargado no coincide"
echo "ok: el servicio sigue operativo y sirve bytes idénticos"

echo "disk-full OK"
