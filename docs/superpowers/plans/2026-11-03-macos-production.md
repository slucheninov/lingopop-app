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
- Non-text/multi-item/promised pasteboard content and foreign writes cannot be silently destroyed (Task 10).
- Per-monitor scaling, target loss and changed display topology cannot redirect capture/Replace (Tasks 11–12, 15–16).
- Reentrant main-thread callbacks and out-of-order show/hide acknowledgments cannot deadlock or resurrect a stale popup (Tasks 13–14).

---

### Task 6: Own shared and platform localization in this repository

**Files:** Create authoritative `i18n/catalogs/{pl,uk,de}.json` from committed generated values and `i18n/macos.json`; modify `i18n/SOURCE.txt`, `scripts/gen-localizations.py`, `crates/lingopop-core/src/i18n.rs`, shell `settings_cmds.rs`/message callsites, `AGENTS.md`; add `scripts/tests/test_localizations.py`. Remove generated/exclusion paths only after every include/check/README reference is migrated in the same commit.

**Interfaces:** Core `CatalogPlatform::{Windows, Macos}`; `lookup_for<'a>(platform: CatalogPlatform, language: Language, key: &'a str) -> Cow<'a, str>`, `catalog_for(platform: CatalogPlatform, language: Language) -> &'static HashMap<String, String>`, `t_for(platform: CatalogPlatform, language: Language, key: &str, args: &[&str]) -> String`. Preserve existing `lookup`/`catalog`/`t` as Windows wrappers until all shell callsites inject platform. Python `check_catalogs(root: Path) -> list[str]` returns deterministic validation issues and the CLI prints them with its existing exit-code contract. `gen-localizations.py check` validates shared + both platform catalogs without Swift checkout; optional `generate` now operates on repository catalogs, never overwrites authoritative values from Swift. Each platform key is distinct from shared keys; promoted shared values retain exact placeholders/newlines. Platform-specific equivalent phrases use separate keys when required.

- [ ] Write core/Python tests: Windows resolves shared+windows but not Mac-only permission/Command keys; Mac resolves shared+macos; English returns the key; a Windows value containing `⌘` fails; a correctly signed Mac value containing `⌘` passes; bad `%1$@`, newline and `{{targetLanguage}}` signatures fail for either platform. Test with nonexistent `LINGOPOP_MAC_REPO` and network disabled. Assert existing Windows merged catalogs remain semantically identical. Promote Task 5's common storage strings into shared catalogs.

**Regression anchor — `windows_rejects_command_symbols_but_macos_accepts_them`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```python
assert windows_issues != []
assert macos_issues == []
assert merged_windows_before == merged_windows_after
```

- [ ] Run `python3 -m unittest discover -s scripts/tests -p 'test_localizations.py'` and `cargo test -p lingopop-core i18n --locked`; capture RED, implement catalog ownership/injected composition, then rerun plus `python3 scripts/gen-localizations.py check`.
- [ ] Review and locally commit: `refactor(i18n): own shared and platform catalogs in the app repository`.

### Task 7: Stable Mac identity and Keychain-backed secret protection

**Files:** Create `app/src-tauri/tauri.macos.conf.json`, `crates/lingopop-platform/src/macos/{secrets.rs,keychain.rs}`, `crates/lingopop-platform/tests/macos_secrets.rs`; modify shell `paths.rs`, platform `native.rs`, `lib.rs`, `dev_protector.rs`, Mac `mod.rs`, platform `Cargo.toml`/lockfile and affected native-bundle tests.

**Interfaces:** `MacProtector::new() -> Self` implements existing `SecretProtector`. Injectable `KeychainKeyStore` supplies `load() -> Result<Option<SecretKey>, ProtectError>` and atomic `insert_if_absent(key: SecretKey) -> Result<SecretKey, ProtectError>`; key Debug is redacted and memory is cleared on drop with a vetted implementation. Use a randomly generated 256-bit key with AES-256-GCM authenticated encryption, random 96-bit nonce per message, a versioned format header authenticated as associated data, and fixed Keychain service/account derived from the stable Mac identifier. Verify compatible crypto/native bindings before selecting versions. A header parse, missing existing decryption key, locked Keychain, failed persistence or invalid tag is a hard error; no fallback. Restrict `DevPlaintextProtector` to tests/explicit isolated fixtures, never `native()` for a shipped platform. LocalPaths on Mac use the approved directory; existing `LingoPop-dev` is not an import source.

