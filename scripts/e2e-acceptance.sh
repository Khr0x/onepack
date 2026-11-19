#!/usr/bin/env bash
# MVP acceptance test (Phase 8): the roadmap's "decisive test", end to end and in order:
#
#   A team installs the server, configures its project, publishes a library and restores it
#   from CI; then revokes a credential and recovers the service from a backup, without opening
#   a web UI or manually editing the database.
#
# Uses only `onepackd` (installation, backup and restore), `onepack` and `dotnet`. Never `sqlite3`.
# Includes the pilot incident exercise: revoke the CI credential, block a version and restore
# from a backup.
#
# Variables:
#   ONEPACK_BIN_DIR=DIR       use the `onepackd` and `onepack` binaries from DIR (e.g. those of
#                             a downloaded release) instead of building them.
#   ONEPACK_E2E_SDK=10.0.100  SDK for global.json (default: the one in tests/conformance-dotnet).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
conformance="$root/tests/conformance-dotnet"
port=${ONEPACK_E2E_ACCEPTANCE_PORT:-5899}
base="http://127.0.0.1:$port"
feed=libs

work=$(mktemp -d)
data="$work/srv/data"
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
  echo "ACCEPTANCE TEST FAILED: $*" >&2
  echo "--- onepackd log ---" >&2
  cat "$work"/server-*.log >&2 2>/dev/null || true
  exit 1
}
step() { echo "==> $*"; }
json() { python3 -c "import json, sys; print(json.load(sys.stdin)$1)"; }
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

# --- 1. Install the server ---------------------------------------------------------------------

if [ -n "${ONEPACK_BIN_DIR:-}" ]; then
  bin=$(cd "$ONEPACK_BIN_DIR" && pwd)
  step "Using the binaries from $bin"
else
  step "Building onepackd and onepack"
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server -p onepack-cli
  bin="$root/target/debug"
fi
[ -x "$bin/onepackd" ] && [ -x "$bin/onepack" ] || fail "missing onepackd or onepack in $bin"
onepackd() { "$bin/onepackd" "$@"; }
onepack() { "$bin/onepack" "$@"; }
step "onepackd $("$bin/onepackd" --version | awk '{print $2}'), onepack $("$bin/onepack" --version | awk '{print $2}')"

# Isolated CLI, no keychain or prompts, as on a CI agent.
export ONEPACK_CONFIG_DIR="$work/onepack-config" ONEPACK_KEYRING=off ONEPACK_NO_INPUT=1
unset ONEPACK_TOKEN

# Always starts onepackd with the same public URL and waits for /readyz.
serve() {
  "$bin/onepackd" serve --data-dir "$data" --listen "127.0.0.1:$port" --public-url "$base" \
    --min-free-space-mib 0 >"$work/server-$1.log" 2>&1 &
  server_pid=$!
  for _ in $(seq 1 50); do
    curl -fs "$base/readyz" >/dev/null 2>&1 && return 0
    kill -0 "$server_pid" 2>/dev/null || fail "onepackd exited at startup"
    sleep 0.2
  done
  fail "onepackd did not become ready"
}
stop() { kill "$server_pid"; wait "$server_pid" 2>/dev/null || true; server_pid=""; }

step "Installing: init and startup"
onepackd init --data-dir "$data" >/dev/null
ADMIN_TOKEN=$(tr -d '\n' <"$data/initial-admin-token")
export ADMIN_TOKEN
serve install

onepack context add registry --url "$base" >/dev/null
admin() { onepack --token-env ADMIN_TOKEN "$@"; }
[ "$(admin whoami --json | json '["administrator"]')" = "True" ] || fail "administrator whoami"

step "Creating the feed, the CI account and its token"
admin feed create "$feed" >/dev/null
admin principal create ci --kind service >/dev/null
admin grant add --principal ci --feed "$feed" --role publisher --publish-pattern 'Onepack.Fixture.*' >/dev/null
new_ci_token() { admin token create --principal ci --name "$1" --expires-in-days 1 2>/dev/null; }
CI_TOKEN=$(new_ci_token pipeline)
export CI_TOKEN
[[ "$CI_TOKEN" == opk_* ]] || fail "token create did not return a token"

