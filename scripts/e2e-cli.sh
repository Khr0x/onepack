#!/usr/bin/env bash
# Phase 6 end-to-end test (ADR-015, ADR-016): registry operated without a UI.
# After startup (`onepackd init`), everything is done with `onepack` and `dotnet`:
# feed -> CI principal -> token -> grant -> push -> nuget init -> exec dotnet restore -> run.
#
# Variables:
#   ONEPACK_E2E_SDK=10.0.100  SDK for global.json (default: the one in tests/conformance-dotnet).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
conformance="$root/tests/conformance-dotnet"
port=${ONEPACK_E2E_CLI_PORT:-5898}
base="http://127.0.0.1:$port"
feed=e2e

work=$(mktemp -d)
client="$work/client"
mkdir -p "$work/src" "$client"
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
  echo "E2E CLI FAILED: $*" >&2
  echo "--- onepackd log ---" >&2
  cat "$work/server.log" >&2 || true
  exit 1
}

step() { echo "==> $*"; }
json() { python3 -c "import json, sys; print(json.load(sys.stdin)$1)"; }

if [ -n "${ONEPACK_E2E_SDK:-}" ]; then
  printf '{ "sdk": { "version": "%s", "rollForward": "latestFeature" } }\n' "$ONEPACK_E2E_SDK" >"$work/global.json"
else
  cp "$conformance/global.json" "$work/global.json"
fi
cp -R "$conformance/fixtures" "$work/src/fixtures"
cp -R "$conformance/consumer" "$client/consumer"
rm -rf "$work"/src/fixtures/*/bin "$work"/src/fixtures/*/obj "$client"/consumer/bin "$client"/consumer/obj
sdk_version=$(cd "$work" && dotnet --version)
tfm="net${sdk_version%%.*}.0"
step "SDK .NET $sdk_version (consumer: $tfm)"

step "Building onepackd and onepack"
cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server -p onepack-cli
onepack() { "$root/target/debug/onepack" "$@"; }

# Isolated CLI: its own configuration and no keychain (as on a CI agent).
export ONEPACK_CONFIG_DIR="$work/onepack-config" ONEPACK_KEYRING=off ONEPACK_NO_INPUT=1
export NUGET_PACKAGES="$work/packages" NUGET_HTTP_CACHE_PATH="$work/http-cache"
unset ONEPACK_TOKEN

# --- Server startup (the only time onepackd is used) ------------------------------------------

step "Initializing and starting onepackd"
"$root/target/debug/onepackd" init --data-dir "$work/data" >/dev/null
ADMIN_TOKEN=$(tr -d '\n' <"$work/data/initial-admin-token")
export ADMIN_TOKEN
"$root/target/debug/onepackd" serve --listen "127.0.0.1:$port" --data-dir "$work/data" \
  --public-url "$base" --min-free-space-mib 0 >"$work/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 50); do
  curl -s -o /dev/null "$base/api/v1/capabilities" && break
  kill -0 "$server_pid" 2>/dev/null || fail "onepackd exited at startup"
  sleep 0.2
done

# --- Administration only with onepack ---------------------------------------------------------

step "Configuring the context and checking the identity"
onepack context add local --url "$base" >/dev/null
admin() { onepack --token-env ADMIN_TOKEN "$@"; }
[ "$(admin whoami --json | json '["administrator"]')" = "True" ] || fail "administrator whoami"

step "Creating the feed, the CI service account, token and grant"
admin feed create "$feed" >/dev/null
admin principal create ci --kind service >/dev/null
CI_TOKEN=$(admin token create --principal ci --name e2e --expires-in-days 1 2>/dev/null)
export CI_TOKEN
[[ "$CI_TOKEN" == opk_* ]] || fail "token create did not return a token"
admin grant add --principal ci --feed "$feed" --role publisher --publish-pattern 'Onepack.Fixture.*' >/dev/null
admin principal create dev --kind user >/dev/null
DEV_TOKEN=$(admin token create --principal dev --expires-in-days 1 2>/dev/null)
export DEV_TOKEN
admin grant add --principal dev --feed "$feed" --role reader >/dev/null
ci() { onepack --token-env CI_TOKEN "$@"; }

step "Packing fixtures with dotnet pack"
pack() { (cd "$work" && dotnet pack "src/fixtures/$1" -c Release -o "$work/nupkgs" --nologo -v quiet "${@:2}") >/dev/null || fail "pack $1"; }
pack Onepack.Fixture.Basic
pack Onepack.Fixture.Basic -p:Version=1.1.0
pack Onepack.Fixture.Basic -p:Version=2.0.0
pack Onepack.Fixture.Dependent
pack Onepack.Fixture.Rich
pack Onepack.Fixture.Ranged "-p:RestoreAdditionalProjectSources=$work/nupkgs"

