#!/usr/bin/env bash
# Linux checks for the `onepack` CLI that the other end-to-end tests skip (they all run with
# ONEPACK_KEYRING=off and plain HTTP):
#
#   1. Credentials in the real system keychain (Secret Service through D-Bus, gnome-keyring).
#   2. A headless machine without a keychain: `login` fails with exit 9; ONEPACK_TOKEN works.
#   3. TLS: rejected with an unknown CA, accepted with --ca-cert and with the CA in the system
#      trust store.
#   4. Optional: the same binary on other distributions (static musl binaries only).
#
# Linux only. Uses sudo to install packages and to trust a temporary CA.
#
# Variables:
#   ONEPACK_BIN_DIR=DIR                  use `onepackd` and `onepack` from DIR (e.g. release
#                                        binaries) instead of building them.
#   ONEPACK_E2E_DISTROS="alpine:3.20 …"  container images to run the CLI in (needs docker and a
#                                        static binary).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
port=${ONEPACK_E2E_LINUX_PORT:-5911}
tls_port=${ONEPACK_E2E_LINUX_TLS_PORT:-8453}
http_url="http://127.0.0.1:$port"
tls_url="https://localhost:$tls_port"
ca_name=onepack-e2e-cli.crt

work=$(mktemp -d)
server_pid=""
cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [ -f "$work/nginx/nginx.pid" ]; then
    kill "$(cat "$work/nginx/nginx.pid")" 2>/dev/null || true
  fi
  if [ -f "/usr/local/share/ca-certificates/$ca_name" ]; then
    sudo rm -f "/usr/local/share/ca-certificates/$ca_name"
    sudo update-ca-certificates --fresh >/dev/null 2>&1 || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

fail() {
  echo "LINUX CLI E2E FAILED: $*" >&2
  echo "--- onepackd log ---" >&2
  cat "$work/server.log" >&2 || true
  exit 1
}
step() { echo "==> $*"; }
json() { python3 -c "import json, sys; print(json.load(sys.stdin)$1)"; }
# Runs a command that must fail and checks its exit code and error code.
expect_failure() {
  local want_exit=$1 want_code=$2
  shift 2
  set +e
  "$@" >"$work/out.log" 2>"$work/err.log"
  local got=$?
  set -e
  [ "$got" = "$want_exit" ] || fail "$* exited $got, expected $want_exit: $(cat "$work/err.log")"
  grep -q "$want_code" "$work/err.log" || fail "$* did not report $want_code: $(cat "$work/err.log")"
}

[ "$(uname -s)" = Linux ] || fail "this test needs Linux"

