# Production macOS Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace every Mac development shortcut with production platform behavior while retaining shared logic and Windows correctness.

**Architecture:** Add focused native services behind the existing platform seams; extend portable types only when necessary. Introduce a queued, generation-fenced presentation seam before AppKit popup integration. Native object ownership stays on the required thread; application callbacks only enqueue work.

**Tech Stack:** Existing Rust/objc2/Core Graphics dependencies, verified native macOS frameworks, Tauri v2, Svelte and repository-owned catalogs.

**Spec:** `docs/superpowers/specs/2026-11-03-cross-platform-design.md`; all Global Constraints/shared gate in `2026-11-03-cross-platform.md` apply verbatim. Depends on Storage Tasks 1–5 before integration/PR publication.

## Global Constraints

- Target macOS 13.0, arm64 and x86_64. Verify every added binding/dependency against official primary sources and installed crate sources; do not invent SDK symbols or silently increase minimum OS.
- Mac identifier `app.lingopop.tauri.macos`; local directory `~/Library/Application Support/LingoPop-Tauri`; preview name LingoPop Preview. Windows identity/path remain unchanged.
- No plaintext fallback, automatic dev-secret import, user-text logging or shared credentials.
- Do not block UI callbacks, hold standard mutexes across awaits, or hold a commit lock while dispatching to the main thread.
- Preserve generation fencing, source targeting, clipboard protection and hide-before-Replace. Keep core OS-independent and orchestrator free of OS/Tauri dependencies.
- Keep and freeze Swift. Read its sources as reference; do not modify, launch, migrate or uninstall it during an implementation test without the relevant authorized test scope.

## Review Focus

- A locked Keychain, damaged ciphertext or concurrent first launch never creates plaintext or replaces an existing key (Task 7).
- Denied/revoked permissions and disabled hotkeys produce actionable states without unrelated prompts or registration churn (Tasks 8–9).
- Non-text/multi-item/promised pasteboard content is never silently destroyed, and a foreign write observed before a destructive write aborts the operation; a write landing between the final `changeCount` check and `clearContents` cannot be excluded because NSPasteboard has no cross-process compare-and-swap (Task 10: `snapshot_aborts_on_concurrent_foreign_write`, `promised_type_aborts_without_read`, `oversized_type_aborts_before_copy`).
- Per-monitor scaling, target loss and changed display topology cannot redirect capture/Replace (Tasks 11–12, 15–16: `display_removal_invalidates_capture_plan`, `revoked_accessibility_never_pastes`, `target_lost_shows_manual_paste`).
- Reentrant main-thread callbacks and out-of-order show/hide acknowledgments cannot deadlock or resurrect a stale popup (Tasks 13–14: `old_show_queued_during_new_capture_has_zero_effects`, `stale_show_after_dismiss_has_no_effect`, `hide_ack_precedes_paste`).

---

### Task 6: Own shared and platform localization in this repository

**Files:** Create authoritative `i18n/catalogs/{pl,uk,de}.json` from committed generated values and `i18n/macos.json`; modify `i18n/SOURCE.txt`, `scripts/gen-localizations.py`, `crates/lingopop-core/src/i18n.rs`, shell `settings_cmds.rs`/message callsites, `AGENTS.md`; add `scripts/tests/test_localizations.py`. Remove generated/exclusion paths only after every include/check/README reference is migrated in the same commit.

**Interfaces:** Core `CatalogPlatform::{Windows, Macos}`; `lookup_for<'a>(platform: CatalogPlatform, language: Language, key: &'a str) -> Cow<'a, str>`, `catalog_for(platform: CatalogPlatform, language: Language) -> &'static HashMap<String, String>`, `t_for(platform: CatalogPlatform, language: Language, key: &str, args: &[&str]) -> String`. Preserve existing `lookup`/`catalog`/`t` as Windows wrappers until all shell callsites inject platform. Python `check_catalogs(root: Path) -> list[str]` returns deterministic validation issues and the CLI prints them with its existing exit-code contract. `gen-localizations.py check` validates shared + both platform catalogs without Swift checkout; optional `generate` now operates on repository catalogs, never overwrites authoritative values from Swift. Each platform key is distinct from shared keys; promoted shared values retain exact placeholders/newlines. Platform-specific equivalent phrases use separate keys when required. This is the FIRST commit of PR B and it changes Windows-visible catalog loading: it runs the full Windows CI leg (`build-and-test`) before any later commit builds on it.

**Commits:** one commit, `refactor(i18n): own shared and platform catalogs in the app repository` (owns `i18n/`, `scripts/gen-localizations.py`, `scripts/tests/`, `crates/lingopop-core/src/i18n.rs`, `app/src-tauri/src/settings_cmds.rs`, `AGENTS.md`).

- [ ] Write core/Python tests: Windows resolves shared+windows but not Mac-only permission/Command keys; Mac resolves shared+macos; English returns the key; a Windows value containing `⌘` fails; a correctly signed Mac value containing `⌘` passes; bad `%1$@`, newline and `{{targetLanguage}}` signatures fail for either platform. Test with nonexistent `LINGOPOP_MAC_REPO` and network disabled. Assert existing Windows merged catalogs remain semantically identical. Promote Task 5's common storage strings into shared catalogs.

**Regression anchor — `windows_rejects_command_symbols_but_macos_accepts_them`:** fixture is a temp repo root with one key `Press ⌘V to paste.` carrying pl/uk/de values that all keep `⌘`, listed once in `windows.json` and once in `macos.json` copies; `merged_windows_before` is the merged Windows catalog dict captured before the migration.

```python
assert windows_issues != []
assert macos_issues == []
assert merged_windows_before == merged_windows_after
```

- [ ] Run `python3 -m unittest discover -s scripts/tests -p 'test_localizations.py'` and `cargo test -p lingopop-core i18n --locked`; capture RED, implement catalog ownership/injected composition, then rerun plus `python3 scripts/gen-localizations.py check`.
- [ ] Review and locally commit: `refactor(i18n): own shared and platform catalogs in the app repository`.

### Task 7: Stable Mac identity and Keychain-backed secret protection

**Files:** Create `app/src-tauri/tauri.macos.conf.json`, `crates/lingopop-platform/src/secrets_aead.rs`, `crates/lingopop-platform/src/macos/{secrets.rs,keychain.rs}`, `crates/lingopop-platform/tests/macos_secrets.rs`; modify core `secrets.rs` and `fsio.rs` (`LoadOutcome::Locked`), platform `windows/dpapi.rs`, `native.rs`, `lib.rs`, `dev_protector.rs`, Mac `mod.rs`, platform `Cargo.toml`/lockfile and affected native-bundle tests, shell `paths.rs`, `provider_cmds.rs`, frontend `settings/ProvidersTab.svelte`/`ProviderForm.svelte`, `i18n/macos.json` (reset dialog and error keys, pl/uk/de).

