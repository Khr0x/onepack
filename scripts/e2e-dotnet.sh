#!/usr/bin/env bash
# End-to-end test with real .NET clients (ADR-019):
# dotnet pack -> dotnet nuget push -> onepackd -> search / restore / delete (unlist) -> run.
#
# Variables:
#   ONEPACK_E2E_SDK=10.0.100  SDK for global.json (default: the one in tests/conformance-dotnet).
#   ONEPACK_E2E_TLS=1         serves onepackd behind nginx with TLS (Linux only; uses sudo to
#                             trust a temporary CA).
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
  echo "E2E FAILED: $*" >&2
  echo "--- onepackd log ---" >&2
  cat "$work/server.log" >&2 || true
  exit 1
}

step() { echo "==> $*"; }

# Package IDs present in a `dotnet package search` JSON output.
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
step "SDK .NET $sdk_version (consumer: $tfm)$([ "$tls" = 1 ] && echo ', behind TLS')"

# --- Server ----------------------------------------------------------------------------------

step "Building onepackd"
cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server

onepackd() { "$root/target/debug/onepackd" "$@" --data-dir "$work/data"; }

step "Initializing the data directory and a CI service account"
onepackd init >/dev/null
onepackd feed create "$feed" >/dev/null
onepackd principal create ci --kind service >/dev/null
onepackd grant set --principal ci --feed "$feed" --role publisher --publish-pattern "Onepack.Fixture.*" >/dev/null
token=$(onepackd token create --principal ci --expires-in-days 1 2>/dev/null)
token_id=${token:4:16}

step "Starting onepackd (public URL $base)"
# HTTP traces (method and path, no headers) to check which resources clients use.
RUST_LOG="info,tower_http=debug" "$root/target/debug/onepackd" serve \
  --listen "127.0.0.1:$port" \
  --data-dir "$work/data" \
  --public-url "$base" \
  --min-free-space-mib 0 >"$work/server.log" 2>&1 &
server_pid=$!

if [ "$tls" = 1 ]; then
  step "Configuring nginx with TLS at $base"
  [ "$(uname -s)" = Linux ] || fail "ONEPACK_E2E_TLS requires Linux"
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
  kill -0 "$server_pid" 2>/dev/null || fail "onepackd exited at startup"
  sleep 0.2
done
curl -fsS -H "X-NuGet-ApiKey: $token" "$source_url" >"$work/index.json" || fail "onepackd did not respond at $source_url"

step "Checking that requests without credentials are rejected (401)"
[ "$(curl -s -o /dev/null -w '%{http_code}' "$source_url")" = "401" ] || fail "the feed responded without credentials"

step "Checking that all service index URLs use the public URL"
python3 - "$work/index.json" "$base/nuget/$feed/" <<'EOF' || fail "the service index has URLs that do not start from the public URL"
import json, sys
resources = json.load(open(sys.argv[1]))["resources"]
bad = [r["@id"] for r in resources if not r["@id"].startswith(sys.argv[2])]
assert resources and not bad, bad
EOF

# --- Packages --------------------------------------------------------------------------------

step "Packing fixtures with dotnet pack"
pack() { (cd "$work" && dotnet pack "src/fixtures/$1" -c Release -o "$work/nupkgs" --nologo -v quiet "${@:2}") || fail "pack $1"; }
pack Onepack.Fixture.Basic
pack Onepack.Fixture.Basic -p:Version=1.1.0
pack Onepack.Fixture.Basic -p:Version=2.0.0
pack Onepack.Fixture.Dependent
pack Onepack.Fixture.Rich
pack Onepack.Fixture.Ranged "-p:RestoreAdditionalProjectSources=$work/nupkgs"

# Isolated configuration: only onepackd as source and no secrets (ADR-016).
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

