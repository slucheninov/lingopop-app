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
#  3. Signing mode (`select_signing_mode`; never a silent downgrade, values are never printed,
#     only variable names):
#     - MACOS_SIGNING_P12 (base64 .p12) and MACOS_SIGNING_P12_PASSWORD both set = `selfsigned`
#       (the pinned self-signed release certificate of release-signing.json); they take
#       precedence over the Developer ID variables, which are then ignored; only one of the two
#       set fails, naming the missing one.
#     - RELEASE_TAG_RUN=1 (set by release.yml on tag runs whose release carries the Mac assets)
#       without both = fail: `tag releases must be signed with the pinned certificate`. A tag run
#       never selects ad hoc or Developer ID (either would change the designated requirement,
#       and every installed copy would refuse the update). Any other non-empty value fails.
#     - Otherwise: none of the six Developer ID variables set = `adhoc`; all six set =
#       `developer-id` + notarization; anything in between fails. The variables are Tauri's
#       standard names: APPLE_CERTIFICATE (base64 .p12), APPLE_CERTIFICATE_PASSWORD,
#       APPLE_SIGNING_IDENTITY, APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID.
#     PACKAGE_MACOS_SELECT_ONLY=1 prints the selected mode and exits before anything else (the
#     unit tests use it, on any OS); PACKAGE_MACOS_PARSE_IDENTITIES=1 prints the lowercase SHA-1s
#     of `security find-identity` output read from stdin and exits.
#  4. `tauri build --target TARGET --bundles app` with the embedded frontend (Tauri runs the
#     `beforeBuildCommand` and enables `custom-protocol`), the Mac override
#     `app/src-tauri/tauri.macos.conf.json` and MACOSX_DEPLOYMENT_TARGET=13.0. Tauri itself never
#     signs here: the Apple and MACOS_SIGNING_* variables are removed from its environment.
#  5. Sign `LingoPop.app`: ad-hoc `codesign -s - --options runtime --timestamp=none`;
#     self-signed: import the .p12 into a temporary keychain, require exactly one code-signing
#     identity whose SHA-1 equals release-signing.json, `codesign --sign <sha1> --options runtime
#     --timestamp=none`; or Developer ID with the hardened runtime and a secure timestamp from a
#     temporary keychain, then notarytool + stapler.
#  6. Stage DIRECTORY/LingoPop-VERSION-macos-ARCH.zip (ditto, the app only) and .dmg (hdiutil
#     UDZO with the app and an /Applications link; Developer ID: signed, notarized, stapled), plus
#     SHA256SUMS-macos-ARCH.txt.
#  7. Verify: the portable verifier (`scripts/verify-release-assets.py verify`), then natively the
#     app extracted from the zip AND the app copied out of the mounted dmg (bundle id, version,
#     minimum OS, LSUIElement, `lipo` architecture, `otool` deployment target, `codesign
#     --verify --strict`, ad-hoc, self-signed (designated requirement exactly `identifier
#     "app.lingopop.tauri.macos" and certificate leaf = H"<pinned sha1>"`) or Developer ID
#     signature with the hardened runtime), and `spctl --assess` on quarantined copies (ad-hoc
#     and self-signed: rejected; Developer ID: accepted as Notarized Developer ID). The dmg is
#     signed only for Developer ID.
#  8. Write DIRECTORY/signing-macos-ARCH.txt (`adhoc`, `selfsigned` or `developer-id`) for the
#     release notes.
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

# select_signing_mode: sets `mode` from the environment (see step 3 above). Prints nothing but
# variable names; never a value.
select_signing_mode() {
  local var present=0 missing=()
  local p12_vars=(MACOS_SIGNING_P12 MACOS_SIGNING_P12_PASSWORD)
  local apple_vars=(APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID)
  case "${RELEASE_TAG_RUN:-}" in
    "" | 1) ;;
    *) die "RELEASE_TAG_RUN must be 1 or unset" ;;
  esac
  for var in "${p12_vars[@]}"; do
    if [ -n "${!var:-}" ]; then
      present=$((present + 1))
    else
      missing+=("$var")
    fi
  done
  if [ "$present" -eq "${#p12_vars[@]}" ]; then
    mode=selfsigned
    return
  fi
  if [ "$present" -ne 0 ]; then
    die "incomplete self-signed signing configuration: ${missing[*]} not set (set both or neither)"
  fi
  if [ "${RELEASE_TAG_RUN:-}" = 1 ]; then
    die "tag releases must be signed with the pinned certificate (MACOS_SIGNING_P12 and MACOS_SIGNING_P12_PASSWORD not set)"
  fi
  present=0
  missing=()
  for var in "${apple_vars[@]}"; do
    if [ -n "${!var:-}" ]; then
      present=$((present + 1))
    else
      missing+=("$var")
    fi
  done
  if [ "$present" -eq 0 ]; then
    mode=adhoc
  elif [ "$present" -eq "${#apple_vars[@]}" ]; then
    mode=developer-id
  else
    die "incomplete Developer ID signing configuration: ${missing[*]} not set (set all six or none)"
  fi
}

# identity_sha1s: the distinct SHA-1s (lowercase) of `security find-identity -p codesigning`
# output on stdin. Without `-v` on purpose: the self-signed release certificate is not trusted
# by the system policy (CSSMERR_TP_NOT_TRUSTED), so `-v` lists no valid identity, while codesign
# signs with it all the same.
identity_sha1s() {
  awk '$1 ~ /^[0-9]+\)$/ && $2 ~ /^[0-9A-Fa-f]{40}$/ { print tolower($2) }' | LC_ALL=C sort -u
}