**Interfaces:**
- Core `secrets.rs`: `ProtectError::{Unavailable, Invalid, KeyMissing}` (fixed text, no data). `SecretStore::open`: `Invalid` quarantines the file (today's behaviour); `Unavailable` / `KeyMissing` return `LoadOutcome::Locked`: the store is read-only, the UI takes the existing `secretsUnavailable` path, and the file is never quarantined and never written. `WindowsProtector` maps every DPAPI failure to `Invalid`, so Windows behaviour is unchanged (this commit touches Windows files and runs the full Windows CI leg).
- Portable `crates/lingopop-platform/src/secrets_aead.rs`: pure Rust `aes-gcm` + `zeroize` + `getrandom` (un-scoped dependencies are allowed because they are not OS-bound; only `security-framework` is `cfg(target_os = "macos")`). Format: magic `LPS1`, 12-byte random nonce, AES-256-GCM, magic as associated data, one key, no rotation. Verify the exact crate versions in the registry before pinning.
- `MacProtector::new() -> Self: SecretProtector`; injectable `KeychainKeyStore` with `load() -> Result<Option<SecretKey>, ProtectError>` and atomic `insert_if_absent(SecretKey) -> Result<SecretKey, ProtectError>`; `SecretKey` Debug is redacted and zeroized on drop. Keychain item: legacy file keychain through `security-framework` (not in `Cargo.lock` today: add `security-framework` 3.7.0, or the newest version verified in the registry, to `crates/lingopop-platform/Cargo.toml` under `cfg(target_os = "macos")` and pin it in `Cargo.lock` first; then verify it exposes `SecItemAdd` / `SecItemCopyMatching` with these attributes, else raw FFI in `src/macos` with `// SAFETY:`), class generic password, service `app.lingopop.tauri.macos`, account `secrets-key-v1`, `kSecAttrSynchronizable = false` (no iCloud Keychain), `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`. A missing key with an existing non-empty `secrets.dat` is `KeyMissing`: never generate a replacement silently. Locked or denied Keychain is `Unavailable`; bad header or tag is `Invalid`.
- Ad-hoc signing changes the cdhash on every build, so the Keychain ACL may prompt or deny after each preview upgrade. This is expected and documented for previews; the pass criterion is "prompt or re-grant", not "silent".
- User-initiated reset: async IPC `provider_reset_secrets` quarantines `secrets.dat`, deletes and regenerates the Keychain key and leaves an empty store; the Providers tab offers it behind a confirm dialog only while `secretsUnavailable`. English keys with pl/uk/de in `i18n/macos.json`.
- `DevPlaintextProtector` becomes `#[cfg(any(test, feature = "dev-plaintext"))]`; the feature is off by default and the Task 17 packaging guard asserts it is off. `native()` on macOS never selects it. `tauri.macos.conf.json` (auto-merged by Tauri v2) sets `identifier` `app.lingopop.tauri.macos` and `productName` `LingoPop Preview`; Windows `tauri.conf.json` is untouched. Mac data dir is `~/Library/Application Support/LingoPop-Tauri`; `LingoPop-dev` is not an import source.

**Commits:**
1. `refactor(core): classify protector failures and lock the secret store` (owns core `secrets.rs`, `fsio.rs`, `windows/dpapi.rs`, `dev_protector.rs`, shell `state.rs` callsites).
2. `feat(platform): add portable AES-256-GCM secret container` (owns `secrets_aead.rs`, platform `Cargo.toml`, `Cargo.lock`).
3. `feat(macos): hold the secrets key in the Keychain` (owns `macos/{secrets,keychain}.rs`, `native.rs`, `lib.rs`, Mac `mod.rs`, `tauri.macos.conf.json`, shell `paths.rs`, `tests/macos_secrets.rs`).
4. `feat(settings): reset unreadable secrets on user request` (owns `provider_cmds.rs`, `ProvidersTab.svelte`, `ProviderForm.svelte`, `i18n/macos.json`).

- [ ] Write tests with an injected key store: portable AEAD `aead_roundtrip_uses_fresh_nonce` (same plaintext twice gives different bytes, neither contains the plaintext), `aead_tamper_wrong_key_or_unknown_magic_is_invalid`; `concurrent_first_launch_returns_one_durable_key`; `failed_key_save_writes_no_secrets_file`; core `dpapi_failure_still_quarantines` (fake protector returning `Invalid` renames the file to `.bak-*`) and `key_missing_never_quarantines` (fake returning `KeyMissing` / `Unavailable` leaves the file byte-identical and yields `Locked`); `reset_quarantines_file_and_regenerates_key`; `native_mac_protector_is_not_identity`; sentinel Debug/Display checks. Pin Windows identity/directory/config unchanged.

**Regression anchor — `unavailable_keychain_never_replaces_ciphertext`:** fixture is a tempdir holding a valid `secrets.dat` (known bytes) and a fake `KeychainKeyStore` whose `load` returns `Err(Unavailable)` and counts `insert_if_absent` calls.

```rust
assert_eq!(open_outcome, LoadOutcome::Locked);
assert_eq!(ciphertext_after, ciphertext_before);
assert_eq!(generated_key_count, 0);
assert!(quarantined_files.is_empty());
```

- [ ] Run `cargo test -p lingopop-platform --test macos_secrets --locked` on Mac (AEAD/format tests portable on every OS), `cargo test -p lingopop-core secrets --locked` and `cargo test -p lingopop-app task7 --locked`; capture RED, implement per commit, then GREEN. Keychain native tests use a throwaway service name and an explicit owner-run marker; never touch the real key.
- [ ] Audit the native bundle and the core-dependency guard (`bash scripts/check-core-deps.sh`); confirm `tauri.macos.conf.json` is picked up by `tauri build`/`tauri dev` on macOS without a `--config` flag.
- [ ] Review and locally commit each commit above in order.

### Task 8: Explicit permission state and recovery actions

**Files:** Create platform `permissions.rs`, `fake/permissions.rs`, `macos/permissions.rs`; modify `native.rs`, platform/fake module exports, `error.rs`, existing fake bundle constructors, orchestrator `messages.rs`; create shell `permissions_cmds.rs`, frontend `lib/permissionsIpc.ts`, `settings/PermissionsSection.svelte`, permission tests and `i18n/macos.json` (status, request, open-settings and relaunch-hint keys with pl/uk/de); wire through shell `lib.rs`/MainTab. Windows-touching: `error.rs`, `native.rs`, fake bundles, `messages.rs` (full Windows CI leg).

**Interfaces:** `PermissionKind::{Accessibility, ScreenRecording}`, `PermissionState::{NotDetermined, Denied, Granted, Restricted}`, trait `Permissions: Send + Sync` with non-prompting `state(kind) -> PermissionState`, user-action `request(kind) -> Result<PermissionState, PlatformError>`, and `open_settings(kind) -> Result<(), PlatformError>`. Add `Platform.permissions: Arc<dyn Permissions>` with no-prompt Granted Windows/fake defaults. Add `PlatformError::PermissionDenied(PermissionKind)`; update the error-variant enumeration test and `platform_message` (new variant maps to per-kind Mac keys, Windows `InputBlocked` text unchanged) so Mac never shows Windows UIPI instructions. Async shell IPC exposes statuses/request/open/refresh with fixed typed keys; no arbitrary settings URLs from the webview. Screen Recording on macOS 13 may need a relaunch after the grant: the retry-after-grant step is "retry; if `CGPreflightScreenCaptureAccess` still reports not granted in this process, or capture still returns the wallpaper-only image, show the relaunch hint" (new key `Screen Recording was granted. Relaunch LingoPop to capture the screen.`). TCC grants are bound to the code signature: ad-hoc previews need a re-grant after each upgrade; this is expected and recorded in the task record and the PR.

**Commits:**
1. `feat(platform): add the permission seam and PermissionDenied error` (owns `permissions.rs`, `fake/permissions.rs`, `error.rs`, `native.rs`, exports, fake bundles, orchestrator `messages.rs`).
2. `feat(macos): query and request TCC permissions` (owns `macos/permissions.rs`, Mac `mod.rs`).
3. `feat(settings): show permission status and recovery actions` (owns `permissions_cmds.rs`, `lib.rs`, `permissionsIpc.ts`, `PermissionsSection.svelte`, MainTab, tests, `i18n/macos.json`).

- [ ] Write fake/shell/frontend tests: bootstrap checks never prompt; selecting Screen Translate requests only ScreenRecording when needed; selection input requests only Accessibility; denied/revoked access keeps controls actionable; returning from System Settings refreshes state; retry after grant works without a permanent cached denial; `post_grant_preflight_false_shows_relaunch_hint`. Assert Windows InputBlocked text remains unchanged and the enumeration test covers every `PlatformError` variant. Verify permission actions never log screenshot/text data.

**Regression anchor — `bootstrap_does_not_prompt_and_revocation_is_recoverable`:** fixture is `FakePermissions` with ScreenRecording `Denied` and a counting `request`, a `FakeCapture` that returns `PermissionDenied` while the fake is Denied, then the test flips the fake to `Granted`.

```rust
assert_eq!(request_calls_after_bootstrap, 0);
assert_eq!(denied_capture, Err(PlatformError::PermissionDenied(PermissionKind::ScreenRecording)));
assert!(capture_after_grant.is_ok());
```

- [ ] Run `cargo test -p lingopop-app permission --locked`, `cargo test -p lingopop-orchestrator messages --locked`, and `npm test --prefix app -- src/settings/permissions.test.ts`; capture RED, implement native queries/actions after SDK verification, then GREEN. Record real installed-app denied/granted/revoked coverage separately.
- [ ] Review and locally commit each commit above in order.

### Task 9: Platform shortcut defaults, labels and menu-bar app behavior

**Files:** Modify core `hotkey.rs` and `settings.rs`, platform `hotkey.rs`/Mac `keys.rs`/`system.rs`/`fake/system.rs`/`windows/system.rs`, shell `hotkeys.rs`, `state.rs`, `settings_cmds.rs`, `lib.rs`, `tray.rs`, `windows.rs`, frontend `settings/HotkeyRecorder.svelte`, `lib/settingsTypes.ts`; create `app/src-tauri/Info.plist` (`LSUIElement` = true), `app/src-tauri/icons/tray/mac-template.png`, shell `swift_source.rs`, portable shortcut tests and frontend shortcut cases, `.superpowers/cross-platform/task-9.md` (hotkey probe record); update `i18n/macos.json` (Mac shortcut and Swift-running notice keys, pl/uk/de). No hand-rolled `app_lifecycle.rs`. Windows-touching: core `settings.rs` load signature, `state.rs`, `system.rs` impls (full Windows CI leg).

**Interfaces:**
- Core `Settings::default_for(host: HostKind) -> Settings`; `HostKind::{Unix, Windows}` is defined by Storage Task 1. `Settings::load` takes the host; a missing `hotkeys` field decodes to the host default (like Windows, only Translate has a default: Option+T on macOS, Ctrl+Alt+T on Windows; the other five operations stay unbound on both hosts). An explicitly cleared binding stays cleared. `Hotkey::display_for(HostKind) -> String` and `default_translate_for(HostKind) -> Hotkey`; existing `display_string()`/`default_translate()` keep Windows output. Persisted modifier raw values stay `ctrl/alt/shift/win`; Mac shows `win` as Command (`⌘`) and `alt` as Option. `UiBootstrap` adds the host for recorder/labels.
- Default chord Option+T is conditional: first verify Carbon `RegisterEventHotKey` through `global-hotkey` 0.9 registers Option+T on macOS 13, 14 and 15 (Apple changed Option-only chords in 15). If any version fails, the Mac Translate default is Ctrl+Option+T and the decision plus evidence is recorded in `.superpowers/cross-platform/task-9.md`.
- Lifecycle: Accessory policy through Tauri `set_activation_policy(ActivationPolicy::Accessory)` plus `LSUIElement`; explicit `NSApp.activate(ignoringOtherApps: true)` (objc2, main thread) when Settings/History opens; tray uses a template icon (`icon_as_template(true)`).
- Swift-running guard (moved here from Task 17): `SwiftSource::is_running() -> Result<bool, PlatformError>` (bundle id `com.lingopop.app`, read-only, via new `SystemInfo::is_bundle_running(&self, bundle_id: &str) -> Result<bool, PlatformError>`; Windows returns `Ok(false)`, fake is scripted). Error mapping: hotkey blocking treats `Err` as running (fail closed); Transition Task 18 maps `Err` to `ImportError::Unavailable` and rejects the import. While it is true, no global hotkeys are registered and an instruction notice is shown; it never terminates, edits or launches Swift. The guard is continuous, not setup-only: it is re-evaluated before every hotkey registration and resume (including after the recorder, Settings window close and `hotkeys_resume`), and a lightweight observer (a bounded poll of `is_bundle_running` on a background task, since no OS notification API is assumed) unregisters the hotkeys when Swift starts and restores them when it exits. Tests with the scripted fake: `swift_starts_after_setup_unregisters_hotkeys` and `swift_exits_restores_hotkeys`; a process-query `Err` fails closed (hotkeys stay unregistered). Transition Task 18 reuses `SwiftSource::is_running`.

**Commits:**
1. `feat(core): host-specific hotkey and settings defaults` (owns core `hotkey.rs`, `settings.rs`, shell `state.rs` callsites).
2. `feat(macos): menu-bar accessory lifecycle and template tray icon` (owns `Info.plist`, `lib.rs`, `windows.rs`, `tray.rs`, `mac-template.png`).
3. `feat(macos): Mac shortcut labels, recorder and Swift coexistence guard` (owns platform `hotkey.rs`, `keys.rs`, `system.rs` impls and fake, shell `hotkeys.rs`, `swift_source.rs`, `settings_cmds.rs`, `HotkeyRecorder.svelte`, `settingsTypes.ts`, `i18n/macos.json`, `.superpowers/cross-platform/task-9.md`).

- [ ] Write assertions `default_translate_for(HostKind::Unix) == Hotkey::new("KeyT", vec![Modifier::Alt])` (or Ctrl+Option+T per the probe), Mac win label contains `⌘` and no `Win`, Windows ctrl+alt label remains `Ctrl+Alt+T`; `old_settings_without_hotkeys_gets_platform_default` (same JSON, no `hotkeys` key, loads Option+T for Unix and Ctrl+Alt+T for Windows); Meta key recording, explicit cleared shortcut, conflicts after resume, dead-key/text input with Option, active map updates and menus without duplicate native accelerators; `swift_running_prevents_duplicate_global_hotkeys`. Existing hotkey reentrancy tests pass unchanged in intent.

**Regression anchor — `mac_translate_default_and_windows_labels_are_distinct`:** fixture is two loads of the same `settings.json` containing `{"activeProvider":"Claude"}` (no `hotkeys`) with `HostKind::Unix` and `HostKind::Windows`; the Swift guard test uses a `FakeSystem` where `is_bundle_running("com.lingopop.app")` is true.

```rust
assert_eq!(Hotkey::default_translate_for(HostKind::Unix), Hotkey::new("KeyT", vec![Modifier::Alt]));
assert_eq!(Hotkey::default_translate().display_string(), "Ctrl+Alt+T");
assert_eq!(mac_loaded.hotkeys.translate, Some(Hotkey::default_translate_for(HostKind::Unix)));
assert_eq!(registered_hotkeys_with_swift_running, 0);
```

- [ ] Run `cargo test -p lingopop-core hotkey settings --locked`, `cargo test -p lingopop-app hotkey --locked`, `npm test --prefix app -- src/settings/shortcuts.test.ts`; capture RED, implement, run GREEN. Run the macOS 13/14/15 Option+T probe and verify real menu-bar/no-Dock behavior on an installed build; record the evidence.
- [ ] Review and locally commit each commit above in order.

### Task 10: Lossless supported pasteboard snapshot and safe restoration

**Files:** Modify platform `clipboard.rs`, `macos/clipboard.rs`, `fake/clipboard.rs`, `selection.rs`, `inject.rs`, orchestrator `messages.rs`, `i18n/macos.json` (localized key for the unsupported-pasteboard message), and existing snapshot fixtures; add `tests/pasteboard_items.rs` and ignored native `tests/macos_clipboard.rs`. Windows-touching (`ClipboardFormat` loses `Copy`): `windows/clipboard.rs`, `windows/util.rs`, `selection.rs`, `inject.rs`, `fake/clipboard.rs`, `tests/windows_smoke.rs`; full Windows CI leg.

**Interfaces:** Extend `ClipboardFormat` with `NativePasteboard { item: u32, type_name: String }`; remove `Copy` and adapt consumers without changing Windows format identity. Preserve the `ClipboardSnapshot` entries contract and redacted Debug. Mac snapshots use native item/type entries so item boundaries and arbitrary readable types survive; `read_text` stays independent.
- Explicit skip list, recorded in `skipped` by name and NEVER read (reading a lazy provider can trigger it): `com.apple.pasteboard.promised-*`, `NSFilePromise*`, `dyn.*` UTIs, and any type whose provider is lazy (listed by `NSPasteboardItem.types` but without available data). Size cap 16 MiB per type: a larger type is skipped and recorded the same way.
- Restoration is all-or-nothing: if skipping would omit ANY non-empty item or representation (promised, lazy, `dyn.*`, oversized or unreadable), not only when no restorable entry remains, `snapshot()` returns `Unsupported("pasteboard snapshot")` before any copy or clear; `platform_message` maps it to a new localized key (`Clipboard content can't be preserved. Nothing was changed.`).
- Snapshot collection rechecks `changeCount` and retries three times, then `ClipboardBusy` (`snapshot_aborts_on_concurrent_foreign_write`). `clearContents` returns the post-write `changeCount`; that value is the sequence observed after OUR write and is what `RestoreGuard` compares. `restore_if` checks `changeCount` as close to the write as the API permits and returns false for an observed foreign write. NSPasteboard has no cross-process compare-and-swap: document this and do not claim Windows-style atomic exclusion. Preserve transient/concealed markers per item.

**Commits:**
1. `refactor(clipboard): make ClipboardFormat non-Copy and add native pasteboard entries` (owns portable `clipboard.rs`, `selection.rs`, `inject.rs`, `fake/clipboard.rs`, `windows/clipboard.rs`, `windows/util.rs`, `windows_smoke.rs` adaptations, fixtures).
2. `fix(macos): preserve pasteboard items and supported representations` (owns `macos/clipboard.rs`, `tests/pasteboard_items.rs`, `tests/macos_clipboard.rs`, `messages.rs`, `i18n/macos.json`).

- [ ] Write portable codec/fake tests: two items with text/RTF/PNG/file data roundtrip byte-for-byte and keep boundaries; `promised_type_aborts_without_read`; `oversized_type_aborts_before_copy`; `snapshot_aborts_on_concurrent_foreign_write`; any content that restoration would omit (promised, oversized or unreadable representation, even next to readable text) aborts before Copy; transient/concealed markers survive; a changed sequence prevents restore; partial own write reports failure and permits guard restoration; native type metadata/bytes never appear in Debug except allowed names/counts. Existing Windows protection-marker and RestoreGuard tests remain required.

**Regression anchor — `promised_type_aborts_without_read`:** fixture is a fake pasteboard with item 0 types `public.utf8-plain-text` (bytes `b"hi"`) and `com.apple.pasteboard.promised-file-url` (a read counter), and an item 1 whose only type is a 17 MiB `public.png`; a second fixture holds one item with only the promised type plus a `send_copy` counter.

```rust
assert_eq!(promised_read_calls, 0);
assert!(snapshot.skipped.contains(&"com.apple.pasteboard.promised-file-url".to_string()));
assert_eq!(first_fixture_result, Err(PlatformError::Unsupported("pasteboard snapshot".into()))); // text plus promised plus 17 MiB PNG: restore would drop items, so abort
assert_eq!((first_send_copy_count, first_clear_contents_count), (0, 0));
assert_eq!(only_promised_result, Err(PlatformError::Unsupported("pasteboard snapshot".into())));
assert_eq!((send_copy_count, clear_contents_count), (0, 0));
```

- [ ] Run `cargo test -p lingopop-platform --test pasteboard_items --locked` and selection/clipboard tests; capture RED, implement native item enumeration and restore, then GREEN. Native tests use a dedicated test clipboard session and are ignored by default because the real pasteboard is user state; owner-run permission is required before touching it.
- [ ] Review and locally commit each commit above in order.

### Task 11: Explicit desktop-space geometry and physical capture conversion

**Files:** Modify platform `monitors.rs`, `capture.rs`, `overlay.rs` documentation, Mac `monitors.rs`, orchestrator `placement.rs`/`popup.rs`, shell `popup_shell.rs` and `lib.rs` (`setup` initial topology capture); create Mac `geometry.rs`, portable `tests/macos_geometry.rs`; adjust Windows geometry documentation/tests without changing numeric behavior.

**Interfaces:** Keep existing Point/Rect as **desktop coordinates**: Windows physical virtual-screen pixels; Mac global Quartz points with top-left Y-down coordinates. Mac `MonitorInfo.scale_factor` for shared logical-popup-size conversion is `1.0`; keep actual display backing scale in native `MacDisplay { id, desktop_bounds, work_area, backing_scale }`. Work areas exclude menu bar/Dock. Define `MacGeometry::desktop_to_appkit(rect: Rect) -> Result<NativeRect, PlatformError>` and `capture_plan(rect: Rect) -> Result<CapturePlan, PlatformError>`, with exact float native bounds, display IDs, display-local pixel crops and an output size. Capture across displays composites at the largest intersecting backing scale and resamples lower-scale tiles; transparent/offscreen gaps are filled with opaque black, never unrelated screen content. CapturedImage always contains physical image pixels. A topology revision accompanies native plans and invalidates execution if displays changed. Ten-pixel overlay threshold means backing pixels on the starting display, not ten Quartz points.
- Topology cache: `MacMonitors` holds the topology captured on the main thread (initial capture in `setup`, refreshed on `NSApplicationDidChangeScreenParametersNotification`, with a `topology_revision()` counter). `monitors()` reads the cache from any thread without a `MainThreadMarker`; `ShellPopup::rect_for` reads the same cache, so no main-thread dispatch ever happens under the orchestrator commit lock.

**Commits:**
1. `feat(macos): add desktop geometry and capture plans` (owns `macos/geometry.rs`, `tests/macos_geometry.rs`, `monitors.rs`/`capture.rs`/`overlay.rs` docs, Windows geometry docs/tests).
2. `fix(geometry): cache display topology and position the popup from desktop coordinates` (owns Mac `monitors.rs`, `placement.rs`, `popup.rs`, shell `popup_shell.rs`, `lib.rs`).

- [ ] Write geometry tests: primary bounds `(0,0,1440,900)` at scale 2 yields a `(10,20,100,50)` desktop crop of 200x100 pixels with no double scaling; adjacent scale-1 display remains correctly positioned; above/left monitor negative origins and AppKit Y flip roundtrip; visible work area excludes menu bar/Dock; a two-display 1x/2x crop has deterministic tile offsets/output size; `display_removal_invalidates_capture_plan`; `monitors_read_off_main_uses_cache_without_marker`. Existing Windows popup placement/clamp values stay identical.

**Regression anchor — `retina_capture_scales_pixels_once`:** fixture is one `MacDisplay { bounds (0,0,1440,900), backing_scale 2.0 }` and the desktop rect `(10,20,100,50)`; `display_removal_invalidates_capture_plan` builds that plan, then publishes a topology without the display (revision +1).

```rust
assert_eq!(output_width, 200);
assert_eq!(output_height, 100);
assert_eq!(mac_shared_popup_scale, 1.0);
assert_eq!(execute_plan_after_removal, Err(PlatformError::InvalidRegion));
```

- [ ] Run `cargo test -p lingopop-platform --test macos_geometry --locked` and `cargo test -p lingopop-orchestrator placement --locked`; capture RED, implement conversions and route shell positioning through desktop geometry rather than always calling Tauri PhysicalPosition on Mac, then GREEN. Explicitly verify Mac floating-point-to-integer rounding with half-open rectangles.
- [ ] Review and locally commit each commit above in order.

### Task 12: Exact Mac source targeting for capture, tray actions and Replace

**Files:** Modify Mac `foreground.rs`, `last_foreground.rs`, `keys.rs`, portable `foreground.rs`, `inject.rs` only as required, orchestrator `messages.rs`, `i18n/macos.json` (new keys below, pl/uk/de), fake targeting tests and `tests/last_foreground.rs`; add `macos/target_registry.rs`, `macos/ax.rs` (raw `extern "C"` FFI to ApplicationServices, `// SAFETY:` on every block), ignored `tests/macos_targeting.rs`. Windows-touching: portable `foreground.rs`, `fake/foreground.rs`, `windows/foreground.rs` (new trait method); full Windows CI leg.

**Interfaces:** Keep opaque `WindowHandle(u64)` and Windows HWND behavior. On Mac, handles identify generation-stamped registry entries retaining the captured application PID, bundle id, focused AX window/element where available, and a bounded lifetime; an evicted/stale target fails safely. Add `ForegroundWindow::is_current_target(handle) -> Result<bool, PlatformError>` (`Ok(false)` = lost or stale target; Accessibility revocation is `Err(PermissionDenied(Accessibility))` and callers propagate it unchanged; Windows compares the native HWND, fake supports target changes). The pre-paste check is NOT atomic: it narrows the race, it does not eliminate it, and docs/tests say so. Rule: refuse to paste only when the frontmost pid differs from the captured pid (or the entry is evicted). AX element identity is best effort: `AXManualAccessibility` is attempted for Chromium/Electron apps; without AX identity the check is pid + bundle id level; Replace is never refused solely for missing AX identity. A same-pid different-window case is therefore a documented best-effort limit, not a guarantee. Permission failure (Accessibility revoked) is `PermissionDenied`, distinct from target loss; a foreground change after restore returns the existing uncertain/failed Replace outcome and never sends to the new app.
- New keys (`i18n/macos.json`): `The original app is no longer in front. The text is on the clipboard: press ⌘V to paste.` and `Replace needs Accessibility access. Allow LingoPop in System Settings, then try again.`

**Commits:**
1. `feat(platform): add is_current_target to ForegroundWindow` (owns portable `foreground.rs`, `fake/foreground.rs`, `windows/foreground.rs`, `inject.rs`).
2. `fix(macos): preserve the original source target for Replace` (owns `macos/{foreground,last_foreground,keys,target_registry,ax}.rs`, `tests/last_foreground.rs`, `tests/macos_targeting.rs`, `messages.rs`, `i18n/macos.json`).

- [ ] Write tests: current target roundtrips; `different_frontmost_pid_never_receives_paste`; process exit/PID reuse is rejected; evicted registry handle is rejected; missing AX identity still allows Replace at pid+bundle level; tray remembers the last external target and ignores own windows; `revoked_accessibility_never_pastes`; `target_lost_shows_manual_paste`; a foreground change after restore never sends to the new app. Assert no selected text/window titles are logged.

**Regression anchor — `revoked_accessibility_never_pastes`:** fixture is `FakeForeground` + `FakeKeys` with the original target captured, then `FakePermissions` Accessibility set to `Denied` before Replace; the second case switches the fake frontmost pid to another app.

```rust
assert_eq!(replace_outcome, Err(PlatformError::PermissionDenied(PermissionKind::Accessibility)));
assert_eq!(send_paste_count, 0);
assert_eq!(foreground.is_current_target(original_target), Ok(false)); // after pid switch
assert_eq!(clipboard_text_after_target_lost, displayed_result);
```

- [ ] Run `cargo test -p lingopop-platform --test last_foreground --locked` plus inject/target unit tests and `cargo test -p lingopop-orchestrator replace --locked`; capture RED, implement target registry/verification, then GREEN. Real multiwindow/fullscreen/Spaces tests remain separately recorded, not simulated as complete.
- [ ] Review and locally commit each commit above in order.

### Task 13: Generation-fenced queued presentation with acknowledgments

**Files:** Modify orchestrator `popup.rs`, `orchestrator.rs`, `actions.rs`, `fake.rs`, `session.rs`, `lib.rs`, `status.rs` and affected existing tests; create portable `presentation.rs`, `tests/presentation_ordering.rs`; modify shell `popup_shell.rs` to implement the queue protocol on a dedicated worker thread using current Windows native calls until Task 14; modify Storage Task 5 `prepare_shutdown`. Windows-touching: all of it (full Windows CI leg plus required desktop smoke).

**Interfaces:**
- Types: `PresentationId { generation: u64, sequence: u64 }`; `PresentationKind::{Show { origin: Point, size_logical: (f64, f64), model: PopupViewModel }, Update { model }, Resize { origin, size_logical }, Hide}`; `PresentationVisibility::{Visible, Hidden, Superseded}`; `PresentationAck { id, result: Result<PresentationVisibility, PlatformError> }`; `PresentationQueueError::{Full, Closed}` (a rejected command is never counted as accepted or visible).
- `PopupView::submit(cmd: PresentationCommand) -> Result<AckReceiver, PresentationQueueError>` where `AckReceiver` is a `tokio::sync::oneshot::Receiver<PresentationAck>`. There is no shared crossbeam `acknowledgments()` receiver. The shell consumer delivers every ack to `Orchestrator::presentation_acked(ack)` and then resolves that command's oneshot.
- Lock-free `PresentationFence`: `AtomicU64 generation` + `AtomicU64 sequence` (the "latest id"); `advance(id)` = store; `apply_if_current(id, effect) -> Option<PresentationAck>` = load + compare, run the effect WITHOUT holding any lock, re-load afterwards; if the id went stale during the effect the ack is `Superseded` and the superseding command (always later in the queue) fixes the screen. No fence mutex exists. `Orchestrator::start` advances the fence inside the generation commit, so every queued command of an older generation is stale once a new generation begins.
- Queue: capacity 32 for Show/Update/Resize; Hide bypasses the capacity limit (it is never rejected with `Full`) but, like every submission, returns `PresentationQueueError::Closed` once the queue is closed; it coalesces onto a pending Hide and cancels pending Show/Update/Resize of the same generation); Update replaces the model of a pending Show; Resize merges origin/size into a pending Show; a Show never reorders past a later Hide. `Full` applies only to non-Hide commands; a coalesced-away command's oneshot resolves `Superseded`. Explicit `Closed` plus a worker-shutdown acknowledgment.
- Orchestrator visibility: `Visibility::{Hidden, Pending(id), Visible(id)}` replaces the synchronous `is_visible()` contract. The dismiss watcher starts on a `Visible` ack whose id is current; a `Visible` ack for a stale id while a newer command is pending is ignored; a `Visible` ack for a stale id with nothing pending and no session enqueues Hide. `present_inner`, `popup_resized`, `schedule_auto_close` and Replace migrate to it; existing `try_lock` assertions are updated.
- Replace awaits its own Hide oneshot outside all locks (injected deadline, default 2 s), then revalidates session/target, then pastes; a timeout, stale, `Full`/`Closed` or failed Hide never pastes.
- No native dispatch or wait happens under the session/commit lock; submit under a lock only advances the fence and enqueues. Storage Task 5's `prepare_shutdown` also waits for the final Hide ack. The Windows consumer is a dedicated worker thread (not main), so the AGENTS.md rule that the main thread stays free to service cross-thread `SetWindowPos` is unchanged. Keep listener-before-`popup_ready` and newest-model re-emission.

