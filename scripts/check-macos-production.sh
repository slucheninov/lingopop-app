#!/usr/bin/env bash
# Readiness gate for the macOS build: fails when the native macOS wiring, a release configuration
# or the macOS identity drifts back to something that must not ship. It complements the behaviour
# tests (it is cheap text inspection, not proof that the adapters work) and needs only POSIX tools.
#
#   scripts/check-macos-production.sh [repo-root]
#
# Exit 0 = ready, exit 1 = at least one problem (each printed to stderr).
set -euo pipefail

root=${1:-"$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"}
cd "$root"

status=0
fail() {
  echo "error: $*" >&2
  status=1
}

native=crates/lingopop-platform/src/native.rs
macos_conf=app/src-tauri/tauri.macos.conf.json

# (1) The macOS `native()` selects production adapters only.
if [ ! -f "$native" ]; then
  fail "$native is missing"
  body=""
else
  body=$(awk '
    /^#\[cfg\(target_os = "macos"\)\]$/ { armed = 1; next }
    armed && /^pub fn native\(\) -> Platform \{/ { inside = 1 }
    { armed = 0 }
    inside { print }
    inside && /^}/ { exit }
  ' "$native")
fi
if [ -z "$body" ]; then
  fail "the macOS native() was not found in $native (the guard would pass vacuously)"
else
  for forbidden in DevPlaintextProtector dev_protector Fake NoLastForeground AlwaysGranted Unsupported 'todo!' 'unimplemented!'; do
    if printf '%s\n' "$body" | grep -n -F -e "$forbidden" >&2; then
      fail "the macOS native() wires a dev, fake, no-op or unsupported adapter ($forbidden)"
    fi
  done
  for required in MacProtector MacLoginItem MacCapture MacOcr MacOverlay MacClipboard MacKeys MacForeground MacPermissions MacPopupWindow MacDismissWatcher; do
    if ! printf '%s\n' "$body" | grep -q -F -e "$required"; then
      fail "the macOS native() does not wire the production adapter $required"
    fi
  done
fi

# (2) No leftover dev-stub marker text in the macOS adapters (the old Unsupported stubs).
if [ -d crates/lingopop-platform/src ]; then
  for marker in 'screen capture on macOS' 'OCR on macOS' 'region overlay on macOS'; do
    if grep -rn -F -e "$marker" crates/lingopop-platform/src/macos "$native" >&2 2>/dev/null; then
      fail "dev stub marker \"$marker\" is still in the macOS adapters"
    fi
  done
fi

# (3) DevPlaintextProtector exists only behind its cfg. Any production source that mentions it
# (outside dev_protector.rs) must have the gate on the line before.
while IFS= read -r file; do
  case "$file" in */dev_protector.rs) continue ;; esac
  hits=$(awk '
    /DevPlaintextProtector|dev_protector/ && prev !~ /cfg\(any\(test, feature = "dev-plaintext"\)\)/ { print FILENAME ":" FNR ": " $0 }
    NF { prev = $0 }
  ' "$file")
  if [ -n "$hits" ]; then
    printf '%s\n' "$hits" >&2
    fail "DevPlaintextProtector is referenced without its cfg gate"
  fi
done < <(find crates app/src-tauri/src -path '*/src/*' -name '*.rs' -not -path '*/target/*' -not -path '*/tests/*' 2>/dev/null | sort)

# (4) The dev-plaintext feature is never enabled by a manifest, a workflow, a packaging config or
# a cargo config. The one allowed mention is its definition in the platform manifest.
while IFS= read -r file; do
  hits=$(grep -n -F -e 'dev-plaintext' "$file" | grep -v -x -E '[0-9]+:dev-plaintext = \[\]' || true)
  if [ -n "$hits" ]; then
    printf '%s: %s\n' "$file" "$hits" >&2
    fail "dev-plaintext is enabled in $file"
  fi
done < <(
  {
    find . -name Cargo.toml -not -path '*/target/*' -not -path '*/node_modules/*'
    find .github/workflows app/src-tauri .cargo scripts -type f \
      \( -name '*.yml' -o -name '*.yaml' -o -name '*.json' -o -name '*.toml' -o -name '*.sh' \) \
      -not -path '*/target/*' -not -name 'check-macos-production.sh' 2>/dev/null
    [ -f app/package.json ] && echo app/package.json
  } | sort -u
)

# (5) The macOS release configuration: approved identity, minimum OS, and never a `version` key
# (the shared version stays in Cargo.toml / tauri.conf.json). The values are read by JSON path
# (python3, standard library), so a key in the wrong place (a top-level `minimumSystemVersion`,
# or one inside a string) cannot satisfy the check.
json_value() { # json_value <file> <dotted.path>: the string at the path, empty when absent
  python3 -c '
import json, sys
node = json.load(open(sys.argv[1]))
for key in sys.argv[2].split("."):
    node = node.get(key) if isinstance(node, dict) else None
print(node if isinstance(node, str) else "")
' "$1" "$2"
}
if [ ! -f "$macos_conf" ]; then
  fail "$macos_conf is missing"
elif ! python3 -c 'import json, sys; json.load(open(sys.argv[1]))' "$macos_conf" 2>/dev/null; then
  fail "$macos_conf is not valid JSON"
else
  [ "$(json_value "$macos_conf" identifier)" = "app.lingopop.tauri.macos" ] \
    || fail "$macos_conf: identifier must be app.lingopop.tauri.macos"
  [ "$(json_value "$macos_conf" bundle.macOS.minimumSystemVersion)" = "13.0" ] \
    || fail "$macos_conf: bundle.macOS.minimumSystemVersion must be 13.0"
  if grep -n -E '"version"[[:space:]]*:' "$macos_conf" >&2; then
    fail "$macos_conf must not contain a version key"
  fi
fi

# (6) A release or packaging build never turns on every cargo feature: that would also enable
# `dev-plaintext` (and any future dev-only feature) in the shipped binary. Applies to release
# workflows and to scripts that package, bundle, sign or notarize.
while IFS= read -r file; do
  hits=$(grep -n -F -e '--all-features' "$file" | grep -v -E '^[0-9]+:[[:space:]]*#' || true)
  if [ -n "$hits" ]; then
    printf '%s: %s\n' "$file" "$hits" >&2
    fail "$file builds with --all-features"
  fi
done < <(
  {
    find .github/workflows -type f \( -iname '*release*' -o -iname '*package*' -o -iname '*bundle*' -o -iname '*dist*' \) 2>/dev/null
    find scripts -type f \( -iname '*release*' -o -iname '*package*' -o -iname '*bundle*' -o -iname '*dmg*' -o -iname '*notar*' -o -iname '*sign*' \) \
      -not -name 'check-macos-production.sh' 2>/dev/null
  } | sort -u
)

if [ "$status" -eq 0 ]; then
  echo "macOS production readiness: ok"
fi
exit "$status"
