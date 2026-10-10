#!/usr/bin/env bash
# Imagen de contenedor (Fase 7): construye, inicializa un volumen, arranca sin root y
# comprueba las sondas y una operación autenticada.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
image=${ONEPACK_IMAGE:-onepackd:test}
name=onepack-container-test
volume=onepack-container-test
port=18080

cleanup() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  docker volume rm "$volume" >/dev/null 2>&1 || true
}
trap cleanup EXIT
fail() { echo "CONTENEDOR FALLÓ: $*" >&2; docker logs "$name" >&2 2>&1 || true; exit 1; }
step() { echo "==> $*"; }

step "Construyendo $image"
docker build -q -f "$root/packaging/container/Containerfile" -t "$image" "$root" >/dev/null

step "Comprobando que la imagen no usa root"
user=$(docker image inspect --format '{{.Config.User}}' "$image")
[ "$user" = "65532:65532" ] || fail "usuario de la imagen: '$user'"

step "Inicializando el volumen de datos"
docker volume create "$volume" >/dev/null
docker run --rm -v "$volume:/data" "$image" init >/dev/null
token=$(docker run --rm -v "$volume:/data" alpine:3 cat /data/initial-admin-token)
mode=$(docker run --rm -v "$volume:/data" alpine:3 stat -c '%a %u' /data)
[ "$mode" = "700 65532" ] || fail "permisos del volumen: $mode"

step "Arrancando el servidor"
docker run -d --name "$name" -p "127.0.0.1:$port:8080" -p "127.0.0.1:$((port + 1)):9464" \
  -e ONEPACK_PUBLIC_URL="http://127.0.0.1:$port" -v "$volume:/data" "$image" >/dev/null
for _ in $(seq 1 50); do
  curl -fs "http://127.0.0.1:$port/readyz" >/dev/null 2>&1 && break
  sleep 0.2
done
curl -fs "http://127.0.0.1:$port/readyz" | grep -q '"ready"' || fail "no llegó a ready"
curl -fs "http://127.0.0.1:$((port + 1))/metrics" | grep -q onepack_build_info || fail "sin métricas"

step "Operación autenticada"
curl -fs -X POST -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
  -d '{"name":"internal"}' "http://127.0.0.1:$port/api/v1/feeds" >/dev/null || fail "crear feed"
curl -fs -H "X-NuGet-ApiKey: $token" "http://127.0.0.1:$port/nuget/internal/v3/index.json" | grep -q PackageBaseAddress \
  || fail "service index"
[ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/nuget/internal/v3/index.json")" = 401 ] \
  || fail "sin credencial debería ser 401"
# Se leen antes de filtrarlos: con pipefail, head/grep -q cierran la tubería y docker logs
# muere por SIGPIPE, lo que daba falsos fallos y ocultaba un token filtrado.
logs=$(docker logs "$name" 2>&1)
[[ "$logs" == "{"* ]] || fail "los logs no son JSON"
grep -qF "$token" <<<"$logs" && fail "el token apareció en los logs"

step "Parada ordenada con SIGTERM"
docker stop -t 10 "$name" >/dev/null
[ "$(docker inspect --format '{{.State.ExitCode}}' "$name")" = 0 ] || fail "código de salida al parar"
echo "CONTENEDOR OK"