- [ ] Write tests with injected key store: ciphertext differs from plaintext; fresh nonce changes ciphertext; tamper/wrong key/unknown format returns error; read never generates a replacement key for existing ciphertext; a first-launch insert race returns the one durable key; failed key save produces no secrets file; locked Keychain does not quarantine/replace a valid file merely because access is temporarily denied. Add `native_mac_protector_is_not_identity` bundle test and sentinel Debug/Display checks. Pin Windows directory/config identity unchanged.

**Regression anchor — `unavailable_keychain_never_replaces_ciphertext`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert!(load_result.is_err());
assert_eq!(ciphertext_after, ciphertext_before);
assert_eq!(generated_key_count, 0);
```

- [ ] Run `cargo test -p lingopop-platform --test macos_secrets --locked` on Mac (fake crypto/format tests remain portable where possible) and focused core secret tests; capture RED. Adapt shell/core load classification as necessary so transient protector unavailability leaves source ciphertext untouched and writable secrets unavailable, while genuinely corrupt authenticated payloads retain existing explicit quarantine semantics.
- [ ] Implement native protector/identity; verify `tauri.macos.conf.json` override is explicitly included in development/packaging commands, not assumed to be auto-loaded by filename. Run GREEN, audit the native bundle and required core-dependency guard. Keychain native integration tests use a throwaway service and explicit owner-run marker; do not touch real keys.
- [ ] Review and locally commit: `feat(macos): protect local secrets with a Keychain-held encryption key`.

### Task 8: Explicit permission state and recovery actions

**Files:** Create platform `permissions.rs`, `fake/permissions.rs`, `macos/permissions.rs`; modify `native.rs`, platform/fake module exports, `error.rs`, existing fake bundle constructors, orchestrator `messages.rs`; create shell `permissions_cmds.rs`, frontend `lib/permissionsIpc.ts`, `settings/PermissionsSection.svelte`, permission tests and Mac catalogs; wire through shell `lib.rs`/MainTab.

**Interfaces:** `PermissionKind::{Accessibility, ScreenRecording}`, `PermissionState::{NotDetermined, Denied, Granted, Restricted}`, trait `Permissions: Send + Sync` with non-prompting `state(kind) -> PermissionState`, user-action `request(kind) -> Result<PermissionState, PlatformError>`, and `open_settings(kind) -> Result<(), PlatformError>`. Add `Platform.permissions: Arc<dyn Permissions>` with no-prompt Granted Windows/fake defaults. Add `PlatformError::PermissionDenied(PermissionKind)` so Mac never shows Windows UIPI instructions. Async shell IPC exposes statuses/request/open/refresh with fixed typed keys; do not add arbitrary settings URLs from the webview.

- [ ] Write fake/shell/frontend tests: bootstrap checks never prompt; selecting Screen Translate requests only ScreenRecording when needed; selection input requests only Accessibility; denied/revoked access keeps controls actionable; returning from System Settings refreshes state; retry after grant works without a permanent cached denial. Assert Windows InputBlocked text remains unchanged. Verify permission actions never log screenshot/text data.

**Regression anchor — `bootstrap_does_not_prompt_and_revocation_is_recoverable`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(request_calls_after_bootstrap, 0);
assert_eq!(denied_capture, Err(PlatformError::PermissionDenied(PermissionKind::ScreenRecording)));
assert!(capture_after_grant.is_ok());
```

- [ ] Run `cargo test -p lingopop-app permission --locked`, `cargo test -p lingopop-orchestrator messages --locked`, and `npm test --prefix app -- src/settings/permissions.test.ts`; capture RED, implement native queries/actions after SDK verification, then GREEN. Record real installed-app denied/granted/revoked coverage separately.
- [ ] Review and locally commit: `feat(macos): expose permission status and recovery actions`.

### Task 9: Platform shortcut defaults, labels and menu-bar app behavior

