#!/usr/bin/env bash
# Builds, signs, packages and verifies the macOS release assets for ONE architecture.
#
#   scripts/package-macos.sh --version VERSION --target aarch64-apple-darwin|x86_64-apple-darwin --out DIRECTORY
#
# Needs macOS with the Xcode command line tools, a Rust toolchain with the target, Node with
# `npm ci --prefix app` already run, and python3. Steps:
#
#  1. `scripts/check-version.py` must print exactly VERSION.
#  2. `scripts/check-macos-production.sh` (the macOS readiness gate) must pass.
#  3. Signing mode: none of the six Developer ID variables set = ad-hoc; all six set = Developer
#     ID + notarization; anything in between fails (never a silent downgrade). The variables are
#     Tauri's standard names: APPLE_CERTIFICATE (base64 .p12), APPLE_CERTIFICATE_PASSWORD,
#     APPLE_SIGNING_IDENTITY, APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID.
#     Their values are never printed; only names are.
#  4. `tauri build --target TARGET --bundles app` with the embedded frontend (Tauri runs the
#     `beforeBuildCommand` and enables `custom-protocol`), the Mac override
#     `app/src-tauri/tauri.macos.conf.json` and MACOSX_DEPLOYMENT_TARGET=13.0. Tauri itself never
#     signs here: the Apple variables are removed from its environment.
#  5. Sign `LingoPop Preview.app`: ad-hoc `codesign -s - --options runtime --timestamp=none`, or
#     Developer ID with the hardened runtime and a secure timestamp from a temporary keychain,
#     then notarytool + stapler.
#  6. Stage DIRECTORY/LingoPop-VERSION-macos-ARCH.zip (ditto, the app only) and .dmg (hdiutil
#     UDZO with the app and an /Applications link; Developer ID: signed, notarized, stapled), plus
#     SHA256SUMS-macos-ARCH.txt.
#  7. Verify: the portable verifier (`scripts/verify-release-assets.py verify`), then natively the
#     app extracted from the zip AND the app copied out of the mounted dmg (bundle id, version,
#     minimum OS, LSUIElement, `lipo` architecture, `otool` deployment target, `codesign
#     --verify --strict`, ad-hoc or Developer ID signature with the hardened runtime), and
#     `spctl --assess` on quarantined copies (ad-hoc: rejected; Developer ID: accepted as
#     Notarized Developer ID).
#  8. Write DIRECTORY/signing-macos-ARCH.txt (`adhoc` or `developer-id`) for the release notes.
#
# DIRECTORY must be empty or absent. Scratch files live under target/macos-package/ only; the
# quarantine attribute is set only on copies there. Nothing is installed, launched or opened.
set -euo pipefail

usage() {
  echo "usage: $0 --version VERSION --target aarch64-apple-darwin|x86_64-apple-darwin --out DIRECTORY" >&2
  exit 2
}
die() {
  echo "error: $*" >&2
  exit 1
}

version="" target="" out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) [ "$#" -ge 2 ] || usage; version=$2; shift 2 ;;
    --target) [ "$#" -ge 2 ] || usage; target=$2; shift 2 ;;
    --out) [ "$#" -ge 2 ] || usage; out=$2; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$version" ] && [ -n "$target" ] && [ -n "$out" ] || usage

case "$target" in
  aarch64-apple-darwin) slug=macos-arm64 arch=arm64 ;;
  x86_64-apple-darwin) slug=macos-x64 arch=x86_64 ;;
  *) die "unsupported target $target (expected aarch64-apple-darwin or x86_64-apple-darwin)" ;;
esac
[ "$(uname -s)" = Darwin ] || die "package-macos.sh runs on macOS only"

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

readonly bundle_id=app.lingopop.tauri.macos
readonly app_name="LingoPop Preview.app"
readonly min_os=13.0
readonly name="LingoPop-$version-$slug"

# --- 1. version --------------------------------------------------------------------------------
repo_version=$(python3 scripts/check-version.py) || die "the repository versions disagree (see above)"
[ "$repo_version" = "$version" ] || die "--version $version does not match the repository version $repo_version"

