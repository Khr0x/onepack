#!/usr/bin/env bash
# Verifies the crate boundaries defined in ADR-003.
# Checks normal, direct, and transitive dependencies.
set -euo pipefail

fail=0

forbid() { # forbidden-crate forbidden1 forbidden2 ...
  local crate=$1; shift
  local deps
  deps=$(cargo tree -p "$crate" -e normal --prefix none --format '{p}' | awk '{print $1}' | sort -u)
  for f in "$@"; do
    if grep -qx "$f" <<<"$deps"; then
      echo "ADR-003: $crate cannot depend on $f" >&2
      fail=1
    fi
  done
}

forbid onepack-core       onepack-nuget onepack-storage onepack-server onepack-cli sqlx axum
forbid onepack-nuget      onepack-server onepack-cli onepack-storage
forbid onepack-storage    onepack-nuget onepack-server onepack-cli axum
forbid onepack-api-client onepack-server onepack-storage
forbid onepack-cli        onepack-server onepack-storage

if [ "$fail" -eq 0 ]; then echo "Crate boundaries: OK"; fi
exit "$fail"