**Files:** Modify core `hotkey.rs`, platform `hotkey.rs`/Mac `keys.rs`, shell `hotkeys.rs`, `settings_cmds.rs`, `lib.rs`, `tray.rs`, frontend `settings/HotkeyRecorder.svelte`, `lib/settingsTypes.ts`; add `macos/app_lifecycle.rs`, portable shortcut tests and frontend shortcut cases; update Mac catalogs.

**Interfaces:** Portable `HotkeyPlatform::{Windows, Macos}`; `Hotkey::display_for(platform) -> String` and `Hotkey::default_translate_for(platform) -> Hotkey`. Keep persisted modifier raw values `ctrl/alt/shift/win`; Mac interprets/displays win as Command and alt as Option. Existing `display_string()`/`default_translate()` retain Windows output for compatibility. `UiBootstrap` adds explicit platform for shortcut recording/labels. Shell only applies a Mac default Option+T when no persisted Translate binding exists; an explicitly cleared binding stays cleared. Native accessory-app lifecycle is initialized on the main thread before windows/menu creation; explicit Settings/History opening remains possible.

- [ ] Write assertions `default_translate_for(Macos) == Hotkey::new("KeyT", vec![Modifier::Alt])`, Mac win label contains `⌘` and no `Win`, Windows ctrl+alt label remains `Ctrl+Alt+T`. Test Meta key recording, explicit cleared shortcut, conflicts after resume, dead-key/text input with Option, active map updates and menus without duplicate native accelerators. Existing hotkey reentrancy tests must pass unchanged in intent.

**Regression anchor — `mac_translate_default_and_windows_labels_are_distinct`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(Hotkey::default_translate_for(HotkeyPlatform::Macos), Hotkey::new("KeyT", vec![Modifier::Alt]));
assert_eq!(Hotkey::default_translate().display_string(), "Ctrl+Alt+T");
```

- [ ] Run `cargo test -p lingopop-core hotkey --locked`, `cargo test -p lingopop-app hotkey --locked`, `npm test --prefix app -- src/settings/shortcuts.test.ts`; capture RED, implement minimal defaults/UI/lifecycle changes, run GREEN. Verify real menu-bar/no-Dock behavior and Option+T registration on an installed Mac build; record required runtime evidence.
- [ ] Review and locally commit: `feat(macos): adapt shortcuts and menu-bar lifecycle`.

### Task 10: Lossless supported pasteboard snapshot and safe restoration

**Files:** Modify platform `clipboard.rs`, `macos/clipboard.rs`, `fake/clipboard.rs`, Windows clipboard match/copy callsites and existing snapshot fixtures; add `tests/pasteboard_items.rs` and ignored native `tests/macos_clipboard.rs`.

**Interfaces:** Extend `ClipboardFormat` with `NativePasteboard { item: u32, type_name: String }`; remove Copy and adapt consumers without changing Windows format identity. Preserve the existing ClipboardSnapshot entries contract and redacted Debug. Mac snapshots use native item/type entries so item boundaries and arbitrary readable types survive; read_text remains independent. An unreadable/promised type that would be lost makes `snapshot()` return fixed `Unsupported("pasteboard snapshot")` before any synthesized copy or clearing occurs. Snapshot collection rechecks changeCount and retries three times, then ClipboardBusy. `restore_if` checks changeCount as close as the native API permits and returns false for an observed foreign write; document that NSPasteboard has no cross-process compare-and-swap and do not claim Windows-style atomic exclusion. Preserve transient/concealed markers per item.

- [ ] Write portable codec/fake tests: two items with text/RTF/PNG/file data roundtrip byte-for-byte and keep boundaries; unreadable promised data aborts before Copy; transient/concealed markers survive; a changed sequence prevents restore; partial own write reports failure and permits guard restoration; native type metadata/bytes never appear in Debug except allowed names/counts. Existing Windows protection-marker and RestoreGuard tests remain required.

**Regression anchor — `promised_pasteboard_content_aborts_before_copy`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert!(snapshot_result.is_err());
assert_eq!(send_copy_count, 0);
assert_eq!(clear_contents_count, 0);
```

- [ ] Run `cargo test -p lingopop-platform --test pasteboard_items --locked` and selection/clipboard tests; capture RED, implement native item enumeration and restore, then GREEN. Native tests snapshot/restore a dedicated test clipboard session and are ignored by default because the real pasteboard is user state; owner-run permission is required before touching it.
- [ ] Review and locally commit: `fix(macos): preserve pasteboard items and supported representations`.

