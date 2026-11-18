#!/usr/bin/env bash
# Regenerates the version corpus with NuGet.Versioning (ADR-008).
# With --check it fails if the versioned corpus does not match the generated one.
set -euo pipefail
dir=tests/conformance-dotnet/version-corpus
dotnet run --project "$dir" -c Release -- "$dir/inputs.json" "$dir/corpus.json"
if [ "${1:-}" = "--check" ]; then
  git diff --exit-code -- "$dir/corpus.json"
fi
