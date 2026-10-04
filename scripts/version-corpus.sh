#!/usr/bin/env bash
# Regenera el corpus de versiones con NuGet.Versioning (ADR-008).
# Con --check falla si el corpus versionado no coincide con el generado.
set -euo pipefail
dir=tests/conformance-dotnet/version-corpus
dotnet run --project "$dir" -c Release -- "$dir/inputs.json" "$dir/corpus.json"
if [ "${1:-}" = "--check" ]; then
  git diff --exit-code -- "$dir/corpus.json"
fi
