#!/usr/bin/env bash
# Native macOS Keychain acceptance: login, use and logout with the real store.
set -euo pipefail

[ "$(uname -s)" = Darwin ] || {
  echo "macOS keychain E2E: skipped (requires macOS)" >&2
  exit 0
}

root=$(cd "$(dirname "$0")/.." && pwd)
port=${ONEPACK_KEYCHAIN_PORT:-5912}
base="http://127.0.0.1:$port"
context=local
work=$(mktemp -d)
server_pid=""
cli=""

fail() {
  echo "macOS KEYCHAIN E2E FAILED: $*" >&2
  if [ -f "$work/server.log" ]; then
    echo "--- onepackd log ---" >&2
    cat "$work/server.log" >&2
  fi
  exit 1
}

cleanup() {
  if [ -n "$cli" ] && [ -x "$cli" ]; then
    "$cli" --context "$context" logout >/dev/null 2>&1 || true
  fi
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

if [ -n "${ONEPACK_BIN_DIR:-}" ]; then
  bin=$(cd "$ONEPACK_BIN_DIR" && pwd)
else
  echo "==> building onepack and onepackd"
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server -p onepack-cli
  bin="$root/target/debug"
fi
cli="$bin/onepack"
daemon="$bin/onepackd"

export ONEPACK_CONFIG_DIR="$work/config" ONEPACK_NO_INPUT=1
unset ONEPACK_TOKEN ONEPACK_KEYRING ONEPACK_CA_CERT ONEPACK_URL ONEPACK_CONTEXT

echo "==> starting onepackd"
"$daemon" init --data-dir "$work/data" >/dev/null
token=$(tr -d '\n' <"$work/data/initial-admin-token")
"$daemon" serve --listen "127.0.0.1:$port" --data-dir "$work/data" \
  --public-url "$base" --min-free-space-mib 0 >"$work/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 50); do
  curl -fsS "$base/readyz" >/dev/null 2>&1 && break
  kill -0 "$server_pid" 2>/dev/null || fail "onepackd exited at startup"
  sleep 0.2
done

echo "==> login, use and logout with the macOS Keychain"
"$cli" context add "$context" --url "$base" --use >/dev/null
printf %s "$token" | "$cli" login --token-stdin >/dev/null
principal=$("$cli" whoami --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["principal"])')
[ "$principal" = "admin" ] || fail "whoami returned $principal"
"$cli" doctor --json | python3 -c '
import json, sys
checks = json.load(sys.stdin)["checks"]
check = next(c for c in checks if c["name"] == "credentials")
assert check["status"] == "ok" and "system keychain" in check["message"], check
'
secret=${token##*_}
if grep -rqF "$secret" "$ONEPACK_CONFIG_DIR"; then
  fail "the token was written to the CLI configuration"
fi

"$cli" logout >/dev/null
set +e
"$cli" whoami >/dev/null 2>&1
code=$?
set -e
[ "$code" = 3 ] || fail "whoami after logout exited $code, expected 3"

echo "MACOS KEYCHAIN E2E OK: login, whoami, doctor and logout with the native Keychain."