step "Publishing with dotnet nuget push"
for pkg in "$work"/nupkgs/*.nupkg; do
  (cd "$client" && dotnet nuget push "$pkg" --source onepack --api-key "$token" >"$work/push.log" 2>&1) \
    || fail "push of $(basename "$pkg"): $(cat "$work/push.log")"
done

step "Checking that a duplicate version is rejected (409)"
dup="$work/nupkgs/Onepack.Fixture.Basic.1.0.0.nupkg"
if (cd "$client" && dotnet nuget push "$dup" --source onepack --api-key "$token" >"$work/dup.log" 2>&1); then
  fail "the duplicate push should have failed"
fi
grep -q "409" "$work/dup.log" || fail "the duplicate push did not return 409: $(cat "$work/dup.log")"

# --- Search ----------------------------------------------------------------------------------

search() { (cd "$client" && dotnet package search "$@" --source onepack --format json); }

step "Searching with dotnet package search (without and with prerelease)"
stable=$(search Onepack.Fixture | json_ids | sort -u | tr '\n' ' ')
[ "$stable" = "Onepack.Fixture.Basic Onepack.Fixture.Dependent Onepack.Fixture.Ranged " ] \
  || fail "unexpected search without prerelease: $stable"
prerelease=$(search Onepack.Fixture --prerelease | json_ids | sort -u | tr '\n' ' ')
[ "$prerelease" = "Onepack.Fixture.Basic Onepack.Fixture.Dependent Onepack.Fixture.Ranged Onepack.Fixture.Rich " ] \
  || fail "unexpected search with prerelease: $prerelease"

step "Searching by exact id (registrations)"
exact=$(search Onepack.Fixture.Basic --exact-match)
for v in 1.0.0 1.1.0 2.0.0; do
  grep -q "\"$v\"" <<<"$exact" || fail "--exact-match does not show $v: $exact"
done

# --- Restore ---------------------------------------------------------------------------------

step "Restoring the consumer with an empty cache"
(cd "$client/consumer" && dotnet restore --configfile "$client/NuGet.Config" --nologo "-p:ConsumerTargetFramework=$tfm") \
  >"$work/restore.log" || { cat "$work/restore.log"; fail "dotnet restore"; }

step "Checking range resolution (Basic 1.1.0 among 1.0.0, 1.1.0 and 2.0.0)"
resolved=$(python3 -c '
import json, sys
libs = json.load(open(sys.argv[1]))["libraries"]
print(" ".join(sorted(k for k in libs if k.startswith("Onepack.Fixture.Basic/"))))' "$client/consumer/obj/project.assets.json")
[ "$resolved" = "Onepack.Fixture.Basic/1.1.0" ] || fail "Basic resolved as '$resolved'"

step "Verifying restored bytes"
for pkg in onepack.fixture.basic/1.1.0 onepack.fixture.dependent/1.0.0 onepack.fixture.ranged/1.0.0 onepack.fixture.rich/2.0.0-beta.1; do
  id=${pkg%/*}
  version=${pkg#*/}
  pushed=$(ls "$work"/nupkgs/*.nupkg | grep -i "/$id.$version.nupkg")
  restored="$NUGET_PACKAGES/$id/$version/$id.$version.nupkg"
  [ -f "$restored" ] || fail "$pkg was not restored"
  cmp -s "$pushed" "$restored" || fail "$pkg restored does not match the published one byte for byte"
done

step "Running the consumer"
output=$(cd "$client/consumer" && dotnet run --no-restore --nologo "-p:ConsumerTargetFramework=$tfm")
expected=$'dependent -> hello from onepack fixture\nranged -> hello from onepack fixture\nrich'
[ "$output" = "$expected" ] || fail "unexpected output: '$output'"

# --- Unlist / relist -------------------------------------------------------------------------

step "Hiding Basic 2.0.0 with dotnet nuget delete (unlist)"
(cd "$client" && dotnet nuget delete Onepack.Fixture.Basic 2.0.0 --source onepack --api-key "$token" --non-interactive) \
  >/dev/null || fail "dotnet nuget delete"
latest=$(search Onepack.Fixture.Basic)
grep -q '"1.1.0"' <<<"$latest" && ! grep -q '"2.0.0"' <<<"$latest" \
  || fail "after unlist the search should show 1.1.0: $latest"
curl -fsS -H "X-NuGet-ApiKey: $token" -o "$work/unlisted.nupkg" \
  "$base/nuget/$feed/v3/flat/onepack.fixture.basic/2.0.0/onepack.fixture.basic.2.0.0.nupkg" \
  || fail "the unlisted version should still be downloadable"
cmp -s "$work/nupkgs/Onepack.Fixture.Basic.2.0.0.nupkg" "$work/unlisted.nupkg" || fail "bytes of the unlisted version"

step "Listing Basic 2.0.0 again (relist)"
[ "$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "X-NuGet-ApiKey: $token" \
  "$base/nuget/$feed/v2/package/Onepack.Fixture.Basic/2.0.0")" = "200" ] || fail "relist"
grep -q '"2.0.0"' <<<"$(search Onepack.Fixture.Basic)" || fail "after relist the search should show 2.0.0"

# --- Blocking --------------------------------------------------------------------------------

admin_token=$(cat "$work/data/initial-admin-token")
availability() {
  curl -s -o "$work/availability.json" -w '%{http_code}' -X POST \
    -H "Authorization: Bearer $admin_token" -H 'Content-Type: application/json' \
    -d "{\"reason\": \"$2\"}" "$base/api/v1/feeds/$feed/packages/onepack.fixture.basic/1.1.0/$1"
}

step "Blocking Basic 1.1.0 through the admin API"
[ "$(availability block 'e2e test')" = "200" ] || fail "block: $(cat "$work/availability.json")"

step "Checking that restore fails with a clear message (empty cache)"
export NUGET_PACKAGES="$work/packages-blocked" NUGET_HTTP_CACHE_PATH="$work/http-cache-blocked"
rm -rf "$client/consumer/obj"
if (cd "$client/consumer" && dotnet restore --configfile "$client/NuGet.Config" --nologo "-p:ConsumerTargetFramework=$tfm") \
  >"$work/restore-blocked.log" 2>&1; then
  fail "restore should have failed with Basic 1.1.0 blocked"
fi
grep -q "410" "$work/restore-blocked.log" && grep -q "PACKAGE_BLOCKED" "$work/restore-blocked.log" \
  || { cat "$work/restore-blocked.log"; fail "the restore error does not explain the block"; }
grep -m1 "PACKAGE_BLOCKED" "$work/restore-blocked.log" | sed 's/^ */    /'

step "Unblocking Basic 1.1.0 and restoring again"
[ "$(availability unblock 'end of test')" = "200" ] || fail "unblock: $(cat "$work/availability.json")"
(cd "$client/consumer" && dotnet restore --configfile "$client/NuGet.Config" --nologo "-p:ConsumerTargetFramework=$tfm") \
  >"$work/restore-unblocked.log" 2>&1 || { cat "$work/restore-unblocked.log"; fail "restore after unblocking"; }

# --- Revocation ------------------------------------------------------------------------------

step "Revoking the CI credential"
onepackd token revoke "$token_id" >/dev/null
[ "$(curl -s -o /dev/null -w '%{http_code}' -H "X-NuGet-ApiKey: $token" "$source_url")" = "401" ] \
  || fail "the revoked token still works"
if (cd "$client" && dotnet nuget push "$dup" --source onepack --api-key "$token" >"$work/revoked.log" 2>&1); then
  fail "the push with a revoked token should have failed"
fi

grep -q "$token" "$work/server.log" && fail "the token appeared in the server log"
grep -q "$admin_token" "$work/server.log" && fail "the admin token appeared in the server log"

step "Checking which resources the clients used"
for resource in /v3/index.json /v3/flat/ /v3/registration/ /v3/query /v2/package; do
  grep -q "uri=/nuget/$feed$resource" "$work/server.log" || fail "no client used $resource"
done

echo "E2E OK (SDK $sdk_version$([ "$tls" = 1 ] && echo ', TLS')): 401 without credentials, push, 409, search with and without"
echo "        prerelease, exact search, restore with range and empty cache, unlist/relist, blocking"
echo "        and revocation."
