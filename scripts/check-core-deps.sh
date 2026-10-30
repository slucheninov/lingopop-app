#!/usr/bin/env bash
# Guard: lingopop-core must stay OS-agnostic. Exit 0 = clean, exit 1 = offending dependency.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

status=0

# (a) Direct dependencies: no OS / UI crates by name prefix.
direct=$(cargo tree -p lingopop-core -e normal,build --depth 1 --prefix none)
bad_direct=$(printf '%s\n' "$direct" | awk '
  { name = $1 }
  name ~ /^(windows|winapi|tauri|objc|cocoa|core-foundation)/ || name == "core-graphics" { print }
')
if [ -n "$bad_direct" ]; then
  echo "error: lingopop-core has forbidden direct dependencies:" >&2
  printf '%s\n' "$bad_direct" >&2
  status=1
fi

# (b) Whole tree: no tauri*, objc*, core-foundation*, windows, winapi, cocoa.
# A transitive windows-sys (e.g. via getrandom or tempfile on Windows) stays legitimate.
full=$(cargo tree -p lingopop-core -e normal,build --prefix none)
bad_tree=$(printf '%s\n' "$full" | awk '
  { name = $1 }
  name ~ /^(tauri|objc|core-foundation)/ || name == "windows" || name == "winapi" || name == "cocoa" { print }
')
if [ -n "$bad_tree" ]; then
  echo "error: lingopop-core dependency tree contains forbidden crates:" >&2
  printf '%s\n' "$bad_tree" >&2
  status=1
fi

exit "$status"