### Task 11: Explicit desktop-space geometry and physical capture conversion

**Files:** Modify platform `monitors.rs`, `capture.rs`, `overlay.rs` documentation, Mac `monitors.rs`, orchestrator `placement.rs`/`popup.rs`, shell `popup_shell.rs`; create Mac `geometry.rs`, portable `tests/macos_geometry.rs`; adjust Windows geometry documentation/tests without changing numeric behavior.

**Interfaces:** Keep existing Point/Rect as **desktop coordinates**: Windows physical virtual-screen pixels; Mac global Quartz points with top-left Y-down coordinates. Mac `MonitorInfo.scale_factor` for shared logical-popup-size conversion is `1.0`; keep actual display backing scale in native `MacDisplay { id, desktop_bounds, work_area, backing_scale }`. Work areas exclude menu bar/Dock. Define `MacGeometry::desktop_to_appkit(rect: Rect) -> Result<NativeRect, PlatformError>` and `capture_plan(rect: Rect) -> Result<CapturePlan, PlatformError>`, with exact float native bounds, display IDs, display-local pixel crops and an output size. Capture across displays composites at the largest intersecting backing scale and resamples lower-scale tiles; transparent/offscreen gaps are filled with opaque black, never unrelated screen content. CapturedImage always contains physical image pixels. A topology revision accompanies native plans and invalidates execution if displays changed. Ten-pixel overlay threshold means backing pixels on the starting display, not ten Quartz points.

- [ ] Write geometry tests: primary bounds `(0,0,1440,900)` at scale 2 yields a `(10,20,100,50)` desktop crop of 200x100 pixels with no double scaling; adjacent scale-1 display remains correctly positioned; above/left monitor negative origins and AppKit Y flip roundtrip; visible work area excludes menu bar/Dock; a two-display 1x/2x crop has deterministic tile offsets/output size; display removal invalidates the plan. Existing Windows popup placement/clamp values stay identical.

**Regression anchor — `retina_capture_scales_pixels_once`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(output_width, 200);
assert_eq!(output_height, 100);
assert_eq!(mac_shared_popup_scale, 1.0);
```

- [ ] Run `cargo test -p lingopop-platform --test macos_geometry --locked` and `cargo test -p lingopop-orchestrator placement --locked`; capture RED, implement conversions and route shell positioning through desktop geometry rather than always calling Tauri PhysicalPosition on Mac, then GREEN. Explicitly verify Mac floating-point-to-integer rounding with half-open rectangles.
- [ ] Review and locally commit: `fix(geometry): distinguish desktop coordinates from captured pixels`.

### Task 12: Exact Mac source targeting for capture, tray actions and Replace

**Files:** Modify Mac `foreground.rs`, `last_foreground.rs`, `keys.rs`, portable `foreground.rs`, `inject.rs` only as required, fake targeting tests and `tests/last_foreground.rs`; add `macos/target_registry.rs`, ignored `tests/macos_targeting.rs`.

**Interfaces:** Keep opaque `WindowHandle(u64)` and Windows HWND behavior. On Mac, handles identify generation-stamped entries retaining the captured application PID, process identity/start generation, focused AX window/element where available, and a bounded registry lifetime; an evicted/stale target fails safely. Add `ForegroundWindow::is_current_target(handle) -> bool` (Windows comparison uses existing native HWND, fake supports target changes). Mac restore reacquires/verifies the original target and focuses its element before paste; inject code checks this immediately before synthetic input. Permission failure is distinct from target loss. Same PID plus a different editor/window is not success; if an app cannot provide stable target identity, Replace is refused instead of guessing.

- [ ] Write tests: current target roundtrips, same-app different window is rejected, process exit/PID reuse is rejected, evicted registry handle is rejected, tray remembers the last external target and ignores own windows, permission revocation prevents paste, a foreground change after restore returns the existing uncertain/failed Replace outcome and never sends to the new app. Assert no selected text/window titles are logged.

**Regression anchor — `same_pid_different_window_never_receives_paste`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert!(!foreground.is_current_target(original_target));
assert_eq!(send_paste_count, 0);
```