step "Publishing with onepack package push"
ci package push --feed "$feed" "$work"/nupkgs/*.nupkg 2>"$work/push.log" >/dev/null \
  || { cat "$work/push.log"; fail "package push"; }
skipped=$(ci package push --feed "$feed" "$work"/nupkgs/*.nupkg --skip-existing-identical --json \
  | python3 -c 'import json, sys; print(sum(r["status"] == "skipped" for r in json.load(sys.stdin)))')
[ "$skipped" = "6" ] || fail "--skip-existing-identical should have skipped the 6 packages ($skipped)"
[ "$(ci package list --feed "$feed" --json | json '.__len__()')" = "4" ] || fail "package list"

# --- NuGet configuration and restore ----------------------------------------------------------

step "Generating NuGet.Config with onepack nuget init (no secrets)"
cat >"$client/NuGet.Config" <<'EOF'
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <!-- previous team configuration -->
  <packageSources>
    <clear />
    <add key="nuget.org" value="https://api.nuget.org/v3/index.json" protocolVersion="3" />
  </packageSources>
</configuration>
EOF
(cd "$client" && onepack nuget init --feed "$feed" --pattern 'Onepack.Fixture.*' --dry-run) >"$work/diff.txt"
grep -q '^+.*onepack_e2e' "$work/diff.txt" || fail "the diff does not show the new source"
(cd "$client" && onepack nuget init --feed "$feed" --pattern 'Onepack.Fixture.*' --yes) >/dev/null 2>&1
grep -q 'previous team configuration' "$client/NuGet.Config" || fail "nuget init lost previous content"
grep -q 'api.nuget.org' "$client/NuGet.Config" || fail "nuget init lost the previous source"
grep -q '<package pattern="Onepack.Fixture.\*" />' "$client/NuGet.Config" || fail "missing source mapping"

step "Diagnostics with onepack doctor"
(cd "$client" && onepack --token-env CI_TOKEN doctor --feed "$feed" --require publish) >"$work/doctor.txt" \
  || { cat "$work/doctor.txt"; fail "doctor with the CI credential"; }
if (cd "$client" && onepack --token-env DEV_TOKEN doctor --feed "$feed" --require publish --json) >"$work/doctor-dev.json"; then
  fail "doctor should have failed without publish permission"
fi
grep -q AUTH_SCOPE_MISSING "$work/doctor-dev.json" || fail "doctor does not report AUTH_SCOPE_MISSING"
grep -q -- "--role publisher" "$work/doctor-dev.json" || fail "doctor does not suggest the action"
grep -q "${DEV_TOKEN##*_}" "$work/doctor-dev.json" && fail "doctor showed the secret"

step "Restoring with onepack exec -- dotnet restore (empty cache)"
(cd "$client/consumer" && onepack --token-env DEV_TOKEN exec --feed "$feed" -- \
  dotnet restore --nologo "-p:ConsumerTargetFramework=$tfm") >"$work/restore.log" 2>&1 \
  || { cat "$work/restore.log"; fail "exec dotnet restore"; }
resolved=$(python3 -c '
import json, sys
libs = json.load(open(sys.argv[1]))["libraries"]
print(" ".join(sorted(k for k in libs if k.startswith("Onepack.Fixture.Basic/"))))' "$client/consumer/obj/project.assets.json")
[ "$resolved" = "Onepack.Fixture.Basic/1.1.0" ] || fail "Basic resolved as '$resolved'"

step "Running the consumer"
output=$(cd "$client/consumer" && dotnet run --no-restore --nologo "-p:ConsumerTargetFramework=$tfm")
expected=$'dependent -> hello from onepack fixture\nranged -> hello from onepack fixture\nrich'
[ "$output" = "$expected" ] || fail "unexpected output: '$output'"

step "Checking that the token was left in neither files nor the environment"
env | grep -q '^NuGetPackageSourceCredentials_' && fail "the credential was left in the shell environment"
for token in "$DEV_TOKEN" "$CI_TOKEN" "$ADMIN_TOKEN"; do
  secret=${token##*_}
  grep -rqF "$secret" "$client" "$ONEPACK_CONFIG_DIR" && fail "a token was written to disk"
  grep -qF "$secret" "$work/server.log" && fail "a token appeared in the server log"
done

step "Revoking the CI token"
token_id=$(admin token list --principal ci --json | json '[0]["id"]')
admin token revoke "$token_id" --yes >/dev/null
if ci whoami >/dev/null 2>&1; then fail "the revoked token still works"; fi
set +e
ci whoami >/dev/null 2>&1
code=$?
set -e
[ "$code" = "3" ] || fail "exit code $code with revoked token (expected 3)"

echo "E2E CLI OK (SDK $sdk_version): feed, principal, token, grant, push, skip-existing-identical,"
echo "            nuget init, doctor, exec dotnet restore with source mapping, run and revocation."
