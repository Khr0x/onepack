#!/usr/bin/env bash
# Container image (Phase 7): builds it, initializes a volume, starts it without root and
# checks the probes and an authenticated operation.
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
fail() { echo "CONTAINER FAILED: $*" >&2; docker logs "$name" >&2 2>&1 || true; exit 1; }
step() { echo "==> $*"; }

step "Building $image"
docker build -q -f "$root/packaging/container/Containerfile" -t "$image" "$root" >/dev/null

step "Checking that the image does not run as root"
user=$(docker image inspect --format '{{.Config.User}}' "$image")
[ "$user" = "65532:65532" ] || fail "image user: '$user'"

step "Initializing the data volume"
docker volume create "$volume" >/dev/null
docker run --rm -v "$volume:/data" "$image" init >/dev/null
token=$(docker run --rm -v "$volume:/data" alpine:3 cat /data/initial-admin-token)
mode=$(docker run --rm -v "$volume:/data" alpine:3 stat -c '%a %u' /data)
[ "$mode" = "700 65532" ] || fail "volume permissions: $mode"

step "Starting the server"
docker run -d --name "$name" -p "127.0.0.1:$port:8080" -p "127.0.0.1:$((port + 1)):9464" \
  -e ONEPACK_PUBLIC_URL="http://127.0.0.1:$port" -v "$volume:/data" "$image" >/dev/null
for _ in $(seq 1 50); do
  curl -fs "http://127.0.0.1:$port/readyz" >/dev/null 2>&1 && break
  sleep 0.2
done
curl -fs "http://127.0.0.1:$port/readyz" | grep -q '"ready"' || fail "did not become ready"
curl -fs "http://127.0.0.1:$((port + 1))/metrics" | grep -q onepack_build_info || fail "no metrics"

step "Authenticated operation"
curl -fs -X POST -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
  -d '{"name":"internal"}' "http://127.0.0.1:$port/api/v1/feeds" >/dev/null || fail "create feed"
curl -fs -H "X-NuGet-ApiKey: $token" "http://127.0.0.1:$port/nuget/internal/v3/index.json" | grep -q PackageBaseAddress \
  || fail "service index"
[ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/nuget/internal/v3/index.json")" = 401 ] \
  || fail "without a credential it should be 401"
# Read before filtering: with pipefail, head/grep -q close the pipe and docker logs
# dies with SIGPIPE, which caused false failures and hid a leaked token.
logs=$(docker logs "$name" 2>&1)
[[ "$logs" == "{"* ]] || fail "the logs are not JSON"
grep -qF "$token" <<<"$logs" && fail "the token appeared in the logs"

step "Graceful shutdown with SIGTERM"
docker stop -t 10 "$name" >/dev/null
[ "$(docker inspect --format '{{.State.ExitCode}}' "$name")" = 0 ] || fail "exit code on shutdown"
echo "CONTAINER OK"