step "Installing test dependencies (gnome-keyring, D-Bus, nginx)"
missing=()
command -v dbus-run-session >/dev/null || missing+=(dbus)
command -v gnome-keyring-daemon >/dev/null || missing+=(gnome-keyring)
command -v nginx >/dev/null || missing+=(nginx)
if [ ${#missing[@]} -gt 0 ]; then
  sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${missing[@]}" >/dev/null
fi

if [ -n "${ONEPACK_BIN_DIR:-}" ]; then
  bin=$(cd "$ONEPACK_BIN_DIR" && pwd)
else
  step "Building onepackd and onepack"
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server -p onepack-cli
  bin="$root/target/debug"
fi
onepack() { "$bin/onepack" "$@"; }
step "$(onepack --version) ($(file -b "$bin/onepack" | cut -d, -f1-2))"

export ONEPACK_CONFIG_DIR="$work/onepack-config" ONEPACK_NO_INPUT=1
unset ONEPACK_TOKEN ONEPACK_KEYRING ONEPACK_CA_CERT ONEPACK_URL ONEPACK_CONTEXT

step "Starting onepackd"
"$bin/onepackd" init --data-dir "$work/data" >/dev/null
ADMIN_TOKEN=$(tr -d '\n' <"$work/data/initial-admin-token")
export ADMIN_TOKEN
"$bin/onepackd" serve --listen "127.0.0.1:$port" --data-dir "$work/data" \
  --public-url "$tls_url" --min-free-space-mib 0 >"$work/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 50); do
  curl -s -o /dev/null "$http_url/api/v1/capabilities" && break
  kill -0 "$server_pid" 2>/dev/null || fail "onepackd exited at startup"
  sleep 0.2
done

# --- 1. System keychain (Secret Service) ------------------------------------------------------

step "Keychain: login, use from a new process and logout (Secret Service)"
onepack context add local --url "$http_url" --use >/dev/null
# A private session bus with an unlocked keyring, like a desktop session after logging in.
dbus-run-session -- bash -euo pipefail -c '
  echo -n onepack-e2e | gnome-keyring-daemon --unlock --components=secrets >/dev/null
  onepack() { "$0" "$@"; }
  printf %s "$ADMIN_TOKEN" | onepack login --token-stdin >/dev/null
  [ "$(onepack whoami --json | python3 -c "import json, sys; print(json.load(sys.stdin)[\"principal\"])")" = admin ]
  onepack doctor --json | python3 -c "
import json, sys
check = next(c for c in json.load(sys.stdin)[\"checks\"] if c[\"name\"] == \"credentials\")
assert check[\"status\"] == \"ok\" and \"system keychain\" in check[\"message\"], check"
  onepack logout >/dev/null
  set +e; onepack whoami >/dev/null 2>&1; code=$?; set -e
  [ "$code" = 3 ] || { echo "whoami after logout exited $code, expected 3" >&2; exit 1; }
' "$bin/onepack" || fail "keychain round trip"
secret=${ADMIN_TOKEN##*_}
grep -rqF "$secret" "$ONEPACK_CONFIG_DIR" && fail "the token was written to the CLI configuration"

# --- 2. Headless machine without a keychain ---------------------------------------------------

step "Headless: login fails with exit 9, ONEPACK_TOKEN and --token-env work"
headless() { env -u DBUS_SESSION_BUS_ADDRESS DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent "$@"; }
printf %s "$ADMIN_TOKEN" >"$work/token"
expect_failure 9 KEYCHAIN_UNAVAILABLE headless "$bin/onepack" login --token-stdin <"$work/token"
grep -q -- "--token-env" "$work/err.log" || fail "login does not suggest --token-env"
headless env ONEPACK_TOKEN="$ADMIN_TOKEN" "$bin/onepack" whoami >/dev/null || fail "whoami with ONEPACK_TOKEN"
headless env ADMIN_TOKEN="$ADMIN_TOKEN" "$bin/onepack" --token-env ADMIN_TOKEN whoami >/dev/null \
  || fail "whoami with --token-env"
ONEPACK_KEYRING=off "$bin/onepack" --token-env ADMIN_TOKEN whoami >/dev/null || fail "whoami with ONEPACK_KEYRING=off"

# --- 3. TLS -----------------------------------------------------------------------------------

step "TLS: nginx with a temporary CA in front of onepackd"
pki="$work/pki"
mkdir -p "$pki" "$work/nginx"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=onepack-e2e-cli-ca" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" \
  -keyout "$pki/ca.key" -out "$pki/ca.crt" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj "/CN=localhost" -keyout "$pki/tls.key" -out "$pki/tls.csr" 2>/dev/null
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n' >"$pki/ext"
openssl x509 -req -in "$pki/tls.csr" -CA "$pki/ca.crt" -CAkey "$pki/ca.key" -CAcreateserial \
  -days 1 -extfile "$pki/ext" -out "$pki/tls.crt" 2>/dev/null
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
  server {
    listen 127.0.0.1:$tls_port ssl;
    ssl_certificate $pki/tls.crt;
    ssl_certificate_key $pki/tls.key;
    location / {
      proxy_pass http://127.0.0.1:$port;
      proxy_set_header Host \$host;
      proxy_set_header X-Forwarded-Proto https;
    }
  }
}
EOF
nginx -p "$work/nginx" -c "$work/nginx/nginx.conf" -e "$work/nginx/error.log"
for _ in $(seq 1 50); do
  curl -s -o /dev/null --cacert "$pki/ca.crt" "$tls_url/api/v1/capabilities" && break
  sleep 0.2
done

export ONEPACK_KEYRING=off
tls() { "$bin/onepack" --url "$tls_url" --token-env ADMIN_TOKEN "$@"; }

step "TLS: an unknown CA is rejected (exit 7, TLS_FAILED)"
expect_failure 7 TLS_FAILED tls whoami

step "TLS: accepted with --ca-cert and with a context CA"
tls --ca-cert "$pki/ca.crt" whoami >/dev/null || fail "whoami with --ca-cert"
onepack context add tls --url "$tls_url" --ca-cert "$pki/ca.crt" >/dev/null
onepack --context tls --token-env ADMIN_TOKEN whoami >/dev/null || fail "whoami with the context CA"
onepack --context tls --token-env ADMIN_TOKEN doctor --json >"$work/doctor.json" \
  || fail "doctor over TLS: $(cat "$work/doctor.json")"

step "TLS: accepted with the CA in the system trust store"
sudo cp "$pki/ca.crt" "/usr/local/share/ca-certificates/$ca_name"
sudo update-ca-certificates >/dev/null
tls whoami >/dev/null || fail "whoami with the system trust store"
[ "$(tls whoami --json | json '["principal"]')" = admin ] || fail "whoami over TLS"

# --- 4. Other distributions -------------------------------------------------------------------

if [ -n "${ONEPACK_E2E_DISTROS:-}" ]; then
  file -b "$bin/onepack" | grep -Eq "statically linked|static-pie linked" || fail "ONEPACK_E2E_DISTROS needs a static binary"
  for image in $ONEPACK_E2E_DISTROS; do
    step "Distribution $image: --version, whoami over HTTP and over TLS with --ca-cert"
    docker run --rm --network host -e ADMIN_TOKEN -e ONEPACK_KEYRING=off -e ONEPACK_CONFIG_DIR=/tmp/opk \
      -v "$bin/onepack:/usr/local/bin/onepack:ro" -v "$pki/ca.crt:/ca.crt:ro" "$image" sh -c "
        set -e
        onepack --version
        onepack --url $http_url --token-env ADMIN_TOKEN whoami >/dev/null
        onepack --url $tls_url --ca-cert /ca.crt --token-env ADMIN_TOKEN whoami >/dev/null
      " >/dev/null || fail "the CLI does not work on $image"
  done
fi

echo "LINUX CLI E2E OK: system keychain (Secret Service), headless without a keychain, TLS"
echo "  (unknown CA rejected, --ca-cert, context CA, system trust store)${ONEPACK_E2E_DISTROS:+, distributions: $ONEPACK_E2E_DISTROS}."