# --- 2. readiness gate -------------------------------------------------------------------------
bash scripts/check-macos-production.sh

# --- 3. signing mode ---------------------------------------------------------------------------
signing_vars=(APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID)
present=0
missing=()
for var in "${signing_vars[@]}"; do
  if [ -n "${!var:-}" ]; then
    present=$((present + 1))
  else
    missing+=("$var")
  fi
done
if [ "$present" -eq 0 ]; then
  mode=adhoc
elif [ "$present" -eq "${#signing_vars[@]}" ]; then
  mode=developer-id
else
  die "incomplete Developer ID signing configuration: ${missing[*]} not set (set all six or none)"
fi
echo "signing mode: $mode"

# --- output and scratch directories ------------------------------------------------------------
mkdir -p "$out"
out=$(cd "$out" && pwd)
if [ -n "$(ls -A "$out")" ]; then
  die "$out is not empty"
fi
work="$root/target/macos-package/$slug"
rm -rf "$work"
mkdir -p "$work"

mounted=""
keychain=""
original_keychains=()
cleanup() {
  if [ -n "$mounted" ]; then
    hdiutil detach "$mounted" -quiet || hdiutil detach "$mounted" -force -quiet || true
  fi
  if [ -n "$keychain" ]; then
    if [ "${#original_keychains[@]}" -gt 0 ]; then
      security list-keychains -d user -s "${original_keychains[@]}" || true
    fi
    security delete-keychain "$keychain" || true
  fi
  rm -f "$work/certificate.p12"
}
trap cleanup EXIT

# --- 4. build ----------------------------------------------------------------------------------
[ -x app/node_modules/.bin/tauri ] || die "app/node_modules/.bin/tauri is missing: run npm ci --prefix app first"
export MACOSX_DEPLOYMENT_TARGET=$min_os
(
  cd app
  env -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD -u APPLE_SIGNING_IDENTITY \
    -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
    -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
    ./node_modules/.bin/tauri build --ci --target "$target" --bundles app \
    --config src-tauri/tauri.macos.conf.json -- --locked
)
built="$root/target/$target/release/bundle/macos/$app_name"
[ -d "$built" ] || die "tauri build did not produce $built"

app="$work/stage/$app_name"
mkdir -p "$work/stage"
ditto "$built" "$app"

