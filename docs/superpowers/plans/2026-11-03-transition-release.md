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

**Files:** Create core `swift_import.rs`/`tests/swift_import.rs`; platform `swift_source.rs`, `macos/swift_source.rs`, `fake/swift_source.rs`; shell `import/{mod.rs,commands.rs,transaction.rs,tests.rs}`; frontend `lib/importIpc.ts`, `settings/ImportSection.svelte`, `settings/import.test.ts`; wire existing platform exports, shell setup/settings tabs, storage journal extension and Mac catalogs.

**Interfaces:** Core `ImportError::{Unavailable, InvalidData, UnsupportedVersion, SwiftRunning, Conflict, RecoveryRequired}` has fixed text/redacted Debug; no raw bytes/fields. Portable `SwiftSource: Send + Sync` exposes `is_running() -> Result<bool, ImportError>` and `read_local_settings() -> Result<Option<Vec<u8>>, ImportError>`. Native reader accesses only UserDefaults domain `com.lingopop.app`, key `LingoPop.settings.v1`, without full-domain export or writes. Raw bytes are short-lived, never serialized to UI or logs. Core `plan_swift_settings(bytes: &[u8], current: &Settings) -> Result<SwiftSettingsPreview, ImportError>` parses the actual Swift schema and maps only activeProvider, fallbacks, targetLanguage, supported hotkeys/legacy hotkey, limits, useAutoDetect, languagePairs, autoCopyOCR and sanitized magicPrompt. Import does not turn on launch-at-login implicitly. `SwiftSettingsPreview` includes a whitelisted candidate, field differences, unsupported-hotkey operation IDs and a current-local-settings revision; no providers/providerOverrideFields/iCloud flags/keys. Carbon keyCode maps to supported W3C codes through an explicit tested table; modifiers map to existing ctrl/alt/shift/win raw values; empty chord stays explicitly cleared.

**Commands:** Async `swift_import_preview(selection_id: Uuid) -> ImportPreview`, `swift_import_apply(preview_id: Uuid, fields: Vec<ImportField>) -> ImportView`, `swift_import_cancel() -> ImportView`; Mac-only UI, backend rejects on Windows. `ImportPreview { id, settings: SwiftSettingsPreview, history_count, usage_count, requires_relaunch: bool }` and `ImportView { pending: bool, completed: bool, unsupported_operations, message_key }` are redacted DTOs. History/usage folder is chosen through the native opaque selection service from Storage Task 5. Preview requires Swift quit and states credentials/provider overrides must be re-entered. Apply revalidates process state, directory identity, file revisions and local settings revision. `ImportField` is a whitelist enum, not arbitrary JSON. An explicit existing-file confirmation is required when importing from the current records directory; identity no-op does not duplicate records.

- [ ] Write pure fixtures locally in `crates/lingopop-core/tests/fixtures/swift-import/` (synthetic, no real settings). Pin Option+T mapping; unknown keyCode reports the operation while preserving the Rust binding; `{keyCode:0,modifierFlags:0}` clears deliberately; legacy hotkey fills Translate only when no modern binding exists; every iCloud/provider/key sentinel is absent from preview/Debug/output; invalid fields retain defaults/current values. Inspect the frozen Swift source rather than inferring schema from Windows settings.

