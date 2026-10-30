#!/usr/bin/env bash
# Compiles the real macOS model files with swiftc and runs the fixture generator / checker.
#   run.sh generate          write crates/lingopop-core/tests/fixtures/mac/*
#   run.sh check <dir>       decode history.json, usage.json, secrets.payload.json from <dir> with the Swift models
# macOS only. The mac repo is read-only here: nothing is ever written into it.
set -euo pipefail

MAC_REPO="${LINGOPOP_MAC_REPO:-$HOME/Works/slava/lingo-pop}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FIXTURE_DIR="$REPO_ROOT/crates/lingopop-core/tests/fixtures/mac"
BUILD_DIR="/tmp/lingopop-mac-fixtures"

if [[ ! -d "$MAC_REPO/Models" ]]; then
  echo "mac repo not found at $MAC_REPO (set LINGOPOP_MAC_REPO)" >&2
  exit 2
fi
if ! command -v swiftc >/dev/null 2>&1; then
  echo "swiftc not found: this script needs the Swift toolchain on macOS" >&2
  exit 2
fi

mode="${1:-}"
case "$mode" in
  generate | check) ;;
  *)
    echo "usage: $0 generate | check <dir>" >&2
    exit 2
    ;;
esac

SOURCES=(
  Models/TranslationHistory.swift
  Models/ProviderConfig.swift
  Models/AIOperation.swift
  Models/ProviderKeyedDictionary.swift
  Models/ProviderSettingsStorage.swift
  Models/Languages.swift
  Models/LanguageCodes.swift
  Core/CloudMerge.swift
  Core/CloudStorage.swift
  Core/UsageStore.swift
  Core/UsagePricing.swift
  Core/PromptBuilder.swift
)
paths=()
for f in "${SOURCES[@]}"; do paths+=("$MAC_REPO/$f"); done

mkdir -p "$BUILD_DIR"
# main.swift must be named main.swift for top-level code; no -parse-as-library.
mkdir -p "$BUILD_DIR/src"
cp "$SCRIPT_DIR/main.swift" "$BUILD_DIR/src/main.swift"
swiftc -O -o "$BUILD_DIR/gen" "${paths[@]}" "$BUILD_DIR/src/main.swift"

# Swift's JSONEncoder key order depends on per-process hash seeding; fix it so regenerating is byte-stable.
export SWIFT_DETERMINISTIC_HASHING=1

if [[ "$mode" == "generate" ]]; then
  mkdir -p "$FIXTURE_DIR"
  "$BUILD_DIR/gen" generate "$FIXTURE_DIR"
  git -C "$MAC_REPO" rev-parse HEAD >"$FIXTURE_DIR/SOURCE.txt"
  echo "wrote fixtures to $FIXTURE_DIR (mac commit $(cat "$FIXTURE_DIR/SOURCE.txt"))"
else
  dir="${2:-}"
  if [[ -z "$dir" ]]; then
    echo "usage: $0 check <dir>" >&2
    exit 2
  fi
  "$BUILD_DIR/gen" check "$dir"
fi
