# Swift Transition and Cross-platform Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Offer an explicit non-destructive Swift import and publish verified Mac preview assets alongside the Windows application.

**Architecture:** Import uses read-only native Swift access, pure field mapping and the storage transaction/recovery machinery. Releases retain one workspace version, build all selected native artifacts before publication, and retain a separate real-machine readiness gate.

**Tech Stack:** Existing Rust core/platform/shell, native UserDefaults/process queries, Tauri packaging, Python artifact validation and pinned GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-03-cross-platform-design.md`; all Global Constraints/shared gate in `2026-11-03-cross-platform.md` apply verbatim. Requires Storage Tasks 1–5 and Mac Tasks 6–17.

## Global Constraints

- Keep and freeze Swift: no repository archival, app uninstall, UserDefaults write, old login-item edit or old data mutation.
- No built-in iCloud synchronization. Only history and usage can use a selected folder; settings, hotkeys, credentials and provider settings stay device-local.
- API keys and provider overrides require re-entry. Never decode/copy the Swift encrypted secret store, machine key or salt, or auto-import dev plaintext secrets.
- Mac identifier `app.lingopop.tauri.macos`, local data `~/Library/Application Support/LingoPop-Tauri`, preview name LingoPop Preview; target macOS 13.0, arm64 and x86_64.
- Preserve Windows DPAPI, existing identity/directory and portable-exe names. All selected platform assets must succeed before combined publication.
- General availability needs Developer ID/notarization or an explicit owner-approved ad-hoc distribution exception. Ad-hoc preview is allowed with accurate instructions and real permission/upgrade coverage.

## Review Focus

- Old settings contain legacy provider secrets, unsupported Carbon chords or iCloud flags: import only whitelisted ordinary values, never expose excluded fields (Task 18).
- Swift is running, the Rust user edits settings after preview, or startup crashes halfway: import refuses/stops/retries without overwriting newer state or source files (Task 18).
- Missing/wrong-architecture assets or wrong checksums must prevent combined publication and automatic asset selection (Task 19).
- Signing/packaging is successful but TCC, login, installed paths or architecture runtime are untested: report those gaps explicitly and withhold a general-availability claim (Task 20).
- A Windows regression during the Mac port remains a release blocker rather than an accepted platform tradeoff (Tasks 19–20).

---

### Task 18: Explicit Swift import with whitelisted settings and journaled records

**Files:** Create core `swift_import.rs`/`tests/swift_import.rs`; platform `swift_source.rs`, `macos/swift_source.rs`, `fake/swift_source.rs`; shell `import/{mod.rs,commands.rs,transaction.rs,tests.rs}`; frontend `lib/importIpc.ts`, `settings/ImportSection.svelte`, `settings/import.test.ts`; wire existing platform exports, shell setup/settings tabs, the Storage Task 3 journal (`JournalKind::SwiftImport`) and `i18n/macos.json`. Fixtures in `crates/lingopop-core/tests/fixtures/swift-import/`.

**Interfaces:** Core `ImportError::{Unavailable, InvalidData, UnsupportedVersion, SwiftRunning, Conflict, RecoveryRequired}` has fixed text and redacted Debug; no raw bytes or fields.

Portable `SwiftSource: Send + Sync`:
- `is_running() -> Result<bool, ImportError>`: delegates to `SystemInfo::is_bundle_running("com.lingopop.app")` from Mac Task 9 (macOS `NSRunningApplication.runningApplications(withBundleIdentifier:)`, non-empty = running; a query error fails closed).
- `read_local_settings() -> Result<Option<Vec<u8>>, ImportError>`: UserDefaults key `LingoPop.settings.v1`.
- `read_local_records() -> Result<SwiftRecords { history: Option<Vec<u8>>, usage: Option<Vec<u8>> }, ImportError>`: keys `LingoPop.history.v1` (`Core/TranslationHistoryStore.swift:26`) and `LingoPop.usage.v1` (`Core/UsageStore.swift:42`), each the envelope JSON bytes written by `CloudMergeSync`.

The native reader accesses only domain `com.lingopop.app` and those three whitelisted keys, read-only, no full-domain export and no writes. It calls `CFPreferencesAppSynchronize` then `CFPreferencesCopyAppValue`; a running Swift app's unflushed state is the stated reason it must be quit. Raw bytes are short-lived and never reach the UI or logs. Records bytes are decoded by core's strict/legacy/BOM envelope decoders.

Source model:
- A Swift install with iCloud off is the primary case: history and usage live in UserDefaults; `history.json` / `usage.json` exist only in the iCloud container.
- A previously cloud-managed folder chosen through the Storage Task 5 opaque selection service is an optional second source, read with the same strict decoders.
- Swift secrets are not in Keychain: they are the AES-GCM `secrets.dat` (`Core/SecretStorage.swift`) plus possibly legacy plaintext `providers` inside `LingoPop.settings.v1`. The importer never opens `secrets.dat` and never copies `providers`.

Core `plan_swift_settings(bytes: &[u8], current: &Settings) -> Result<SwiftSettingsPreview, ImportError>` parses the actual Swift schema and maps only activeProvider, fallbacks, targetLanguage, supported hotkeys/legacy hotkey, limits, useAutoDetect, languagePairs, autoCopyOCR and sanitized magicPrompt. Launch-at-login is never turned on implicitly. `SwiftSettingsPreview` holds a whitelisted candidate, field differences, unsupported-hotkey operation IDs and a current-local-settings revision; no providers, providerOverrideFields, iCloud flags or keys.

Hotkeys: Carbon keyCode maps to W3C codes through an explicit tested table. Swift `modifierFlags` Command maps to the raw `win` value (frozen) and the Mac recorder labels from Mac Task 9 display it as Command; ctrl/alt/shift map to the existing raw values. An empty chord stays explicitly cleared.

Records merge: reuse Storage Task 1 `RecordsSnapshot::merged`. Destination tombstones and `clearedBefore` win over older Swift entries; UUIDs dedupe case-insensitively; `history_limit` comes from current settings (0 = no truncation). Limitation, documented in `docs/migration-from-swift.md`: Swift entries older than an already pruned 90-day tombstone cannot be distinguished and may reappear.

Journal: Storage Task 3 declares `JournalKind::{FolderSwitch, SwiftImport}` and the startup order (a pending folder switch completes first, then the import runs into the resulting active folder). This task fills `SwiftImport`: old and intended local settings bytes, destination file revisions and a completion marker in `LocalPaths`. The marker commits last; with it present a retry is a no-op and the UI shows "Imported on <date>" instead of offering import. A deliberate re-import needs a fresh preview. The relaunch-time import runs in `setup` before `ShellConfig` opens writable settings and before hotkeys, so it needs no `settings_write` sharing; it still takes the records directory leases.

**Commands:** Async `swift_import_preview(selection_id: Option<Uuid>) -> ImportPreview` (`None` = UserDefaults source), `swift_import_apply(preview_id: Uuid, fields: Vec<ImportField>) -> ImportView`, `swift_import_cancel() -> ImportView`; Mac-only UI, backend rejects on Windows. `ImportPreview { id, settings: SwiftSettingsPreview, history_count, usage_count, requires_relaunch: bool }` and `ImportView { pending: bool, completed: bool, completed_at: Option<i64>, unsupported_operations, message_key }` are redacted DTOs. Preview requires Swift quit and states that credentials and provider overrides must be re-entered. Swift-running is checked at preview, at scheduling and again at relaunch execution; apply also revalidates directory identity, file revisions and the local settings revision. `ImportField` is a whitelist enum, not arbitrary JSON. Importing from the current records directory needs explicit existing-file confirmation; an identity no-op does not duplicate records. `ImportSection` includes a hint to disable the old app's login item and hotkeys (keys with pl/uk/de in `i18n/macos.json`).

**Commits:**
1. `feat(core): map Swift settings and records for import` - `crates/lingopop-core/src/swift_import.rs`, `crates/lingopop-core/tests/swift_import.rs`, `crates/lingopop-core/tests/fixtures/swift-import/`.
2. `feat(platform): read Swift UserDefaults read-only` - `crates/lingopop-platform/src/{swift_source.rs,macos/swift_source.rs,fake/swift_source.rs}`.
3. `feat(macos): import Swift settings and records without modifying the source` - `app/src-tauri/src/import/`, journal extension in `app/src-tauri/src/storage/`, `app/src/lib/importIpc.ts`, `app/src/settings/{ImportSection.svelte,import.test.ts}`, `i18n/macos.json`.

- [ ] Write pure fixtures (synthetic, no real settings). Pin Option+T mapping; an unknown keyCode reports the operation while preserving the Rust binding; `{keyCode:0,modifierFlags:0}` clears deliberately; the legacy hotkey fills Translate only when no modern binding exists. The settings fixture carries a legacy `providers` entry with sentinel `SENTINEL-LEGACY-KEY` plus iCloud flags: the sentinel and flags must be absent from the preview, from `Debug` and from every written file. Invalid fields retain defaults/current values. Inspect the frozen Swift source rather than inferring the schema from Windows settings.

**Regression anchor — `swift_import_refuses_newer_settings_and_preserves_source`:** Fixture: a preview is taken against Rust settings at revision 1; the user then changes `target_language` (revision 2) before apply; the Swift UserDefaults fake holds a settings blob and history bytes.

```rust
assert_eq!(apply_result, Err(ImportError::Conflict));
assert_eq!(rust_settings_after, newer_rust_settings);
assert_eq!(swift_source_after, swift_source_before);
assert!(!completion_marker_exists);
```

**Regression anchor — `swift_import_does_not_resurrect_tombstoned_or_cleared_entries`:** Fixture: destination history holds a tombstone for UUID `AAAA…` (lowercase in the Swift bytes) and `clearedBefore = T`; Swift history has that UUID, one entry older than `T` and one newer entry `NEW`.

```rust
assert!(!history_after.contains_uuid("aaaa-..."));
assert!(!history_after.items().iter().any(|e| e.date < T));
assert_eq!(history_after.items().len(), 1);
assert_eq!(history_after.items()[0].id_upper(), "NEW");
```

**Regression anchor — `swift_import_recovers_after_each_write_step`:** Fixture: Storage Task 3 `FaultFs` armed with each `FaultStep` in turn (settings write, history write, usage write, marker write), then the relaunch recovery runs on the same directory.

```rust
for step in FaultStep::import_steps() {
    let dir = run_import_with_fault(step);
    recover_on_relaunch(&dir);
    assert_eq!(snapshot(&dir), complete_import_snapshot());
    assert_eq!(swift_source_after, swift_source_before);
}
assert!(completion_marker_exists);
```

**Regression anchor — `swift_launched_between_preview_and_relaunch_aborts`:** Fixture: fake `SwiftSource` reports not running at preview and at scheduling, then running when the relaunch import executes.

```rust
assert_eq!(relaunch_result, Err(ImportError::SwiftRunning));
assert!(pending_state_exists);
assert!(!completion_marker_exists);
assert_eq!(rust_settings_after, rust_settings_before);
```

**Regression anchor — `command_modifier_imports_as_win_and_displays_as_command`:** Fixture: Swift hotkey `{keyCode: 17, modifierFlags: Command|Option}` (Cmd+Option+T).

```rust
assert_eq!(imported.modifiers, vec![Modifier::Alt, Modifier::Win]);
assert_eq!(imported.key, "KeyT");
assert_eq!(mac_display_string(&imported), "⌥⌘T");
```

- [ ] Run `cargo test -p lingopop-core --test swift_import --locked`; capture RED, implement the pure whitelist mapper, then GREEN.
- [ ] Write shell transaction/frontend tests: Swift running prevents import and hotkey registration; a process-query error fails closed; source changes since preview abort; newer local settings are not overwritten; cancel creates no completion marker; retries deduplicate UUIDs and honor tombstones/clear; a completed import shows "Imported on <date>" and offers no second import. Assert source bytes/UserDefaults never change and no secrets or config files appear in selected RecordsPaths.
- [ ] Run `cargo test -p lingopop-app import:: --locked` and `npm test --prefix app -- src/settings/import.test.ts`; capture RED. Fill `JournalKind::SwiftImport` as specified above and execute pending imports at relaunch before writable stores/hotkeys; recovery validates hashes before rollback. The UI is honest about the relaunch. Then GREEN.
- [ ] Run the shared PR gate. Native UserDefaults/process tests use an injected throwaway domain/process, never real developer settings. Validate the actual frozen-app import only on explicit owner-run test copies.
- [ ] Review and commit locally per the Commits list.

### Task 19: Build both platform architectures and select compatible release assets

Independent of Task 18; may be executed before it.

**Files:** Modify `.github/workflows/release.yml`, `.github/workflows/ci.yml`, core `updates.rs`, shell `settings_cmds.rs`/About IPC, frontend `settings/AboutTab.svelte`, `CHANGELOG.md` (header no longer "for Windows"), Mac Tauri override `app/src-tauri/tauri.macos.conf.json` (must not contain `version`) and affected update/catalog tests; create `scripts/package-macos.sh`, `release-platforms.json`, `scripts/check-version.py`, `scripts/verify-release-assets.py`, `scripts/tests/{test_release_assets.py,test_check_version.py}`, `docs/RELEASING.md`.

**Interfaces:**
- `scripts/check-version.py [--tag vX.Y.Z]` asserts `Cargo.toml` `[workspace.package]` version, `app/src-tauri/tauri.conf.json`, `app/package.json` and `app/package-lock.json` are equal, that `tauri.macos.conf.json` has no `version`, and (with `--tag`) that the tag without `v` equals it. Used by every release leg and by CI.
- `package-macos.sh --version VERSION --target aarch64-apple-darwin|x86_64-apple-darwin --out DIRECTORY` runs `check-version.py`, includes the Mac config override, builds with the embedded frontend, stages `.dmg` and `.zip` containing LingoPop Preview.app, calls Mac Task 17's `check-macos-production.sh`, and checks bundle ID/minimum OS/architecture/signing status. Prefer native arm64 and native x86_64 runner legs; verify explicit current runner labels against primary GitHub runner-image documentation during implementation. Do not assume `macos-latest` is Intel or treat cross-compilation as native runtime coverage.
- `release-platforms.json { "macosPreview": bool }`: malformed or missing fails closed (no publish). Mac legs always build and verify; `macosPreview: false` only skips attaching Mac assets to the release.
- Prerequisites (gated, not optional): `actionlint` and `shellcheck` installed via Homebrew in this task. `swiftc` is required only by the mac-fixtures script.

**Asset contract:** Keep `LingoPop-<version>-windows-x64-portable.exe`; add `LingoPop-<version>-macos-arm64.dmg`, `.zip`, `LingoPop-<version>-macos-x64.dmg`, `.zip`. Each leg uploads an artifact named `LingoPop-<version>-<platform>` holding its assets plus `SHA256SUMS-<platform>.txt`. Job names stay `build-windows-portable` and `publish-prerelease` (job ids `build`/`publish` unchanged); add `build-macos-arm64` and `build-macos-x64`. `publish-prerelease` downloads all artifacts into separate directories, concatenates the per-platform files into the single `SHA256SUMS.txt` (lowercase SHA-256, two spaces, unique basename, LF) and verifies it before `gh release create`. Python `verify_assets(directory: Path, version: str, platforms: set[str]) -> list[str]` returns sorted fixed validation errors; zero errors means the exact selected asset set, valid sizes, no duplicate names, matching checksums, bundle metadata and inspected executable architecture. Native Mac legs mount/verify the dmg and inspect the installed app before upload; the publish verifier parses the zip Info.plist and Mach-O headers and hashes, without mounting a dmg on Linux. `scripts/verify-release-assets.py --docs` is added in Task 20.

**Signing:** default is ad-hoc `codesign -s - --options runtime --timestamp=none`. The Developer ID path uses Tauri's standard secrets `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` with `notarytool` and `stapler`; a completeness check fails the leg when some but not all are set (never a silent downgrade). Secrets stay in secret mechanisms, not config or logs. Native Mac legs set `com.apple.quarantine` on the built zip/dmg and verify `spctl --assess` output matches the expected ad-hoc or notarized outcome.

**Update contract:** Core `ReleasePlatform::{WindowsX64, MacosArm64, MacosX64}`, `ReleaseAsset { name: String, download_url: String }`, `ReleaseInfo { version: String, assets: Vec<ReleaseAsset> }`, `compatible_asset(release: &ReleaseInfo, target: ReleasePlatform) -> Option<&ReleaseAsset>`. `fetch_latest_release_with(transport, deadline) -> Option<ReleaseInfo>` queries `/repos/.../releases?per_page=10`, picks the newest by `is_newer`, includes prereleases only while `VERSION` itself is a prerelease (contains `-`); stable builds consider stable releases only. The existing `fetch_latest_version_with` wrapper, `RELEASES_PAGE_URL`, 3-hour interval and 10-second injected timeout stay. This also fixes today's Windows About for prerelease builds; note it in CHANGELOG. Require the exact generated filename, an HTTPS GitHub download URL on this repository's matching tag path and a supported architecture; never interpolate an arbitrary release URL into a shell command. About opens the compatible download/release page through fixed backend validation; it does not install or auto-replace an app.

**Commits:**
1. `chore(release): add shared version check` - `scripts/check-version.py`, `scripts/tests/test_check_version.py`, `.github/workflows/ci.yml`, `CHANGELOG.md`.
2. `feat(release): verify release assets` - `scripts/verify-release-assets.py`, `scripts/tests/test_release_assets.py`, `release-platforms.json`.
3. `feat(updates): select compatible release assets` - `crates/lingopop-core/src/updates.rs`, `app/src-tauri/src/{updates.rs,settings_cmds.rs}`, `app/src/settings/AboutTab.svelte`.
4. `feat(release): package macOS and Windows artifacts with compatible downloads` - `scripts/package-macos.sh`, `app/src-tauri/tauri.macos.conf.json`, `.github/workflows/release.yml`, `docs/RELEASING.md`.

- [ ] Install and gate tools: `brew install actionlint shellcheck`; record `actionlint --version` and `shellcheck --version`.
- [ ] Write Python tests for missing Mac slice/artifact, duplicate filename, uppercase/malformed/mismatched checksum, wrong version/bundle ID/deployment target, wrong executable arch, malformed `release-platforms.json`, and `check-version.py` mismatches (including a `version` key in `tauri.macos.conf.json`). Write core update tests for wrong OS/arch/name/tag URL, missing asset, hostile URL, deadline/network failures, existing stable comparison behavior, and the two channel tests below.

**Regression anchor — `missing_mac_asset_blocks_publication`:** Fixture: directory with the Windows exe and the macOS arm64 dmg/zip for `0.3.0-alpha.1` but no x64 dmg, plus a publish-script double counting `gh release create` calls.

```python
assert verify_assets(incomplete_dir, "0.3.0-alpha.1", {"windows", "macos"}) != []
assert publish_calls == 0
```

**Regression anchor — `wrong_arch_asset_blocks_publication`:** Fixture: a complete set where `macos-x64.zip` contains an arm64 Mach-O executable.

```python
errors = verify_assets(wrong_arch_dir, "0.3.0-alpha.1", {"windows", "macos"})
assert "macos-x64.zip: executable architecture is not x86_64" in errors
assert publish_calls == 0
```

**Regression anchor — `checksum_mismatch_blocks_publication`:** Fixture: a complete set whose concatenated `SHA256SUMS.txt` has one digest altered by one hex character.

```python
errors = verify_assets(tampered_dir, "0.3.0-alpha.1", {"windows", "macos"})
assert any("checksum mismatch" in e for e in errors)
assert publish_calls == 0
```

**Regression anchor — `malformed_release_platforms_fails_closed`:** Fixture: `release-platforms.json` contents `{"macosPreview": "yes"}`, then a missing file, then invalid JSON.

```python
for variant in (wrong_type, missing_file, invalid_json):
    assert read_release_platforms(variant) is None
    assert publish_calls == 0