**Commits:**
- 13a `refactor(popup): add the portable presentation protocol, fence and fake` (owns `presentation.rs`, `fake.rs`, `popup.rs` types, `tests/presentation_ordering.rs`).
- 13b `refactor(orchestrator): migrate popup flows to queued presentation` (owns `orchestrator.rs`, `actions.rs`, `session.rs`, `status.rs`, `lib.rs`, existing orchestrator tests, Storage Task 5 `prepare_shutdown`).
- 13c `refactor(shell): consume presentation commands on a dedicated Windows worker` (owns shell `popup_shell.rs`, shell popup tests).

- [ ] Write deterministic `presentation_ordering` tests with explicit channels (no sleeps): `old_show_queued_during_new_capture_has_zero_effects` (RED first); enqueue old Show, supersede, execute old job -> no native effect; update/hide/new show preserves newest model; Hide accepted at full capacity and cancels pending Show/Update/Resize; Update merges into pending Show; Resize merges into pending Show; Show never passes a later Hide; stale-after-effect acks `Superseded`; old ack cannot start dismissal; failed Show leaves Hidden; inline/reentrant consumer can submit a new operation without deadlock; duplicate ack is inert; Replace pastes only after its own successful Hide ack and never after timeout/cancellation/supersession. Use injected ack deadline futures, not a wall-clock assertion. Persistence generation tests still pass.