**Regression anchor — `swift_import_refuses_newer_settings_and_preserves_source`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(apply_result, Err(ImportError::Conflict));
assert_eq!(rust_settings_after, newer_rust_settings);
assert_eq!(swift_source_after, swift_source_before);
assert!(!completion_marker_exists);
```

- [ ] Run `cargo test -p lingopop-core --test swift_import --locked`; capture RED, implement the pure whitelist mapper, then GREEN.
- [ ] Write shell transaction/frontend tests: Swift running prevents import and hotkey registration; process-query error fails closed; source changes since preview abort; newer local settings are not overwritten; cancel does not create a completion marker; retries deduplicate UUIDs and honor tombstones/clear; failure after each record/settings/marker write recovers to one complete import. Assert source bytes/UserDefaults never change and no secrets/config files appear in selected RecordsPaths.
- [ ] Run `cargo test -p lingopop-app import:: --locked` and `npm test --prefix app -- src/settings/import.test.ts`; capture RED. Extend Task 3's local journal to include old/intended local settings bytes and a completion marker. Prepare imports as pending and execute at relaunch before writable stores/hotkeys, using the same directory leases and strict decoders; UI is honest about relaunch. Marker commits last, and recovery validates hashes before rollback. Once committed, retry is a no-op; another deliberate import requires a fresh preview against current state. The importer never opens secrets.dat or shell-exports UserDefaults.
- [ ] Run GREEN plus shared PR gate. Native UserDefaults/process tests use an injected throwaway domain/process, never real developer settings. Validate actual frozen-app import only on explicit owner-run test copies.
- [ ] Review and locally commit: `feat(macos): import Swift settings and records without modifying the source`.

### Task 19: Build both platform architectures and select compatible release assets

**Files:** Modify `.github/workflows/release.yml`, shell/core `updates.rs`, shell `settings_cmds.rs`/About IPC and frontend `settings/AboutTab.svelte`; create `scripts/package-macos.sh`, `release-platforms.json`, `scripts/verify-release-assets.py`, `scripts/tests/test_release_assets.py`, `docs/RELEASING.md`; modify Mac Tauri override, workflow permissions/checksum staging, `.github/workflows/ci.yml` and affected update/catalog tests.

**Interfaces:** `package-macos.sh --version VERSION --target aarch64-apple-darwin|x86_64-apple-darwin --out DIRECTORY` verifies workspace/npm/Tauri/lock versions, includes the Mac config override, builds with embedded frontend, stages `.dmg` and `.zip` containing LingoPop Preview.app and checks bundle ID/minimum OS/architecture/signing status. Prefer native arm64 and native x86_64 runner legs; verify availability of explicit current runner labels against primary GitHub runner-image documentation during implementation. Do not assume `macos-latest` is Intel or silently treat cross-compilation as native runtime coverage.

**Asset contract:** Keep `LingoPop-<version>-windows-x64-portable.exe`; add `LingoPop-<version>-macos-arm64.dmg`, `.zip`, `LingoPop-<version>-macos-x64.dmg`, `.zip`; one `SHA256SUMS.txt` with lowercase SHA-256, two spaces, unique basename and LF. Verification API in Python: `verify_assets(directory: Path, version: str, platforms: set[str]) -> list[str]` returns sorted fixed validation errors; zero errors means exact selected platform asset set, valid file sizes, no duplicate names, matching checksums, bundle metadata and inspected executable architecture. Native Mac legs mount/verify the dmg and inspect its installed app before upload; the portable publication verifier parses zip Info.plist and Mach-O headers and validates hashes, without trying to mount a dmg on Linux. No host-specific shell assumptions in checksum validation.

**Update contract:** Core `ReleasePlatform::{WindowsX64, MacosArm64, MacosX64}`, `ReleaseAsset { name: String, download_url: String }`, `ReleaseInfo { version: String, assets: Vec<ReleaseAsset> }`, `compatible_asset(release: &ReleaseInfo, target: ReleasePlatform) -> Option<&ReleaseAsset>`. Release lookup gains `fetch_latest_release_with(transport, deadline) -> Option<ReleaseInfo>` while preserving existing `fetch_latest_version_with` wrapper, 3-hour interval and 10-second injected timeout. Require exact generated filename, HTTPS GitHub download URL on this repository's matching tag path and supported architecture; never interpolate an arbitrary release URL into a shell command. About opens the compatible download/release page through fixed backend validation; it does not install or auto-replace an app. Keep current stable-latest channel semantics; same-base alpha-to-alpha update delivery is not added implicitly. Previews are explicitly downloaded from Releases until an approved channel policy exists.

- [ ] Write Python tests for missing Mac slice/artifact, duplicate filename, uppercase/malformed/mismatched checksum, wrong version/bundle ID/deployment target and wrong executable arch. Assert incomplete selected platform output cannot publish. Write core update tests for wrong OS/arch/name/tag URL, missing asset, hostile URL, deadline/network failures and existing stable comparison behavior.

**Regression anchor — `missing_mac_asset_blocks_publication`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```python
assert verify_assets(incomplete_dir, "0.3.0-alpha.1", {"windows", "macos"}) != []
assert publish_calls == 0
```

- [ ] Run `python3 -m unittest discover -s scripts/tests -p 'test_release_assets.py'`, `cargo test -p lingopop-core updates --locked` and relevant About tests; capture RED, implement validators/update selection, then GREEN.
- [ ] Extend Release workflow: Windows build plus Mac arm64/x64 legs each test/build/package and upload uniquely named workflow artifacts; one publish job downloads all, verifies combined checksums/metadata and only then creates the pre-release. Retain tag-versus-version checks and contents:write solely in publish. workflow_dispatch builds artifacts only; platform selection is explicit (`windows`, `macos`, `both`), with tag publication controlled by committed `release-platforms.json` (`macosPreview: false` initially). Task 20 sets macosPreview true only with recorded preview-readiness evidence, then tag publication selects both. workflow_dispatch may build both earlier without public publication. Failed Mac leg prevents combined publication; no partial public release is called complete.
- [ ] Support optional secret-backed Developer ID signing/notarization without requiring secrets for ad-hoc preview. Identity/password handling stays in secret mechanisms, not config/logs; missing requested signing inputs fails clearly rather than silently downgrading. Document ad-hoc permission re-grant behavior and exact install path. Verify scripts with shellcheck/actionlint when available and record unavailable tools.
- [ ] Run the shared gate; run workflow_dispatch and inspect/download every selected artifact, executable architecture, minimum OS and checksum manifest. This validates packaging, not interactive runtime. Do not push a release tag as a test or publish without the owner's release authorization.
- [ ] Review and locally commit: `feat(release): package macOS and Windows artifacts with compatible downloads`.

### Task 20: Real-platform acceptance, documentation and preview cutover

**Files:** Update `AGENTS.md`, `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `PRIVACY.md`, `CHANGELOG.md`, `docs/RELEASING.md`, `docs/testing/manual-checklist.md`; add `docs/testing/cross-platform-acceptance.md`, `docs/migration-from-swift.md`. Task 20 changes `release-platforms.json` to macosPreview true only after the preview readiness matrix is satisfied. Only update About/topics on GitHub when publishing that external metadata is separately requested/authorized.