```

**Regression anchor — `prerelease_build_sees_prerelease_assets`:** Fixture: `VERSION = "0.3.0-alpha.1"`; releases response contains only prerelease `v0.3.0-alpha.2` with the macOS arm64 zip.

```rust
let info = fetch_latest_release_with(&transport, deadline).await.unwrap();
assert_eq!(info.version, "0.3.0-alpha.2");
assert!(compatible_asset(&info, ReleasePlatform::MacosArm64).is_some());
```

**Regression anchor — `stable_build_ignores_prereleases`:** Fixture: `VERSION = "0.3.0"`; response contains prerelease `v0.4.0-alpha.1` and stable `v0.3.1`.

```rust
let info = fetch_latest_release_with(&transport, deadline).await.unwrap();
assert_eq!(info.version, "0.3.1");
```

- [ ] Run `python3 -m unittest discover -s scripts/tests`, `cargo test -p lingopop-core updates --locked` and relevant About tests; capture RED, implement validators/update selection, then GREEN.
- [ ] Add the version-bump step to `docs/RELEASING.md` (AGENTS.md Release steps 1–2) and run `check-version.py` in every release leg and CI.
- [ ] Extend Release workflow as specified: Windows build plus Mac arm64/x64 legs each test/build/package and upload `LingoPop-<version>-<platform>` artifacts; one publish job downloads all, builds and verifies `SHA256SUMS.txt`, then creates the pre-release. Retain tag-versus-version checks and `contents: write` solely in publish. `workflow_dispatch` builds artifacts only. A failed Mac leg prevents combined publication; no partial public release is called complete. Run `actionlint` and `shellcheck scripts/*.sh` clean.
- [ ] Document the ad-hoc permission re-grant behavior and exact install path in `docs/RELEASING.md`.
- [ ] Run the shared gate. Pre-merge validation: the owner pushes the branch and runs `workflow_dispatch` once (the index's "do not push" is lifted for this single step by the owner); inspect/download every artifact, executable architecture, minimum OS and checksum manifest, and record the run URL in `task-19.md` before the PR is marked ready. This validates packaging, not interactive runtime. Do not push a release tag as a test.
- [ ] Review and commit locally per the Commits list.

### Task 20: Real-platform acceptance, documentation and preview cutover

**Files:** Update `AGENTS.md`, `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `PRIVACY.md`, `CHANGELOG.md`, `docs/RELEASING.md`, `docs/testing/manual-checklist.md`, `scripts/verify-release-assets.py` (`--docs`), `scripts/tests/test_release_assets.py`, `release-platforms.json`; add `docs/testing/cross-platform-acceptance.md`, `docs/migration-from-swift.md`. Only update About/topics on GitHub when that external metadata is separately requested/authorized.

**Interfaces:** `scripts/verify-release-assets.py --docs` asserts the asset names in `docs/RELEASING.md` equal the names generated by the workflow and `release-platforms.json`. The acceptance report records exact commit/artifact hash, platform/OS/architecture, signing mode, installation path, permissions state, test case, observed result and outstanding blockers. It is evidence, not a program API. Real acceptance cannot be replaced by unit fakes or CI pass. The general-availability decision stays separate from authorizing a preview release.

**Owner hardware matrix:** Apple Silicon Mac (macOS 13 or later), Intel Mac (or an explicit Rosetta-only note that is not an Intel pass), Windows 11 machine. Absent hardware is recorded as a gate gap in `cross-platform-acceptance.md` before PR G is marked ready; it never becomes an inferred pass.

**Commits:**
1. `test(release): check RELEASING.md asset names` - `scripts/verify-release-assets.py`, `scripts/tests/test_release_assets.py`.
2. `docs: document cross-platform preview and Swift transition` - `AGENTS.md`, `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `PRIVACY.md`, `CHANGELOG.md`, `docs/RELEASING.md`, `docs/testing/`, `docs/migration-from-swift.md`.
3. `chore(release): enable macOS preview assets` - `release-platforms.json` only (`macosPreview: true`), made only after the readiness matrix is satisfied or its gaps are recorded and the owner accepts them.

- [ ] Prepare the manual matrix before testing: fresh install and upgrade; Swift coexistence/quit/import; missing/denied/revoked permissions; clipboard text/RTF/image/files/multi-item/unreadable type; selection and Replace across source windows; tray actions/hotkeys; popup Copy/Retry/Escape/outside-click/reopen; Screen Translate/OCR/vision retry; multiple display scales/negative origins/Spaces/fullscreen/display removal; login/second launch; custom-folder copy/use/default, crash recovery, unavailable folder, external replacement and delete/clear/reset. Cover Windows regression cases as required release gates.
- [ ] Record the expected outcome that an ad-hoc preview upgrade can require re-granting TCC permissions (Accessibility, Screen Recording); verify it on the upgrade case and record the observation rather than treating it as a failure.
- [ ] Run the shared automated gate and required desktop CI, then installed-app acceptance on the hardware above with consent for clipboard/input/screen/login side effects. Record Intel and Apple Silicon runtime separately; a cross-built or inspected executable does not mean it was run.
- [ ] Make docs accurately state Swift freeze, no iCloud service, history/usage-only folder, source-preserving import, credential/provider re-entry, sequential cloud-folder support and limits, unsigned/ad-hoc preview instructions, installation identity and supported OS/architectures. Add the spec's limitation statement where the folder is selected (the Storage settings section text, with pl/uk/de keys, and `docs/migration-from-swift.md`). Perform the README/AGENTS/design scope update: replace the old Windows-only product scope in `AGENTS.md`, `README.md` and the design-spec scope while preserving Windows-specific instructions, API-guide anchors and localization/fixture provenance.
- [ ] Inspect exact docs surrounding every claimed correction; run `python3 scripts/verify-release-assets.py --docs` (RED first with a wrong name in RELEASING.md, then GREEN), verify release instructions against the actual scripts/workflow and checksum format, and run `git diff --check`. No product behavior tests are needed for prose-only edits.
- [ ] Review and commit locally per the Commits list; the `macosPreview: true` flip is its own commit, never part of the docs commit.
- [ ] Hand off exact branch diff, gate results and acceptance gaps. Only after release authorization, follow the repository release-via-PR/tag process, verify the resulting downloaded artifacts and checksums, and publish Mac as Preview. Do not archive Swift or claim general availability until the signing exception or Developer ID and runtime acceptance gates are satisfied.