- [ ] Run `cargo test -p lingopop-platform --test last_foreground --locked` plus inject/target unit tests and orchestrator Replace tests; capture RED, implement target registry/verification, then GREEN. Real multiwindow/fullscreen/Spaces tests remain separately recorded, not simulated as complete.
- [ ] Review and locally commit: `fix(macos): preserve the original source target for Replace`.

### Task 13: Generation-fenced queued presentation with acknowledgments

**Files:** Modify orchestrator `popup.rs`, `orchestrator.rs`, `actions.rs`, `fake.rs`, `session.rs`, `lib.rs` and affected existing tests; create portable `presentation.rs`, `tests/presentation_ordering.rs`; modify shell `popup_shell.rs` to implement queue protocol using current Windows native calls until Task 14.

**Interfaces:** Replace synchronous PopupView show-success semantics with `PopupView::submit(command: PresentationCommand) -> Result<(), PresentationQueueError>` and a completion receiver. Define `PresentationId { generation: u64, sequence: u64 }`, `PresentationKind::{Show { origin: Point, size_logical: (f64,f64), model: PopupViewModel }, Update { model: PopupViewModel }, Resize { origin: Point, size_logical: (f64,f64) }, Hide}`, `PresentationAck { id, result: Result<PresentationVisibility, PlatformError> }`, `PresentationVisibility::{Visible, Hidden}`. `PresentationQueueError::{Full, Closed}` distinguishes a rejected command from a closed worker; rejected commands never count as accepted/visible. Exact receiver method is `acknowledgments() -> crossbeam_channel::Receiver<PresentationAck>`; queue failure uses fixed text. Shared `PresentationFence` offers synchronous `advance(id)` and `apply_if_current(id, effect: impl FnOnce() -> Result<PresentationVisibility, PlatformError>) -> Option<PresentationAck>`. Its small lock is separate from the orchestrator session/commit lock. Submit from a locked session may update the fence/enqueue only; it never dispatches or waits for the main thread. A dedicated shell consumer owns no session/commit lock while posting work; no caller waits for that dispatch under a lock. Native effects run synchronously on main under the fence only, with native callbacks enqueueing async work; release fence before emitting ack/calling orchestrator. Do not acquire session while holding fence. Ack handling checks generation+sequence before visibility/dismiss state changes. Replace awaits its own Hide ack outside all locks, revalidates the session/target, then pastes; failed/timed-out/stale hide never pastes.

- [ ] Write deterministic `presentation_ordering` tests with explicit channels (no sleeps): enqueue old Show, supersede, execute old job -> no native effect; update/hide/new show preserves newest model; old ack cannot start dismissal; failed Show leaves Hidden; inline/reentrant dispatcher can enqueue a new operation without deadlock; duplicate ack is inert; Replace sends paste only after its own successful Hide ack and never after cancellation/supersession. Use injected ack deadline futures, not a 150ms wall-clock assertion. Verify persistence generation tests still pass.

