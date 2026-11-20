#!/usr/bin/env bash
# Print the changelog section for a version, without its heading.
# Usage: scripts/changelog-notes.sh <version> [CHANGELOG.md]
set -euo pipefail

version="${1:?usage: scripts/changelog-notes.sh <version> [file]}"
file="${2:-CHANGELOG.md}"
version="${version#v}"

python3 - "$version" "$file" <<'PY'
import re
import sys
from pathlib import Path

version, path = sys.argv[1], sys.argv[2]
text = Path(path).read_text(encoding="utf-8")
pattern = re.compile(
    rf"^## \[{re.escape(version)}\][^\n]*\n(.*?)(?=^## \[|\Z)",
    re.MULTILINE | re.DOTALL,
)
match = pattern.search(text)
if not match:
    sys.exit(f"no changelog section for {version} in {path}")
notes = match.group(1).strip()
if not notes:
    sys.exit(f"empty changelog section for {version} in {path}")
print(notes)
PY