**Interfaces:** Acceptance report records exact commit/artifact hash, platform/OS/architecture, signing mode, installation path, permissions state, test case, observed result and outstanding blockers. It is evidence, not a new program API. Real acceptance cannot be replaced by unit fakes/CI pass. General-availability decision remains separate from authorizing a preview release.

- [ ] Prepare the manual matrix before testing: fresh install and upgrade; Swift coexistence/quit/import; missing/denied/revoked permissions; clipboard text/RTF/image/files/multi-item/unreadable type; selection and Replace across source windows; tray actions/hotkeys; popup Copy/Retry/Escape/outside-click/reopen; Screen Translate/OCR/vision retry; multiple display scales/negative origins/Spaces/fullscreen/display removal; login/second launch; custom-folder copy/use/default, crash recovery, unavailable folder, external replacement and delete/clear/reset. Cover Windows regression cases as required release gates.
- [ ] Run the shared automated gate and required desktop CI, then installed-app acceptance on real Mac and Windows with consent for clipboard/input/screen/login side effects. Record Intel and Apple Silicon runtime separately; a cross-built or inspected executable does not mean it was run. Unavailable hardware remains an explicit gate gap and does not become an inferred pass.
- [ ] Make docs accurately state Swift freeze, no iCloud service, history/usage-only folder, source-preserving import, credential/provider re-entry, sequential cloud-folder support and limits, unsigned/ad-hoc preview instructions, installation identity and supported OS/architectures. Replace the old Windows-only product scope in AGENTS/README while preserving Windows-specific instructions. Preserve API-guide anchors and existing localization/fixture provenance.
- [ ] Inspect exact docs surrounding every claimed correction; verify release instructions against actual scripts/workflow, all platform asset names and checksum format. No product behavior tests are needed for prose-only edits; use focused link/command consistency and `git diff --check`.
- [ ] Review and locally commit: `docs: document cross-platform preview and Swift transition`.
- [ ] Hand off exact branch diff, gate results and acceptance gaps. Only after release authorization, follow repository release-via-PR/tag process, verify the resulting downloaded artifacts and checksums, and publish Mac as Preview. Do not archive Swift or claim general availability until signing exception/Developer ID and runtime acceptance gates are satisfied.
