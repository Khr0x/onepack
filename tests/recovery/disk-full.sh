#!/usr/bin/env bash
# Phase 2 blocking test: disk full during an upload.
# Expected: controlled error (507), nothing in staging, no metadata pointing to a
# nonexistent blob, and the service keeps working once there is space.
#
# Requires Linux and sudo (mounts a small tmpfs as the data directory).
set -euo pipefail

if [ "$(uname -s)" != "Linux" ]; then
  echo "disk-full: skipped (requires Linux to mount tmpfs)"
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
  echo "disk-full FAILED: $*" >&2
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

# Packages with incompressible content: one does not fit in the tmpfs and the other does.
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
  # The server may respond before receiving the whole body (e.g. 507); curl then reports
  # a send error even though it received the code, so its output is ignored.
  curl -s -o "$work/push.out" -w '%{http_code}' -X PUT "${auth[@]}" \
    -F "package=@$1;type=application/octet-stream" "$base/nuget/internal/v2/package" || true
}
status() { curl -s -o /dev/null -w '%{http_code}' "${auth[@]}" "$1"; }

code=$(push "$work/Big.nupkg")
[ "$code" = "507" ] || fail "upload with a full disk returned $code (expected 507): $(cat "$work/push.out")"
echo "ok: full disk -> 507"

[ -z "$(ls -A "$data/staging")" ] || fail "files were left in staging: $(ls "$data/staging")"
echo "ok: empty staging"

[ "$(status "$base/nuget/internal/v3/flat/big/index.json")" = "404" ] || fail "the failed version is visible"
echo "ok: the failed version is not visible"

code=$(push "$work/Small.nupkg")
[ "$code" = "201" ] || fail "after freeing space, publishing returned $code: $(cat "$work/push.out")"
curl -fsS "${auth[@]}" -o "$work/Small.downloaded" "$base/nuget/internal/v3/flat/small/1.0.0/small.1.0.0.nupkg"
cmp -s "$work/Small.nupkg" "$work/Small.downloaded" || fail "the downloaded package does not match"
echo "ok: the service is still operational and serves identical bytes"

echo "disk-full OK"
