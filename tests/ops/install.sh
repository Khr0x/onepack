#!/usr/bin/env bash
# install.sh: CLI-only by default, both binaries in server mode, and explicit
# failures when the server archive, its checksum or the platform is unsupported.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() { echo "INSTALL FAILED: $*" >&2; exit 1; }
step() { echo "==> $*"; }

hash_files() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$@"
  else
    shasum -a 256 "$@"
  fi
}

mirror="$work/mirror/v0.1.0"
build="$work/build"
mkdir -p "$mirror" "$build" "$work/fake"

make_archive() {
  bin=$1
  dir="$build/$bin-0.1.0-x86_64-unknown-linux-musl"
  mkdir -p "$dir"
  printf '#!/bin/sh\necho "%s 0.1.0"\n' "$bin" >"$dir/$bin"
  chmod +x "$dir/$bin"
  tar -czf "$mirror/$bin-0.1.0-x86_64-unknown-linux-musl.tar.gz" \
    -C "$build" "$(basename "$dir")"
}

make_archive onepack
make_archive onepackd
(cd "$mirror" && hash_files onepack-*.tar.gz onepackd-*.tar.gz >SHA256SUMS)

# install.sh selects the archive from uname; pin Linux x86-64 for the test.
printf '#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo x86_64;; *) /usr/bin/uname "$@";; esac\n' \
  >"$work/fake/uname"
chmod +x "$work/fake/uname"

run() {
  PATH="$work/fake:$PATH" \
    ONEPACK_VERSION=0.1.0 \
    ONEPACK_BASE_URL="file://$work/mirror" \
    ONEPACK_INSTALL_DIR="$1" \
    sh "$root/install.sh"
}

step "CLI only by default"
mkdir "$work/cli"
run "$work/cli" >/dev/null
[ -x "$work/cli/onepack" ] || fail "onepack was not installed"
[ ! -e "$work/cli/onepackd" ] || fail "onepackd must not be installed by default"

step "Server mode installs both binaries"
mkdir "$work/server"
ONEPACK_INSTALL_SERVER=1 run "$work/server" >/dev/null
[ -x "$work/server/onepack" ] || fail "onepack was not installed"
[ -x "$work/server/onepackd" ] || fail "onepackd was not installed in server mode"

step "Server archive without the binary preserves both previous binaries"
mkdir -p "$work/previous" "$work/badserver/onepackd-0.1.0-x86_64-unknown-linux-musl"
printf 'previous cli\n' >"$work/previous/onepack"
printf 'previous server\n' >"$work/previous/onepackd"
chmod +x "$work/previous/onepack" "$work/previous/onepackd"
printf 'not the server binary\n' >"$work/badserver/onepackd-0.1.0-x86_64-unknown-linux-musl/README"
cp "$work/previous/onepack" "$work/expected-cli"
cp "$work/previous/onepackd" "$work/expected-server"
tar -czf "$mirror/onepackd-0.1.0-x86_64-unknown-linux-musl.tar.gz" \
  -C "$work/badserver" onepackd-0.1.0-x86_64-unknown-linux-musl
(cd "$mirror" && hash_files onepack-*.tar.gz onepackd-*.tar.gz >SHA256SUMS)
if ONEPACK_INSTALL_SERVER=1 run "$work/previous" >/dev/null 2>&1; then
  fail "server mode succeeded without the onepackd binary"
fi
cmp "$work/previous/onepack" "$work/expected-cli" \
  || fail "the CLI was replaced after the server archive failed"
cmp "$work/previous/onepackd" "$work/expected-server" \
  || fail "the previous onepackd was replaced after the server archive failed"
make_archive onepackd
(cd "$mirror" && hash_files onepack-*.tar.gz onepackd-*.tar.gz >SHA256SUMS)

step "Missing server archive fails"
mkdir "$work/missing"
mv "$mirror/onepackd-0.1.0-x86_64-unknown-linux-musl.tar.gz" "$work/onepackd.tar.gz"
if ONEPACK_INSTALL_SERVER=1 run "$work/missing" >/dev/null 2>&1; then
  fail "server mode succeeded without the server archive"
fi
mv "$work/onepackd.tar.gz" "$mirror/onepackd-0.1.0-x86_64-unknown-linux-musl.tar.gz"

step "Missing server checksum fails"
cp "$mirror/SHA256SUMS" "$work/SHA256SUMS"
grep -v 'onepackd-' "$work/SHA256SUMS" >"$mirror/SHA256SUMS"
mkdir "$work/missing-checksum"
if ONEPACK_INSTALL_SERVER=1 run "$work/missing-checksum" >/dev/null 2>&1; then
  fail "server mode succeeded without a server checksum"
fi
cp "$work/SHA256SUMS" "$mirror/SHA256SUMS"

step "macOS server mode fails clearly"
mkdir "$work/macos"
printf '#!/bin/sh\ncase "$1" in -s) echo Darwin;; -m) echo arm64;; *) /usr/bin/uname "$@";; esac\n' \
  >"$work/fake/uname"
if ONEPACK_INSTALL_SERVER=1 run "$work/macos" >"$work/macos.log" 2>&1; then
  fail "server mode succeeded on macOS"
fi
grep -q 'only supported on Linux' "$work/macos.log" || fail "the macOS error is not clear"

echo "INSTALL OK"