**Regression anchor — `superseded_show_and_hide_ack_do_not_paste`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(stale_native_effect_count, 0);
assert_eq!(paste_count_before_current_hide_ack, 0);
assert_eq!(paste_count_after_stale_hide_ack, 0);
assert_eq!(paste_count_after_current_hide_ack, 1);
```

- [ ] Run `cargo test -p lingopop-orchestrator --test presentation_ordering --locked`; capture RED. Implement the portable protocol first, fake and Windows shell consumer second, then update all old PopupView tests to assert ack-confirmed visibility. Keep model readiness listener-before-ready/re-emission ordering. No MainThread dispatch under either session or fence lock; the actual native effect may hold the fence, but cannot reenter session synchronously.
- [ ] Run all orchestrator tests, shell popup tests and required Windows desktop smoke in CI. Expected: zero stale effects, hide-before-paste preserved, no enqueue success reported as already-on-screen success. The queue must be bounded/coalesced with explicit close/failure and a native-worker shutdown acknowledgment; choose capacity 32, coalesce Update/Resize per generation while never silently dropping Hide or reordering Show/Hide. Full returns an explicit error; Replace then refuses to paste. After this seam change, prepare_shutdown from Task 5 also waits for its final Hide ack. The orchestrator owns confirmed/pending visibility instead of asking the old synchronous is_visible() contract.
- [ ] Review and locally commit: `refactor(popup): acknowledge generation-fenced native presentation`.

### Task 14: Native AppKit popup and dismiss monitoring

**Files:** Modify Mac `popup_window.rs`, `dismiss` implementation currently in that file, Mac module exports; create `macos/presentation.rs`, `macos/dismiss.rs`, shell `macos_main_thread.rs`, portable worker tests and ignored `tests/macos_popup.rs`; wire `popup_shell.rs`, shell setup and native teardown.

**Interfaces:** Task 13 owns ordering/ack/fence; Task 11 owns desktop conversion. The Mac main-thread registry retains native popup panel/window state keyed by the shell's opaque popup handle. `MacPresentationConsumer::apply(kind: &PresentationKind) -> Result<PresentationVisibility, PlatformError>` lives in the shell and executes native show/order-without-activation, resize/update and hide only on main. Do not add Tauri/orchestrator dependencies to platform: shell translates PresentationKind into platform-local `MacWindowCommand` carrying native handle/rect; native methods implement existing NoActivateWindow synchronously **only on main**. Platform DismissWatcher methods remain non-blocking and publish geometry/commands; event tap/monitor callbacks emit Escape/outside-click events but never call orchestrator. Re-check accessibility state and report installation failure rather than pretending dismissal is installed.

- [ ] Write fake/main-dispatch tests: no caller lock spans dispatch; Show returns ack only after native completion; hide/worker shutdown removes monitors; stale native show is fenced; click inside is ignored, outside click and physical Escape deliver once; own injected input does not dismiss the new popup. Failed installation leaves an actionable state and no leaked monitor. Test listener registration precedes `popup_ready` and queued latest-model replay remains newest.

**Regression anchor — `main_thread_popup_show_preserves_source_focus`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(ack.result, Ok(PresentationVisibility::Visible));
assert_eq!(foreground_after_show, foreground_before_show);
assert_eq!(native_monitors_after_shutdown, 0);
```

- [ ] Run `cargo test -p lingopop-app popup --locked` and platform Mac presenter/dismiss unit tests; capture RED, verify required AppKit bindings/minimum OS, implement native panel configuration and consumer, then GREEN. Remove Mac Tauri show/hide activation fallbacks. Real installed-app acceptance covers retained source focus, mouse text selection/Copy, Settings focus, full screen/Spaces and repeated reopen.
- [ ] Review and locally commit: `feat(macos): present and dismiss the popup without stealing focus`.

### Task 15: Native screenshot capture and Vision OCR

**Files:** Modify Mac `capture.rs`, `ocr.rs`, platform target dependencies and native wiring; create portable `tests/macos_capture_contract.rs`, ignored `tests/macos_screen.rs`; retain shared `capture.rs`/`ocr.rs` APIs.

**Interfaces:** Implement existing `ScreenCapture::capture_region(Rect) -> Result<CapturedImage, PlatformError>` through Task 11 capture plans and `Ocr::recognize(&self, image: &CapturedImage) -> Result<OcrText, PlatformError>`. Preflight Task 8 ScreenRecording permission; native screen API choice must support macOS 13 or use a verified compatibility path. Vision uses accurate recognition and language correction, matching the Swift reference. Preserve reading order, trimmed blank detection, existing OCR image limits/encoding, and vision Retry provider eligibility. Captured rows are top-down RGBA with alpha 255, correct stride/color conversion, and no retained native buffer escapes its owning scope.

- [ ] Write contract tests: 100x50 desktop points on a 2x screen produces 200x100 pixels; clipping and multi-display composition follow CapturePlan exactly; denied/revoked permission returns PermissionDenied and never a successful empty image; padded row strides are copied correctly; empty OCR is a real empty result and vision retry retains the captured image. Verify rotated/orientation-aware OCR on a fixed native fixture without printing recognized user text.

