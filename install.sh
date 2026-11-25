#!/bin/sh
# Installs the onepack CLI from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/Khr0x/onepack/v0.1.0/install.sh | sh
#
# Variables:
#   ONEPACK_VERSION=0.1.0          version to install (without the "v")
#   ONEPACK_INSTALL_DIR=~/.local/bin
#   ONEPACK_INSTALL_SERVER=1       also installs onepackd (Linux only)
#   ONEPACK_BASE_URL=...           alternative base (mirror or file://)
set -eu
umask 077

fail() {
  printf 'onepack install: %s\n' "$*" >&2
  exit 1
}

VERSION="${ONEPACK_VERSION:-0.1.0}"
VERSION="${VERSION#v}"
BASE="${ONEPACK_BASE_URL:-https://github.com/Khr0x/onepack/releases/download}"
BASE="${BASE%/}"
INSTALL_DIR="${ONEPACK_INSTALL_DIR:-$HOME/.local/bin}"
SERVER="${ONEPACK_INSTALL_SERVER:-0}"

case "$SERVER" in
  0) ;;
  1) ;;
  *) fail "ONEPACK_INSTALL_SERVER must be 0 or 1" ;;
esac

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"
command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 \
  || fail "sha256sum or shasum is required"

os=$(uname -s)
case "$os" in
  Linux)
    case "$(uname -m)" in
      x86_64|amd64) target=x86_64-unknown-linux-musl ;;
      aarch64|arm64) target=aarch64-unknown-linux-musl ;;
      *) fail "unsupported Linux architecture: $(uname -m)" ;;
    esac
    ;;
  Darwin)
    case "$(uname -m)" in
      x86_64) target=x86_64-apple-darwin ;;
      arm64|aarch64) target=aarch64-apple-darwin ;;
      *) fail "unsupported macOS architecture: $(uname -m)" ;;
    esac
    ;;
  *) fail "unsupported OS: $(uname -s); see docs/install-cli.md" ;;
esac

if [ "$SERVER" = 1 ] && [ "$os" != Linux ]; then
  fail "ONEPACK_INSTALL_SERVER=1 is only supported on Linux; see docs/runbooks/install.md"
fi

cli_asset="onepack-$VERSION-$target.tar.gz"
server_asset="onepackd-$VERSION-$target.tar.gz"
release="$BASE/v$VERSION"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

verify() {
  asset=$1
  awk -v asset="$asset" '$2 == asset { print }' "$tmp/SHA256SUMS" > "$tmp/$asset.sha256"
  [ -s "$tmp/$asset.sha256" ] || fail "no checksum for $asset in SHA256SUMS"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$tmp" && sha256sum -c "$asset.sha256") \
      || fail "checksum verification failed for $asset"
  else
    (cd "$tmp" && shasum -a 256 -c "$asset.sha256") \
      || fail "checksum verification failed for $asset"
  fi
}

printf 'Downloading onepack %s for %s...\n' "$VERSION" "$target"
curl -fL --retry 3 -o "$tmp/$cli_asset" "$release/$cli_asset" \
  || fail "could not download $release/$cli_asset"
if [ "$SERVER" = 1 ]; then
  printf 'Downloading onepackd %s for %s...\n' "$VERSION" "$target"
  curl -fL --retry 3 -o "$tmp/$server_asset" "$release/$server_asset" \
    || fail "could not download $release/$server_asset"
fi
curl -fL --retry 3 -o "$tmp/SHA256SUMS" "$release/SHA256SUMS" \
  || fail "could not download $release/SHA256SUMS"

verify "$cli_asset"
if [ "$SERVER" = 1 ]; then
  verify "$server_asset"
fi

tar -xzf "$tmp/$cli_asset" -C "$tmp"
src="$tmp/onepack-$VERSION-$target"
[ -f "$src/onepack" ] || fail "the archive does not contain onepack"

mkdir -p "$INSTALL_DIR"
install -m 0755 "$src/onepack" "$INSTALL_DIR/onepack"
if [ "$SERVER" = 1 ]; then
  tar -xzf "$tmp/$server_asset" -C "$tmp"
  src="$tmp/onepackd-$VERSION-$target"
  [ -f "$src/onepackd" ] || fail "the server archive does not contain onepackd"
  install -m 0755 "$src/onepackd" "$INSTALL_DIR/onepackd"
  printf 'Installed onepackd at %s/onepackd\n' "$INSTALL_DIR"
fi

printf 'Installed onepack %s at %s/onepack\n' "$VERSION" "$INSTALL_DIR"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) printf 'Add it to PATH:\n  export PATH="%s:$PATH"\n' "$INSTALL_DIR" ;;
esac
"$INSTALL_DIR/onepack" --version