# --- 2. Configure the project ------------------------------------------------------------------

if [ -n "${ONEPACK_E2E_SDK:-}" ]; then
  printf '{ "sdk": { "version": "%s", "rollForward": "latestFeature" } }\n' "$ONEPACK_E2E_SDK" >"$work/global.json"
else
  cp "$conformance/global.json" "$work/global.json"
fi
mkdir -p "$work/lib" "$work/app"
cp -R "$conformance/fixtures" "$work/lib/fixtures"
cp -R "$conformance/consumer" "$work/app/consumer"
rm -rf "$work"/lib/fixtures/*/bin "$work"/lib/fixtures/*/obj "$work"/app/consumer/bin "$work"/app/consumer/obj
sdk_version=$(cd "$work" && dotnet --version)
tfm="net${sdk_version%%.*}.0"

step "Configuring the project with onepack nuget init (SDK $sdk_version)"
printf '<?xml version="1.0" encoding="utf-8"?>\n<configuration>\n  <packageSources>\n    <clear />\n  </packageSources>\n</configuration>\n' \
  >"$work/app/NuGet.Config"
(cd "$work/app" && onepack nuget init --feed "$feed" --pattern '*' --yes) >/dev/null 2>&1 || fail "nuget init"

# --- 3. Publish a library ----------------------------------------------------------------------