**Regression anchors:**

`old_show_queued_during_new_capture_has_zero_effects` — fixture: `FakePopup` consumer with a blocked queue; generation 1 Show is queued, then `Orchestrator::start` begins generation 2 (fence advanced), then the consumer is released.

```rust
assert_eq!(stale_native_effect_count, 0);
assert_eq!(old_show_ack.result, Ok(PresentationVisibility::Superseded));
assert_eq!(visibility, Visibility::Hidden);
```

`superseded_show_and_hide_ack_do_not_paste` — fixture: Replace issued after Show; the fake consumer holds the Hide, first acks a stale-generation Hide, then the current one.

```rust
assert_eq!(paste_count_before_current_hide_ack, 0);
assert_eq!(paste_count_after_stale_hide_ack, 0);
assert_eq!(paste_count_after_current_hide_ack, 1);
```

- [ ] Run `cargo test -p lingopop-orchestrator --test presentation_ordering --locked`; capture RED. Implement 13a, then 13b, then 13c, updating all old PopupView tests to assert ack-confirmed visibility.
- [ ] Run all orchestrator tests, shell popup tests and the required Windows desktop smoke in CI. Expected: zero stale effects, hide-before-paste preserved, no enqueue success reported as on-screen success.
- [ ] Review and locally commit each commit above in order (final subject of 13c keeps the PR title `refactor(popup): acknowledge generation-fenced native presentation`).