if [ "${PACKAGE_MACOS_SELECT_ONLY:-}" = 1 ]; then
  select_signing_mode
  echo "signing mode: $mode"
  exit 0
fi
if [ "${PACKAGE_MACOS_PARSE_IDENTITIES:-}" = 1 ]; then
  identity_sha1s
  exit 0
fi

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
readonly app_name="LingoPop.app"
readonly min_os=13.0
readonly name="LingoPop-$version-$slug"

# --- 1. version --------------------------------------------------------------------------------
repo_version=$(python3 scripts/check-version.py) || die "the repository versions disagree (see above)"
[ "$repo_version" = "$version" ] || die "--version $version does not match the repository version $repo_version"

# --- 2. readiness gate -------------------------------------------------------------------------
bash scripts/check-macos-production.sh

# --- 3. signing mode ---------------------------------------------------------------------------
select_signing_mode
echo "signing mode: $mode"
pinned_sha1=""
pinned_requirement=""
if [ "$mode" = selfsigned ]; then
  # release-signing.json is exactly {"macosCertificateSha1": "<40 lowercase hex>"}; the same
  # value is core `updates::RELEASE_CERT_SHA1`, and the requirement below is the platform's
  # `pinned_requirement()` that every installed copy checks an update against.
  pinned_sha1=$(python3 -c 'import json, re, sys
try:
    data = json.load(open(sys.argv[1], encoding="utf-8"))
except (OSError, ValueError):
    sys.exit(1)
if not isinstance(data, dict) or set(data) != {"macosCertificateSha1"}:
    sys.exit(1)
value = data["macosCertificateSha1"]
if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{40}", value):
    sys.exit(1)
print(value)' release-signing.json) || die "release-signing.json is missing or invalid"
  pinned_requirement="identifier \"$bundle_id\" and certificate leaf = H\"$pinned_sha1\""
fi

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
    -u MACOS_SIGNING_P12 -u MACOS_SIGNING_P12_PASSWORD -u RELEASE_TAG_RUN \
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
import_identity() { # import_identity <base64 p12> <p12 password>: a temporary signing keychain
  # The EXIT trap restores the keychain search list and deletes the keychain and the .p12.
  local keychain_password line
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
  ( umask 077 && printf '%s' "$1" | base64 --decode > "$work/certificate.p12" ) \
    || die "the signing certificate is not valid base64"
  security import "$work/certificate.p12" -k "$keychain" -P "$2" -T /usr/bin/codesign >/dev/null \
    || die "the signing certificate could not be imported"
  rm -f "$work/certificate.p12"
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" \
    "$keychain" >/dev/null
  security list-keychains -d user -s "$keychain" "${original_keychains[@]}"
}
if [ "$mode" = adhoc ]; then
  codesign --force --sign - --options runtime --timestamp=none "$app"
elif [ "$mode" = selfsigned ]; then
  import_identity "$MACOS_SIGNING_P12" "$MACOS_SIGNING_P12_PASSWORD"
  identities=$(security find-identity -p codesigning "$keychain" | identity_sha1s)
  [ -n "$identities" ] || die "the self-signed .p12 holds no code-signing identity"
  [ "$(printf '%s\n' "$identities" | wc -l | tr -d ' ')" = 1 ] \
    || die "the self-signed .p12 holds more than one code-signing identity"
  [ "$identities" = "$pinned_sha1" ] \
    || die "the imported certificate $identities is not the pinned release certificate $pinned_sha1 (release-signing.json)"
  echo "signing identity: $identities (pinned)"
  codesign --force --sign "$pinned_sha1" --keychain "$keychain" --options runtime \
    --timestamp=none "$app"
else
  import_identity "$APPLE_CERTIFICATE" "$APPLE_CERTIFICATE_PASSWORD"
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
  if hdiutil create -quiet -volname "LingoPop" -srcfolder "$work/dmg" -fs HFS+ \
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
  local candidate=$1 where=$2 exe details requirement
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
  elif [ "$mode" = selfsigned ]; then
    if printf '%s\n' "$details" | grep -q -x 'Signature=adhoc'; then
      die "$where: signed ad hoc, not with the pinned certificate"
    fi
    requirement=$(codesign -d -r- "$candidate" 2>&1 | sed -n 's/^designated => //p')
    [ "$requirement" = "$pinned_requirement" ] \
      || die "$where: designated requirement is '$requirement', expected '$pinned_requirement'"
    codesign --verify --strict --deep -R="$pinned_requirement" "$candidate" \
      || die "$where: does not satisfy the pinned designated requirement"
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
  if [ "$mode" = adhoc ] || [ "$mode" = selfsigned ]; then
    # Neither is notarized; the self-signed certificate is not trusted by Gatekeeper either.
    if [ "$status" -eq 0 ] || ! printf '%s\n' "$result" | grep -q 'rejected'; then
      die "$where: Gatekeeper did not reject the $mode build as expected: $result"
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
if [ "$mode" != developer-id ]; then
  # An ad-hoc or self-signed release's dmg carries no signature of its own; Gatekeeper rejects
  # opening it.
  assess dmg --type open --context context:primary-signature "$work/quarantine/$name.dmg"
else
  xcrun stapler validate -q "$out/$name.dmg" || die "dmg: notarization ticket is not stapled"
  assess dmg --type open --context context:primary-signature "$work/quarantine/$name.dmg"
fi

# --- 8. signing mode for the release notes -----------------------------------------------------
printf '%s\n' "$mode" > "$out/signing-$slug.txt"

echo "packaged $name ($mode):"
cat "$out/SHA256SUMS-$slug.txt"