step "Publishing the library from CI"
pack() { (cd "$work" && dotnet pack "lib/fixtures/$1" -c Release -o "$work/nupkgs" --nologo -v quiet "${@:2}") >/dev/null || fail "pack $1"; }
pack Onepack.Fixture.Basic
pack Onepack.Fixture.Basic -p:Version=1.1.0
pack Onepack.Fixture.Dependent
pack Onepack.Fixture.Rich
pack Onepack.Fixture.Ranged "-p:RestoreAdditionalProjectSources=$work/nupkgs"
onepack --token-env CI_TOKEN package push --feed "$feed" "$work"/nupkgs/*.nupkg >/dev/null 2>"$work/push.log" \
  || { cat "$work/push.log"; fail "package push"; }

# --- 4. Restore from CI ------------------------------------------------------------------------

# Each restore simulates a clean CI agent: no NuGet cache or previous obj/.
# Usage: ci_restore NAME  (leaves the log in $work/restore-NAME.log)
ci_restore() {
  rm -rf "$work/app/consumer/obj" "$work/app/consumer/bin"
  export NUGET_PACKAGES="$work/cache-$1/packages" NUGET_HTTP_CACHE_PATH="$work/cache-$1/http"
  (cd "$work/app/consumer" && onepack --token-env CI_TOKEN exec --feed "$feed" -- \
    dotnet restore --nologo "-p:ConsumerTargetFramework=$tfm") >"$work/restore-$1.log" 2>&1
}
run_app() {
  output=$(cd "$work/app/consumer" && dotnet run --no-restore --nologo "-p:ConsumerTargetFramework=$tfm")
  expected=$'dependent -> hello from onepack fixture\nranged -> hello from onepack fixture\nrich'
  [ "$output" = "$expected" ] || fail "unexpected output: '$output'"
}

step "Restoring from CI with an empty cache"
ci_restore first || { cat "$work/restore-first.log"; fail "dotnet restore from CI"; }
run_app

# --- 5. Incident: revoke the CI credential -----------------------------------------------------

step "Incident 1: the CI token leaks and is revoked"
token_id=$(admin token list --principal ci --json | json '[0]["id"]')
admin token revoke "$token_id" --yes >/dev/null
set +e
ci_restore revoked
code=$?
set -e
# onepack exec checks the credential before launching dotnet: clear error, no NU1301.
[ "$code" = "3" ] || { cat "$work/restore-revoked.log"; fail "exit code $code with the revoked token (expected 3)"; }
grep -q "AUTH_" "$work/restore-revoked.log" && ! grep -q "NU1301" "$work/restore-revoked.log" \
  || { cat "$work/restore-revoked.log"; fail "the error does not explain that the credential is invalid"; }

step "Rotating: new token for the pipeline"
CI_TOKEN=$(new_ci_token pipeline-rotated)
export CI_TOKEN
ci_restore rotated || { cat "$work/restore-rotated.log"; fail "restore with the rotated token"; }

# --- 6. Incident: block a version --------------------------------------------------------------

step "Incident 2: Onepack.Fixture.Basic 1.1.0 has a vulnerability and is blocked"
admin package block --feed "$feed" Onepack.Fixture.Basic 1.1.0 --reason "drill: fictional CVE" --yes >/dev/null
if ci_restore blocked; then fail "the restore downloaded a blocked version"; fi
grep -q "PACKAGE_BLOCKED" "$work/restore-blocked.log" \
  || { cat "$work/restore-blocked.log"; fail "the error does not explain the block"; }
admin package unblock --feed "$feed" Onepack.Fixture.Basic 1.1.0 --reason "drill: published patch" >/dev/null
ci_restore unblocked || { cat "$work/restore-unblocked.log"; fail "restore after unblocking"; }

# --- 7. Incident: lose the server and recover from backup --------------------------------------

step "Incident 3: hot backup and total loss of the data directory"
onepackd backup --data-dir "$data" --output "$work/backup" >/dev/null || fail "backup"
(cd "$work/nupkgs" && for f in *.nupkg; do echo "$(sha "$f") $f"; done) >"$work/expected.sha256"
stop
rm -rf "$data"
if curl -fs "$base/readyz" >/dev/null 2>&1; then fail "the server is still responding"; fi

step "Recovering: restore, check and startup with the same URL"
onepackd restore --from "$work/backup" --data-dir "$data" >/dev/null || fail "restore"
onepackd check --data-dir "$data" >/dev/null || fail "check after restoring"
serve recovered

step "Verifying the recovered service"
ci_restore recovered || { cat "$work/restore-recovered.log"; fail "restore from CI after recovery"; }
run_app
checked=0
while read -r hash file; do
  found=$(find "$NUGET_PACKAGES" -name "$(echo "$file" | tr '[:upper:]' '[:lower:]')" | head -1)
  [ -n "$found" ] || continue
  [ "$(sha "$found")" = "$hash" ] || fail "$file restored does not match the published one"
  checked=$((checked + 1))
done <"$work/expected.sha256"
[ "$checked" -ge 4 ] || fail "only $checked packages were compared"

# Security state survives the restore: the revoked token is still revoked.
revoked=$(admin token list --principal ci --json | python3 -c '
import json, sys
print(sum(1 for t in json.load(sys.stdin) if t.get("revoked_at")))')
[ "$revoked" = "1" ] || fail "the revoked token is not still revoked after the restore ($revoked)"

step "Checking the incident audit trail"
actions=$(admin audit list --feed "$feed" --limit 100 --json | python3 -c '
import json, sys
print(" ".join(sorted({e["action"] for e in json.load(sys.stdin)})))')
for a in package.block package.unblock; do
  [[ " $actions " == *" $a "* ]] || fail "missing $a in the audit trail ($actions)"
done
admin audit list --action token.revoke --json | grep -q token.revoke || fail "missing token.revoke in the audit trail"

step "Checking that no secret was left on disk or in logs"
for token in "$CI_TOKEN" "$ADMIN_TOKEN"; do
  secret=${token##*_}
  grep -rqF "$secret" "$work/app" "$ONEPACK_CONFIG_DIR" && fail "a token was written to the project or to the configuration"
  grep -qF "$secret" "$work"/server-*.log && fail "a token appeared in the server log"
done
stop

echo "ACCEPTANCE TEST OK (SDK $sdk_version): install, configure, publish, restore from CI,"
echo "  revoke and rotate the credential, block a version and recover from backup;"
echo "  $checked packages identical byte for byte, with no UI or database access."