**Regression anchor — `screen_permission_denial_is_not_empty_success`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(result, Err(PlatformError::PermissionDenied(PermissionKind::ScreenRecording)));
assert_eq!(native_capture_calls, 0);
```

- [ ] Run `cargo test -p lingopop-platform --test macos_capture_contract --locked` and orchestrator screen tests; capture RED, implement adapters after native API/dependency validation, then GREEN. Native screen tests require explicit real-screen authorization and must not capture the developer desktop in default CI.
- [ ] Review and locally commit: `feat(macos): capture screen regions and recognize text natively`.

### Task 16: Native Mac region overlay with serialized cancellation

**Files:** Modify Mac `overlay.rs`, shell `picker.rs` only for required lifecycle integration; create `macos/overlay_window.rs`, portable reducer/lifecycle tests and ignored `tests/macos_overlay.rs`; extend orchestrator screen tests.

**Interfaces:** Implement existing `RegionOverlay::pick(cancel: Receiver<()>) -> Result<Option<Rect>, PlatformError>`. Worker posts native overlay creation/input work to main and waits only off-main; cancellation/disconnect owns cleanup acknowledgment before another overlay starts. Shared ordered `fold_input_batch` remains the reducer, Task 11 maps desktop geometry and starting-display 10-physical-pixel threshold. On display topology change, cancel with cleanup instead of capturing a new unintended region. Overlay windows cannot become the remembered external source target.

- [ ] Write deterministic tests: Esc, dropped cancel sender, too-small and reverse drags return None/normalized rectangle appropriately; coalesced moves preserve Down/Up order; cancel/restart max native overlays is 1; capture loss may cancel as approved by Task 13's original Windows overlay spec; display removal cancels; cleanup precedes capture so overlay tint does not enter screenshots. Verify exact threshold at 1x/2x and negative origins.

**Regression anchor — `cancel_restart_waits_for_overlay_cleanup`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(first_result, Ok(None));
assert_eq!(max_native_overlays, 1);
assert!(first_cleanup_sequence < second_create_sequence);
```

- [ ] Run `cargo test -p lingopop-platform --test overlay_geometry --locked`, Mac overlay unit tests and orchestrator screen tests; capture RED, implement native overlay, then GREEN. Real multi-monitor mouse tests are ignored owner-run tests; preserve required Windows smoke and optional input-driven test classification.
- [ ] Review and locally commit: `feat(macos): select screen regions with a cancellable native overlay`.

### Task 17: Functional login items, repeat launch and native readiness gate

**Files:** Modify Mac `login_item.rs`, shell `lib.rs`/single-instance handling, Mac Tauri override; create platform `tests/macos_login.rs`, `scripts/check-macos-production.sh`; update `scripts/check-core-deps.sh` only if adding a separate production guard entry and manual checklist.

**Interfaces:** Implement existing LoginItem `set_enabled(enabled: bool, exe: &Path) -> Result<(), PlatformError>` for the installed Rust Mac bundle, not Swift. Resolve/validate the installed `.app` identity from its executable and report unbundled/relocated unsupported cases explicitly. Login enable/disable jobs retain existing latest-value fencing. Setup registers hotkeys only after detecting that frozen Swift is not running; coexistence detection is read-only and offers instructions instead of terminating or editing Swift. `check-macos-production.sh` fails if Mac native wiring selects plaintext/no-op/Unsupported dev adapters or release config misses the approved identity/minimum OS; it complements behavior tests rather than grepping as sole proof.

- [ ] Write fake/native tests: enable and disable target the Rust bundle; late queued enable cannot override newer disable; Swift running means no duplicate hotkey registration; second Rust launch opens Settings and registers no duplicate native services; relocate/upgrade failure shows a fixed actionable error without changing Swift login data. Readiness guard fails against an intentional fixture containing DevPlaintextProtector and passes real production wiring.

**Regression anchor — `swift_running_prevents_duplicate_global_hotkeys`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(registered_hotkeys, 0);
assert_eq!(swift_login_item_writes, 0);
```

- [ ] Run `cargo test -p lingopop-app task5 --locked`, platform login tests and readiness guard; capture RED, implement, then GREEN. Native login tests use a throwaway installed test bundle and explicit cleanup; default tests never change the user's actual login configuration. Run shared PR gate and installed-app acceptance for permission persistence across upgrade.
- [ ] Review and locally commit: `feat(macos): complete login and native production readiness`.