# --- 5. sign -----------------------------------------------------------------------------------
json_field() { # json_field <key>: a top-level string of the JSON object on stdin, or nothing
  python3 -c 'import json, sys
try:
    value = json.load(sys.stdin).get(sys.argv[1], "")
except (ValueError, AttributeError):
    value = ""
print(value if isinstance(value, str) else "")' "$1"
}
notarize() { # notarize <file>: submit, wait and require status Accepted
  # The credentials come from the environment; on an ephemeral runner passing them as arguments
  # is acceptable (docs/RELEASING.md). notarytool's JSON carries only id, status and message.
  local result status=0 id state
  result=$(xcrun notarytool submit "$1" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" --wait --timeout 30m --output-format json) || status=$?
  id=$(printf '%s' "$result" | json_field id)
  state=$(printf '%s' "$result" | json_field status)
  echo "notarization of $(basename "$1"): status ${state:-unknown}, id ${id:-none}, exit $status"
  if [ "$state" != Accepted ]; then
    if [ -n "$id" ]; then
      echo "notarytool log $id:" >&2
      xcrun notarytool log "$id" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
        --team-id "$APPLE_TEAM_ID" >&2 || true
    else
      printf 'notarytool output: %s\n' "$result" >&2
    fi
    die "notarization of $(basename "$1") was not accepted (status ${state:-unknown})"
  fi
}
if [ "$mode" = adhoc ]; then
  codesign --force --sign - --options runtime --timestamp=none "$app"
else
  keychain="$work/signing.keychain-db"
  keychain_password=$(uuidgen)
  while IFS= read -r line; do
    line=${line#"${line%%[![:space:]]*}"}
    line=${line%\"}
    original_keychains+=("${line#\"}")
  done < <(security list-keychains -d user)
  security create-keychain -p "$keychain_password" "$keychain"
  security set-keychain-settings -lut 3600 "$keychain"
  security unlock-keychain -p "$keychain_password" "$keychain"
  printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$work/certificate.p12"
  security import "$work/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" \
    -T /usr/bin/codesign >/dev/null
  rm -f "$work/certificate.p12"
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" \
    "$keychain" >/dev/null
  security list-keychains -d user -s "$keychain" "${original_keychains[@]}"
  codesign --force --sign "$APPLE_SIGNING_IDENTITY" --keychain "$keychain" --options runtime \
    --timestamp "$app"
  ditto -c -k --keepParent "$app" "$work/notarize-app.zip"
  notarize "$work/notarize-app.zip"
  xcrun stapler staple "$app"
fi

# --- 6. package --------------------------------------------------------------------------------
ditto -c -k --norsrc --noextattr --noqtn --noacl --keepParent "$app" "$out/$name.zip"

mkdir -p "$work/dmg"
ditto "$app" "$work/dmg/$app_name"
ln -s /Applications "$work/dmg/Applications"
created=0
for attempt in 1 2 3; do
  if hdiutil create -quiet -volname "LingoPop Preview" -srcfolder "$work/dmg" -fs HFS+ \
    -format UDZO -ov "$out/$name.dmg"; then
    created=1
    break
  fi
  echo "hdiutil create failed (attempt $attempt), retrying" >&2
  sleep 5
done
[ "$created" -eq 1 ] || die "hdiutil create failed"
if [ "$mode" = developer-id ]; then
  codesign --force --sign "$APPLE_SIGNING_IDENTITY" --keychain "$keychain" --timestamp "$out/$name.dmg"
  notarize "$out/$name.dmg"
  xcrun stapler staple "$out/$name.dmg"
fi

(cd "$out" && shasum -a 256 "$name.dmg" "$name.zip") > "$work/SHA256SUMS-$slug.txt"
mv "$work/SHA256SUMS-$slug.txt" "$out/SHA256SUMS-$slug.txt"

# --- 7. verify ---------------------------------------------------------------------------------
python3 scripts/verify-release-assets.py verify --version "$version" --platforms "$slug" \
  --manifest "SHA256SUMS-$slug.txt" "$out"

plist_value() { # plist_value <app> <key>
  plutil -extract "$2" raw -o - "$1/Contents/Info.plist" 2>/dev/null || true
}
check_app() { # check_app <app> <where>
  local candidate=$1 where=$2 exe details
  [ -d "$candidate" ] || die "$where: $app_name missing"
  [ "$(plist_value "$candidate" CFBundleIdentifier)" = "$bundle_id" ] || die "$where: bundle identifier is not $bundle_id"
  [ "$(plist_value "$candidate" CFBundleShortVersionString)" = "$version" ] || die "$where: bundle version is not $version"
  [ "$(plist_value "$candidate" LSMinimumSystemVersion)" = "$min_os" ] || die "$where: minimum macOS is not $min_os"
  [ "$(plist_value "$candidate" LSUIElement)" = true ] || die "$where: LSUIElement is not set"
  exe="$candidate/Contents/MacOS/$(plist_value "$candidate" CFBundleExecutable)"
  [ -f "$exe" ] && [ -x "$exe" ] || die "$where: bundle executable missing or not executable"
  [ "$(lipo -archs "$exe")" = "$arch" ] || die "$where: executable architecture is not $arch"
  [ "$(otool -l "$exe" | awk '$1 == "cmd" { build = ($2 == "LC_BUILD_VERSION") } build && $1 == "minos" { print $2; exit }')" = "$min_os" ] \
    || die "$where: executable deployment target is not $min_os"
  codesign --verify --strict --deep "$candidate" || die "$where: code signature does not verify"
  details=$(codesign -dv --verbose=2 "$candidate" 2>&1)
  printf '%s\n' "$details" | grep -q -E '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' \
    || die "$where: the hardened runtime is not enabled"
  if [ "$mode" = adhoc ]; then
    printf '%s\n' "$details" | grep -q -x 'Signature=adhoc' || die "$where: not ad-hoc signed"
  else
    printf '%s\n' "$details" | grep -q '^Authority=Developer ID Application' || die "$where: not signed with a Developer ID"
    printf '%s\n' "$details" | grep -q -x "TeamIdentifier=$APPLE_TEAM_ID" || die "$where: unexpected team identifier"
    xcrun stapler validate -q "$candidate" || die "$where: notarization ticket is not stapled"
  fi
}
quarantine() { # quarantine <path>: mark it as downloaded, like a browser does
  xattr -w com.apple.quarantine "0081;$(printf '%x' "$(date +%s)");LingoPop release check;" "$1"
}
assess() { # assess <where> <spctl args...>: the outcome must match the signing mode
  local where=$1 result status=0
  shift
  result=$(spctl --assess -vvv "$@" 2>&1) || status=$?
  if printf '%s\n' "$result" | grep -q 'override=security disabled'; then
    die "$where: Gatekeeper assessments are disabled on this machine (spctl reports override=security disabled), so the signing result cannot be checked; re-enable them with 'sudo spctl --global-enable' or use a machine with Gatekeeper on"
  fi
  if [ "$mode" = adhoc ]; then
    if [ "$status" -eq 0 ] || ! printf '%s\n' "$result" | grep -q 'rejected'; then
      die "$where: Gatekeeper did not reject the ad-hoc build as expected: $result"
    fi
  elif [ "$status" -ne 0 ] || ! printf '%s\n' "$result" | grep -q 'source=Notarized Developer ID'; then
    die "$where: Gatekeeper did not accept the notarized build: $result"
  fi
  echo "$where: spctl: $(printf '%s' "$result" | tr '\n' ' ')"
}

# The zip, as a user downloads it.
mkdir -p "$work/quarantine" "$work/from-zip"
cp "$out/$name.zip" "$out/$name.dmg" "$work/quarantine/"
quarantine "$work/quarantine/$name.zip"
quarantine "$work/quarantine/$name.dmg"
ditto -x -k "$work/quarantine/$name.zip" "$work/from-zip"
[ "$(ls -A "$work/from-zip")" = "$app_name" ] || die "zip: must contain only $app_name"
check_app "$work/from-zip/$app_name" zip
xattr -w -r com.apple.quarantine "0081;$(printf '%x' "$(date +%s)");LingoPop release check;" "$work/from-zip/$app_name"
assess zip --type execute "$work/from-zip/$app_name"

# The dmg: mount read-only, check the volume, then "install" a copy into the scratch directory.
mkdir -p "$work/mnt" "$work/installed"
hdiutil attach -quiet -nobrowse -readonly -noautoopen -mountpoint "$work/mnt" "$work/quarantine/$name.dmg"
mounted="$work/mnt"
[ -L "$work/mnt/Applications" ] && [ "$(readlink "$work/mnt/Applications")" = /Applications ] \
  || die "dmg: the /Applications link is missing"
check_app "$work/mnt/$app_name" dmg
ditto "$work/mnt/$app_name" "$work/installed/$app_name"
hdiutil detach "$work/mnt" -quiet
mounted=""
check_app "$work/installed/$app_name" "installed copy"
if [ "$mode" = adhoc ]; then
  # An ad-hoc dmg carries no signature of its own; Gatekeeper rejects opening it.
  assess dmg --type open --context context:primary-signature "$work/quarantine/$name.dmg"
else
  xcrun stapler validate -q "$out/$name.dmg" || die "dmg: notarization ticket is not stapled"
  assess dmg --type open --context context:primary-signature "$work/quarantine/$name.dmg"
fi

# --- 8. signing mode for the release notes -----------------------------------------------------
printf '%s\n' "$mode" > "$out/signing-$slug.txt"

echo "packaged $name ($mode):"
cat "$out/SHA256SUMS-$slug.txt"
