#!/usr/bin/env bash
# Verifica las fronteras entre crates definidas en ADR-003.
# Comprueba dependencias normales, directas y transitivas.
set -euo pipefail

fail=0

forbid() { # crate prohibido1 prohibido2 ...
  local crate=$1; shift
  local deps
  deps=$(cargo tree -p "$crate" -e normal --prefix none --format '{p}' | awk '{print $1}' | sort -u)
  for f in "$@"; do
    if grep -qx "$f" <<<"$deps"; then
      echo "ADR-003: $crate no puede depender de $f" >&2
      fail=1
    fi
  done
}

forbid onepack-core       onepack-nuget onepack-storage onepack-server onepack-cli sqlx axum
forbid onepack-nuget      onepack-server onepack-cli onepack-storage
forbid onepack-storage    onepack-nuget onepack-server onepack-cli axum
forbid onepack-api-client onepack-server onepack-storage
forbid onepack-cli        onepack-server onepack-storage

if [ "$fail" -eq 0 ]; then echo "Fronteras entre crates: OK"; fi
exit "$fail"