### Task 14: Native AppKit popup and dismiss monitoring

**Files:** Modify Mac `popup_window.rs`, `dismiss` implementation currently in that file, Mac module exports, platform `Cargo.toml` (`objc2-app-kit` features `NSPanel`, `NSWindow`, `NSScreen`, `NSEvent`, `NSView`, `NSApplication`, `NSResponder`; verify each exists in the pinned 0.3.x before use), `i18n/macos.json` (Escape-unavailable notice, pl/uk/de); create `macos/presentation.rs`, `macos/dismiss.rs`, shell `macos_main_thread.rs`, portable worker tests and ignored `tests/macos_popup.rs`; wire `popup_shell.rs`, shell setup and native teardown.

**Interfaces:** Task 13 owns ordering/ack/fence; Task 11 owns desktop conversion and the topology cache.
- Popup window: at `setup` on main, the tao `NSWindow` is converted to an in-repo `NSPanel` subclass through `object_setClass` (the `tauri-nspanel` technique, no new dependency; the subclass adds no ivars). Style `nonActivatingPanel | borderless`, level `NSPopUpMenuWindowLevel`, collectionBehavior `canJoinAllSpaces | fullScreenAuxiliary | stationary`, `hidesOnDeactivate = false`, `becomesKeyOnlyIfNeeded = true`, and `acceptsFirstMouse` so Copy and text selection work without activating the app. The `NSWindow` pointer is cached once in `setup` on main (like the Windows hwnd); no `PopupView` method fetches it.
- `MacPresentationConsumer::apply(kind: &PresentationKind) -> Result<PresentationVisibility, PlatformError>` lives in the shell on the Task 13 worker and runs native show/order-without-activation, resize/update and hide by dispatching to main and waiting off every lock. The shell translates `PresentationKind` into platform-local `MacWindowCommand` (native handle + rect); platform gets no Tauri/orchestrator dependency. Remove Mac Tauri show/hide activation fallbacks.
- Dismiss: passive `NSEvent` global + local monitors; outside click through mouse-down monitors (no permission; a click inside the popup rect is ignored); Escape through a global keyDown monitor, which requires Accessibility trust (already needed for paste). Escape is NOT swallowed on macOS (documented difference from Windows). Without trust, Escape dismissal is unavailable (reported through the Escape-unavailable key) while click-outside still works. Watcher methods stay non-blocking; monitor callbacks only emit events and never call the orchestrator; own injected input does not dismiss; shutdown removes monitors.

