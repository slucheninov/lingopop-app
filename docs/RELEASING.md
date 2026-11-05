# Releasing LingoPop

Releases are GitHub pre-releases built by `.github/workflows/release.yml` from a `v<version>` tag on `main`. The install instructions for users are in [INSTALL.md](INSTALL.md); the release workflow copies them into every release's notes.

## Steps

1. Bump the version to the same value in `Cargo.toml` (`[workspace.package]`), `app/src-tauri/tauri.conf.json` and `app/package.json`, then update `app/package-lock.json` with `npm install --package-lock-only --prefix app` and `Cargo.lock` with `cargo check --workspace`. Never add a `version` to `app/src-tauri/tauri.macos.conf.json`.
2. Run `python3 scripts/check-version.py`: it prints the version when all of them agree (CI and every release leg run it too).
3. Move the CHANGELOG `Unreleased` entries into a `## [<version>] - <date>` section. That section becomes the top of the release notes.
4. Check `release-platforms.json` (see below).
5. Merge through a PR, then push the tag `v<version>` on `main`. Never push a tag as a test: use `workflow_dispatch` to try the packaging.

## What the workflow does

| Job | Runner | Output artifact |
|---|---|---|
| `build-windows-portable` | `windows-latest` | `LingoPop-<version>-windows-x64`: `LingoPop-<version>-windows-x64-portable.exe`, `SHA256SUMS-windows-x64.txt` |
| `build-macos-arm64` | `macos-26` (arm64) | `LingoPop-<version>-macos-arm64`: `LingoPop-<version>-macos-arm64.dmg`, `.zip`, `SHA256SUMS-macos-arm64.txt`, `signing-macos-arm64.txt` |
| `build-macos-x64` | `macos-26-intel` (x64) | `LingoPop-<version>-macos-x64`: the same for x64 |
| `publish-prerelease` | `ubuntu-latest` | the GitHub pre-release, created as a draft and published after the upload check (tag runs only) |

The two Mac legs are one matrix job (`build-macos`) with those display names. The runner labels are explicit because `macos-latest` is arm64 and moves with new images; they were checked against the [runner-images README](https://github.com/actions/runner-images) and the [GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) on 2026-11-05. Each leg also checks `uname -m`, so a label that changes architecture fails the leg instead of shipping a cross build.

- Every leg runs `scripts/check-version.py` (its output is captured with `tr -d '\r'`, because Python on Windows prints CRLF, and must not be empty) (with `--tag` on a tag run) and the frontend tests.
- The Windows leg builds `lingopop.exe` with `--features custom-protocol` and verifies the staged exe with `scripts/verify-release-assets.py verify`.
- Each Mac leg runs `scripts/package-macos.sh --version <version> --target <target> --out dist-release`: version check, the macOS readiness gate (`scripts/check-macos-production.sh`), `tauri build --bundles app` with the Mac override (`bundle.active` is true only there; Windows stays a portable exe), signing, the `.zip` and `.dmg`, the per-platform checksums, then the portable verifier plus native checks: the app extracted from the zip and the app copied out of the mounted dmg must have the bundle id `app.lingopop.tauri.macos`, the release version, minimum macOS 13.0, `LSUIElement`, the leg's architecture (`lipo`), deployment target 13.0 (`otool`), a valid signature with the hardened runtime, and the expected Gatekeeper result on quarantined copies (`spctl`).
- `publish-prerelease` needs every leg (a failed Mac leg means no release at all), downloads all artifacts into separate directories and runs `scripts/verify-release-assets.py publish`: it reads `release-platforms.json`, copies the selected assets, concatenates the per-platform manifests into one `SHA256SUMS.txt`, verifies the exact asset set (names, sizes, checksums, zip Info.plist, Mach-O architecture / deployment target / signature, PE machine, dmg format), writes the notes (CHANGELOG section + the Installation part of `docs/INSTALL.md`) and only then runs `gh release create --draft --prerelease` with every asset. The next step downloads the draft's files again and requires exactly the verified set, byte for byte (plus `sha256sum -c`); only then does `gh release edit --draft=false --prerelease` publish it. A failure after the draft was created leaves a draft, never a public release with missing files: delete the draft (and only the draft) before re-running the job. It is the only job with `contents: write`.
- `workflow_dispatch` runs the three build legs and uploads their artifacts; nothing is published.

## `release-platforms.json`

`{"macosPreview": true}` attaches the macOS assets to the release. With `false` the Mac legs still build and verify, but the release (and its `SHA256SUMS.txt` and notes) has only the Windows exe. A missing or malformed file (any other key, a non-boolean value, invalid JSON) stops the publication.

## macOS signing

- **Default: ad hoc.** `codesign --sign - --options runtime --timestamp=none`. Gatekeeper blocks the first launch (users click Open Anyway, see [INSTALL.md](INSTALL.md)), and the notes include that paragraph. Each ad-hoc build has a new code identity, so after an update macOS may keep showing a permission as granted while it no longer applies: users remove LingoPop Preview from System Settings > Privacy & Security (Accessibility, Screen Recording) and grant it again, and the login keychain may ask again for LingoPop's own key. Tell testers this in the release notes of every ad-hoc release (the template does).
- **Developer ID + notarization** runs only when ALL six repository secrets are set: `APPLE_CERTIFICATE` (base64 of the `.p12`), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` (for example `Developer ID Application: Name (TEAMID)`), `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password) and `APPLE_TEAM_ID`. The script imports the certificate into a temporary keychain, signs the app with the hardened runtime and a secure timestamp, notarizes and staples the app and the dmg (`notarytool`, `stapler`), and expects `spctl` to accept both as Notarized Developer ID; the notes then omit the Gatekeeper paragraph. Some but not all of the six set fails the Mac legs (no silent downgrade to ad hoc). The values stay in GitHub secrets: the script prints only variable names, never values, and never runs with `set -x`. The certificate password and the Apple password are passed to `security import -P` and `notarytool --password` as command-line arguments, so other processes of the same user could see them in the process list while those commands run; that is accepted on GitHub's ephemeral, single-job runners (do not run the Developer ID path on a shared machine). Every `notarytool submit` must report `status: Accepted` (read from `--output-format json`); otherwise the script prints the submission's `notarytool log` and fails the leg.
- Every `spctl` check fails with a clear message when Gatekeeper assessments are disabled on the runner (`override=security disabled`), because the expected result can then not be checked.
- The installed app is `/Applications/LingoPop Preview.app`; its data is in `~/Library/Application Support/LingoPop-Tauri`.

## Local checks

```bash
python3 scripts/check-version.py
python3 -m unittest discover -s scripts/tests
actionlint
shellcheck scripts/*.sh
bash scripts/check-macos-production.sh
# macOS only, ad hoc, nothing is installed or launched (needs `npm ci --prefix app`):
bash scripts/package-macos.sh --version "$(python3 scripts/check-version.py)" --target aarch64-apple-darwin --out target/release-assets/macos-arm64
```

Before marking a release PR ready, run the workflow once with `workflow_dispatch` on the branch, download every artifact and check the executable architectures, the minimum macOS version and the checksum manifests.