**Commits:**
1. `feat(macos): configure the popup as a non-activating panel` (owns `popup_window.rs`, `macos/presentation.rs`, `macos_main_thread.rs`, platform `Cargo.toml`, shell setup).
2. `feat(macos): present the popup through the queued consumer` (owns `popup_shell.rs`, native teardown, `tests/macos_popup.rs`).
3. `feat(macos): dismiss the popup with passive event monitors` (owns `macos/dismiss.rs`, `i18n/macos.json`).

- [ ] Write fake/main-dispatch tests: `stale_show_after_dismiss_has_no_effect`; `hide_ack_precedes_paste` (Replace's Hide ack resolves before any paste on the Mac consumer); no caller lock spans dispatch; Show returns its ack only after native completion; hide/worker shutdown removes monitors; click inside is ignored, outside click and physical Escape deliver once; own injected input does not dismiss; missing Accessibility trust reports Escape unavailable, installs click-outside and leaks no monitor; listener registration precedes `popup_ready` and queued latest-model replay stays newest. The real-focus check `foreground_after_show == foreground_before_show` lives only in ignored `tests/macos_popup.rs` as installed-app acceptance, not a fake test.

**Regression anchor — `stale_show_after_dismiss_has_no_effect`:** fixture is a fake main-thread dispatcher with a queued Show (generation 3) and a `DismissEvent::ClickOutside` delivered before the dispatcher runs, with a `native_monitors` counter on the fake dismiss backend.

```rust
assert_eq!(stale_native_show_count, 0);
assert_eq!(ack.result, Ok(PresentationVisibility::Superseded));
assert_eq!(native_monitors_after_shutdown, 0);
```

- [ ] Run `cargo test -p lingopop-app popup --locked` and platform Mac presenter/dismiss unit tests; capture RED, verify required AppKit bindings/minimum OS, implement, then GREEN. Real installed-app acceptance covers retained source focus, mouse text selection/Copy, Settings focus, full screen/Spaces and repeated reopen.
- [ ] Review and locally commit each commit above in order.

### Task 15: Native screenshot capture and Vision OCR

**Files:** Modify Mac `capture.rs`, `ocr.rs`, platform `Cargo.toml` (target dependencies: `objc2-vision` and the Core Graphics bindings) and native wiring, `i18n/macos.json` (new keys below, pl/uk/de); create portable `tests/macos_capture_contract.rs`, ignored `tests/macos_screen.rs`; retain shared `capture.rs`/`ocr.rs` APIs.

**Interfaces:** Implement `ScreenCapture::capture_region(Rect) -> Result<CapturedImage, PlatformError>` through Task 11 capture plans and `Ocr::recognize(&self, image: &CapturedImage) -> Result<OcrText, PlatformError>`.
- Capture: the macOS 13 floor means `CGDisplayCreateImageForRect` (the `core-graphics` crate is already present; the API is deprecated in 15 but functional), behind a mandatory `CGPreflightScreenCaptureAccess` / `CGRequestScreenCaptureAccess` preflight from Task 8: without permission the API returns a wallpaper-only image, not an error, so the adapter must never call it unpermitted. After the call it repeats `CGPreflightScreenCaptureAccess` and, if access is no longer granted, returns `PermissionDenied` without exposing the image (this narrows the revocation race, it does not eliminate it; the Task 8 relaunch hint is unchanged). If the macOS 15 SDK refuses to link the symbol, stop and report `NEEDS_CONTEXT` with the linker evidence.
- OCR: `objc2-vision` pinned to a version verified in the registry (`VNRecognizeTextRequest`, accurate recognition level, language correction on, matching the Swift reference), or `NEEDS_CONTEXT` if no version builds at the floor. Preserve reading order, trimmed blank detection, existing OCR image limits/encoding and vision Retry provider eligibility. Captured rows are top-down RGBA with alpha 255, correct stride/color conversion, and no retained native buffer escapes its owning scope.
- New keys (`i18n/macos.json`): `Screen capture failed. Try again.` and `Text recognition failed. Try again.` (Screen Recording denial uses the Task 8 keys).

**Commits:**
1. `feat(macos): capture screen regions behind the permission preflight` (owns `macos/capture.rs`, `tests/macos_capture_contract.rs`, `tests/macos_screen.rs`, platform `Cargo.toml`).
2. `feat(macos): recognize text with Vision` (owns `macos/ocr.rs`, `i18n/macos.json`, native wiring).

- [ ] Write contract tests: 100x50 desktop points on a 2x screen produces 200x100 pixels; clipping and multi-display composition follow CapturePlan exactly; denied/revoked permission returns PermissionDenied and never a successful empty image; padded row strides are copied correctly; empty OCR is a real empty result and vision retry retains the captured image. Verify rotated/orientation-aware OCR on a fixed native fixture without printing recognized user text.

**Regression anchor — `screen_permission_denial_is_not_empty_success`:** fixture is `MacCapture` over an injected `NativeScreen` fake that counts `create_image` calls and a `FakePermissions` with ScreenRecording `Denied`; the stride case feeds a 3x2 image with 16-byte padded rows.

```rust
assert_eq!(result, Err(PlatformError::PermissionDenied(PermissionKind::ScreenRecording)));
assert_eq!(native_capture_calls, 0);
assert_eq!(padded_rows_output.len(), 3 * 2 * 4);
assert_eq!(output_alpha_values, vec![255; 6]);
```

- [ ] Run `cargo test -p lingopop-platform --test macos_capture_contract --locked` and orchestrator screen tests; capture RED, implement adapters after native API/dependency validation, then GREEN. Native screen tests require explicit real-screen authorization and must not capture the developer desktop in default CI.
- [ ] Review and locally commit each commit above in order.

### Task 16: Native Mac region overlay with serialized cancellation

**Files:** Modify Mac `overlay.rs`, shell `picker.rs` only for required lifecycle integration, `i18n/macos.json` (no new keys: the size label is numeric; `catalogCoverage` and `check` must stay green); create `macos/overlay_window.rs`, portable reducer/lifecycle tests and ignored `tests/macos_overlay.rs`; extend orchestrator screen tests.

**Interfaces:** Implement existing `RegionOverlay::pick(cancel: Receiver<()>) -> Result<Option<Rect>, PlatformError>`. Worker posts native overlay creation/input work to main and waits only off-main; cancellation/disconnect owns cleanup acknowledgment before another overlay starts. Shared ordered `fold_input_batch` remains the reducer, Task 11 maps desktop geometry and the starting-display 10-physical-pixel threshold. On display topology change (Task 11 revision), cancel with cleanup (`Ok(None)`, fixed log line) instead of capturing a new unintended region. Overlay windows cannot become the remembered external source target. New keys: none.

**Commits:**
1. `feat(macos): add the native overlay window and lifecycle` (owns `macos/overlay_window.rs`, `overlay.rs`, `tests/macos_overlay.rs`).
2. `feat(macos): serialize overlay cancellation with the region picker` (owns shell `picker.rs`, orchestrator screen tests).

- [ ] Write deterministic tests: Esc, dropped cancel sender, too-small and reverse drags return None/normalized rectangle appropriately; coalesced moves preserve Down/Up order; cancel/restart max native overlays is 1; capture loss may cancel as approved by the original Windows overlay spec; display removal cancels; cleanup precedes capture so overlay tint does not enter screenshots. Verify exact threshold at 1x/2x and negative origins.

**Regression anchor — `cancel_restart_waits_for_overlay_cleanup`:** fixture is a fake native overlay backend recording `create`/`cleanup` sequence numbers; the first pick is cancelled by dropping its cancel sender, and a second pick is requested immediately.

```rust
assert_eq!(first_result, Ok(None));
assert_eq!(max_native_overlays, 1);
assert!(first_cleanup_sequence < second_create_sequence);
```

- [ ] Run `cargo test -p lingopop-platform --test overlay_geometry --locked`, Mac overlay unit tests and orchestrator screen tests; capture RED, implement native overlay, then GREEN. Real multi-monitor mouse tests are ignored owner-run tests; preserve required Windows smoke and optional input-driven test classification.
- [ ] Review and locally commit each commit above in order.

### Task 17: Functional login items, repeat launch and native readiness gate

**Files:** Modify Mac `login_item.rs`, platform `error.rs` and orchestrator/shell message mapping (new `PlatformError::LoginItemRequiresApproval`; update the variant-enumeration test; Windows-touching, full Windows CI leg), shell `lib.rs`/single-instance handling, `app/src-tauri/tauri.macos.conf.json`, `i18n/macos.json` (`Allow LingoPop in System Settings > Login Items to start it at login.`, `Open Login Items settings`, pl/uk/de); create `macos/service_management.rs`, platform `tests/macos_login.rs`, `scripts/check-macos-production.sh`; update `scripts/check-core-deps.sh` only if adding a separate production guard entry.

**Interfaces:** Implement existing `LoginItem::set_enabled(enabled: bool, _exe: &Path) -> Result<(), PlatformError>` through `SMAppService.mainApp` (ServiceManagement, macOS 13+) by objc2 `msg_send` to the `SMAppService` class; if `objc2-service-management` is not in the registry, use a raw class/framework declaration in `src/macos/service_management.rs` with `// SAFETY:`. On macOS the exe argument is ignored (Windows keeps using it). Status mapping: `enabled` ok; `notRegistered` after disable ok; `requiresApproval` returns `LoginItemRequiresApproval`, shown with the "Open Login Items settings" key and an action calling `SMAppService.openSystemSettingsLoginItems`; `notFound` and an unbundled/relocated executable return explicit `Unsupported`. No LaunchAgent is written. Enable/disable jobs retain latest-value fencing. Coexistence detection with Swift is Task 9's `SwiftSource::is_running` (no hotkey guard here); Swift login data is never read or changed.
- `tauri.macos.conf.json` is auto-merged by Tauri v2 and owns `bundle.macOS.minimumSystemVersion = "13.0"`; it never contains a `version` key (the shared version stays in `Cargo.toml`/`tauri.conf.json`).
- `check-macos-production.sh` fails if Mac native wiring selects plaintext/no-op/Unsupported dev adapters, if the `dev-plaintext` feature is enabled in any release/packaging configuration, or if the release config misses the approved identity or minimum OS; it complements behavior tests rather than being the sole proof.

**Commits:**
1. `feat(macos): register the login item through SMAppService` (owns `macos/login_item.rs`, `macos/service_management.rs`, `error.rs`, message mapping, `tests/macos_login.rs`, `i18n/macos.json`).
2. `feat(macos): add the native production readiness gate` (owns `scripts/check-macos-production.sh`, `tauri.macos.conf.json`, shell `lib.rs`, `check-core-deps.sh` if needed).

- [ ] Write fake/native tests: `login_enable_targets_main_app_and_ignores_exe`; late queued enable cannot override newer disable; `requires_approval_is_not_silent_success`; unbundled executable returns `Unsupported`; second Rust launch opens Settings and registers no duplicate native services; relocate/upgrade failure shows a fixed actionable error without touching Swift login data. Readiness guard fails against intentional fixtures containing `DevPlaintextProtector` wiring and an enabled `dev-plaintext` feature, and passes real production wiring.

**Regression anchor — `requires_approval_is_not_silent_success`:** fixture is a fake `SMAppService` backend returning status `requiresApproval` after `register`, with a Swift login-item write counter.

```rust
assert_eq!(result, Err(PlatformError::LoginItemRequiresApproval));
assert_eq!(platform_message(&result.unwrap_err()).key, KEY_LOGIN_REQUIRES_APPROVAL);
assert_eq!(swift_login_item_writes, 0);
assert!(!guard_passes_with_dev_plaintext_fixture);
```

- [ ] Run `cargo test -p lingopop-app task5 --locked`, platform login tests and the readiness guard; capture RED, implement, then GREEN. Native login tests use a throwaway installed test bundle and explicit cleanup; default tests never change the user's actual login configuration. Run the shared PR gate and installed-app acceptance for permission persistence across upgrade (ad-hoc previews re-prompt: expected).
- [ ] Review and locally commit each commit above in order.
