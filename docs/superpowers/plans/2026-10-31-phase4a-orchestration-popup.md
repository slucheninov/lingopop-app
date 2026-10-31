# Phase 4a: Orchestration, popup and the Tauri shell — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** hotkey -> capture selection -> resolve target language -> `OperationRunner::run` -> non-activating popup with Copy / Replace / Retry-with-other-provider / Rewrite / Explain, plus the Screen Translate orchestration and OCR popup phase, all orchestration tested on fakes; a minimal Tauri v2 shell (tray with Quit, hidden popup window, hotkeys) running it on Windows.

**Architecture:** New OS-agnostic crate `crates/lingopop-orchestrator` (no `tauri`, no direct `windows` dependency; dependencies: `lingopop-core`, `lingopop-platform`, `tokio` [`rt`, `sync`, `time`, `macros`], `thiserror`, `serde`, `log`). It holds ALL flow logic over traits: `PlatformGate` (serialized blocking platform calls with timeouts), pure helpers (placement, view model, retry providers, error mapping) and `Orchestrator` (text operations, popup actions, Screen Translate). The UI side is abstracted by the trait `PopupView` (implemented by the Tauri shell; a fake records calls). The Tauri crate `app/src-tauri` (package `lingopop-app`, workspace member) is a thin shell: windows, tray, the hotkey service on the event-loop thread, Tauri commands forwarding popup actions to the orchestrator, Win32 show-without-activation. The frontend `app/src` (Svelte 5 + TS) renders the popup view model and sends actions.

**Tech Stack:** Rust stable (edition 2021), `tokio`, `serde`, `thiserror`, `log`, existing `lingopop-core` and `lingopop-platform`; `windows` 0.62.2 only inside `lingopop-platform` (hooks, `SetWindowPos`); `tauri` 2.12.5, `tauri-build` 2.7.4, `tauri-plugin-single-instance` 2.5.5; Svelte 5, Vite, Vitest, TypeScript (pins under Global Constraints).

**Spec:** `docs/superpowers/specs/2026-10-30-lingopop-app-design.md` (Russian): sections 3 (Popup, Screen Translate), 5 (`app/` Tauri: orchestration lives in Rust), 6 (hotkeys, capture, UIPI, clipboard safety, Popup, Replace, OCR), 9 (orchestration on fakes; c) Vitest) and the new section 12 "Фаза 4: оркестрация и popup". macOS reference app (read-only): `/Users/lucheninov/Works/slava/lingo-pop` v0.4.0 (`App/AppDelegate.swift`, `App/AppDelegate+PopupActions.swift`, `App/OperationPresenter.swift`, `App/TextReplacer.swift`, `App/ScreenTranslateFlow.swift`, `UI/PopupView.swift`, `UI/PopupWindow.swift`, `UI/PopupState.swift`, `UI/PopupSupport.swift`, `Core/PopupPlacement.swift`, `Core/PopupLayout.swift`).

## Roadmap

- **Phases 1-3 (done):** core, providers + runner, platform layer (PRs #1, #2; `main` at `0a0daef`).
- **Phase 4a (this plan):** `lingopop-orchestrator`, popup (view model, placement, actions, Screen Translate orchestration), platform additions (no-activate show, dismiss hooks, Phase 3 carry-overs), minimal Tauri shell, popup frontend, CI for the app.
- **Phase 4b:** settings window (Main / Providers / Shortcuts / Usage / Magic / About) including API-key entry and hotkey re-registration, history window, full tray menu (recent history, operations, target-language submenu, states idle / working / error), capture overlay window per monitor (feeds the `RegionPicker` from Task 5), localization import + check + Rust lookup + `appLanguage`, usage aggregation + `UsagePricing`, `tauri-plugin-autostart`.
- **Phase 5:** NSIS installer, `release.yml`, publish script, `RELEASING.md`, update checker, docs polish.

Notes: until 4b there is no key-entry UI, so in a 4a build only keyless Google Free works unless `secrets.dat` already has keys. The Screen Translate hotkey is not registered until the overlay exists in 4b (the orchestration is complete and tested on fakes).

Phase 4a runs on branch `phase4-app` (from `main` at `0a0daef`), one pull request. Windows code is compiled only by CI: run `gh pr checks` after each Windows-touching task; the DRAFT PR is opened at the end of Task 6. Pushing REQUIRES EXPLICIT GO-AHEAD FROM THE OWNER AT EXECUTION TIME.

## Global Constraints

- core and platform rules are unchanged (`forbid`/`deny(unsafe_code)`, `scripts/check-core-deps.sh`). Orchestrator: `#![forbid(unsafe_code)]`, no `tauri` dependency; Task 1 adds a guard to `scripts/check-core-deps.sh`: direct dependencies of `lingopop-orchestrator` must not match `tauri*`, `windows*`, `objc*`.
- Every platform call (clipboard, keys, foreground, monitors, capture, OCR) goes through `PlatformGate::run` => `spawn_blocking`, never on the UI/event-loop thread or an async worker. Only hotkey register/unregister run on the event-loop thread (`AppHandle::run_on_main_thread`).
- User text never appears in logs, error `Display`/`Debug`, or events other than the popup view model sent to the popup window.
- The user's clipboard is never lost (platform guarantees); the orchestrator never writes the clipboard except Copy (normal `write_text`) and autoCopyOCR, both through the gate.
- No focus stealing: the popup is never activated; it is shown only via the platform `NoActivateWindow::show_at` (Task 6), never Tauri `show()` / `set_focus()`.
- Rust toolchain: workspace `rust-version` 1.89 stays; `app/src-tauri` declares `rust-version = "1.90"` (tauri 2.12.5 MSRV).
- Versions pinned: tauri 2.12.5 (features `tray-icon`, `image-png`), tauri-build 2.7.4, tauri-plugin-single-instance 2.5.5; npm `@tauri-apps/api` 2.12.4, `@tauri-apps/cli` 2.12.4, svelte 5.57.3, vite 8.3.5, `@sveltejs/vite-plugin-svelte` 7.3.1, vitest 5.0.5, typescript ^6 (NOT 7: svelte-check 4.7.7 peer is ^5 || ^6), svelte-check 4.7.7, `@testing-library/svelte` 5.4.2, jsdom 30.1.3; Node 22 in CI. Tauri 3.0 alphas are excluded (dependabot ignore `tauri*` >=3 and `@tauri-apps/*` >=3).
- Task gate: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`. From Task 7 on, first `npm ci --prefix app && npm run build --prefix app` (tauri-codegen panics if `frontendDist` is missing; icons `app/src-tauri/icons/icon.ico` + `icon.png` are committed), plus `npm test --prefix app` and `npm run check --prefix app` from Task 8.
- Strings: popup and error texts are catalog keys = the mac English source strings (`UserMessage { key, args }` from core, `t(key, args)` in the frontend). Phase 4a ships `t()` as English pass-through with `%@` / `%lld` substitution; Phase 4b plugs the catalog. New Windows-only keys are listed in Task 2.
- One PR, conventional commits, no Co-Authored-By / "Generated with" footers; each task appends its modules to the AGENTS.md Layout in the same commit; `cargo test` takes at most one filter.

## Review Focus

Five failure modes, most likely first; each has pinning tests in the owning task (marked `(RF n)`).

1. **Focus stolen from the user's app:** show via Tauri `show()`, a WebView2 first navigation stealing focus (the popup window is created hidden at startup and navigated before any hotkey), a click on a popup button activating it (`WS_EX_NOACTIVATE` via `focusable(false)`; unverified on real Windows -> manual checklist). Tests: Task 6 `windows_smoke::show_at_does_not_change_foreground` (ignored, interactive desktop), Task 3 `orchestrator_never_calls_focus_on_popup` (the fake `PopupView` has no focus method; assert only `show_at` / `update` / `hide` are used).
2. **Two flows touching the clipboard at once / a hung clipboard owner (Excel delayed rendering) freezing everything:** the gate serializes and times out; a new hotkey cancels the previous run's async part but its blocking capture still finishes under the gate. Tests: Task 1 `gate_serializes_jobs`, `gate_times_out_long_job_and_stays_locked_until_it_finishes`; Task 3 `second_trigger_cancels_first_and_only_second_result_is_shown`, `trigger_while_capture_hangs_reports_clipboard_busy`.
3. **Esc / click-outside with a never-activated popup:** the popup receives no keyboard input, so Esc is caught by the platform `DismissWatcher` (low-level keyboard + mouse hooks, Esc swallowed only while the popup is visible, a click inside the popup rect ignored). Tests: Task 6 `dismiss_click_inside_rect_is_ignored`, `dismiss_watcher_stops_on_hide`; Task 4 `escape_closes_popup_and_clears_session`.
4. **Replace into the wrong window or an elevated one:** target = `ForegroundWindow::current()` taken BEFORE capture; popup hidden before paste; `InputBlocked` -> UIPI message; `ForegroundLost` -> "text is on your clipboard, paste with Ctrl+V" message. Tests: Task 4 `replace_targets_window_captured_before_capture`, `replace_elevated_shows_uipi_message`, `replace_foreground_lost_shows_manual_paste_message`.
5. **Popup placement** off-screen on a secondary monitor with negative coordinates / mixed DPI / near the bottom edge (flip above the cursor). Tests: Task 2 `place_below_right_of_cursor`, `place_flips_above_when_no_room_below`, `place_uses_side_with_more_room_when_neither_fits`, `place_on_left_monitor_negative_coords`, `place_mixed_dpi_scales_by_monitor_under_cursor`.

## Dependency decisions (verified 2026-10-31)

- **tauri 2.12.5:** `WebviewWindowBuilder::focused(false)` and `focusable(false)` exist (`focusable` since 2.8.0, PR tauri#13564). tao 0.37 maps `focusable(false)` to `WS_EX_NOACTIVATE`, but only the FIRST show uses `SW_SHOWNOACTIVATE` (tao clears `MARKER_DONT_FOCUS` after creation); later `show()` = `SW_SHOW` and may activate. Therefore we never call `show()`; we call Win32 `SetWindowPos(HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW)` on `window.hwnd()` and `ShowWindow(SW_HIDE)` to hide (platform crate, Task 6).
- A never-activated window gets no keyboard input -> Esc needs `WH_KEYBOARD_LL` (Task 6). A click outside needs `WH_MOUSE_LL` (no focus-lost event exists without activation).
- **`global-hotkey` 0.9:** our `GlobalHotkeyService` (Phase 3) is `!Send`; it is created in Tauri `setup` (runs on the event-loop thread; the official plugin does the same, `tauri-plugin-global-shortcut` 2.4.1 `lib.rs:486`) and kept in a `thread_local!` on that thread; later (4b) re-registration goes through `AppHandle::run_on_main_thread`. Its `events()` crossbeam receiver is drained by a dedicated std thread that calls `tauri::async_runtime::spawn(orchestrator.trigger(op))`. `tauri-plugin-global-shortcut` is NOT used (it pins global-hotkey 0.8 with a separate global channel).
- **Tray-only app:** `app.windows = []` in `tauri.conf.json`; the popup window is built in `setup`; `RunEvent::ExitRequested { code: None, api }` -> `api.prevent_exit()`; tray Quit calls `app.exit(0)`.
- `tauri-codegen` needs `frontendDist` (`../dist`) to exist at compile time and the icons; hence the npm build before cargo in CI and locally (documented in AGENTS.md "Build / test").
- `tauri::test` (feature `test`, unstable) is NOT used: the orchestration is tested in the orchestrator crate without Tauri.
- `tauri-plugin-single-instance` 2.5.5 in Task 7 (a second launch just exits; 4b may open Settings).

## File Structure

```
crates/lingopop-orchestrator/
  Cargo.toml                         (Task 1)
  src/lib.rs                         lints, module decls, re-exports                          (Tasks 1-5)
  src/gate.rs                        GateTimeouts, GateError, PlatformGate                    (Task 1)
  src/placement.rs                   popup_origin                                             (Task 2)
  src/view_model.rs                  PopupViewModel, PopupPhase, ResultView, ErrorView ...    (Task 2)
  src/messages.rs                    platform_message, gate_message, error keys               (Task 2)
  src/providers.rs                   retry_providers, vision_providers, ocr_translate_providers (Task 2)
  src/popup.rs                       PopupView trait                                          (Task 3)
  src/session.rs                     Session, generation / abort handling                     (Task 3)
  src/orchestrator.rs                Deps, ConfigSource, Persist, Orchestrator::trigger       (Task 3)
  src/actions.rs                     copy / replace / retry / rewrite / explain / ... close   (Task 4)
  src/screen.rs                      RegionPicker, screen_translate, translate_ocr, vision retry (Task 5)
  src/fake.rs                        FakePopup, FakeConfig, FakePersist, FakePicker           (Tasks 3-5)
  tests/{gate,placement,view_model,orchestrator_text,orchestrator_actions,orchestrator_screen}.rs
crates/lingopop-platform/ (Task 6)
  src/{popup_window.rs, dismiss.rs}, src/windows/{popup_window.rs, dismiss.rs}, src/macos/..., fake additions,
  image downscale in src/capture.rs
app/
  package.json, package-lock.json, vite.config.ts, tsconfig.json, svelte.config.js, index.html
  src/main.ts, src/popup/*.svelte, src/lib/{i18n.ts, actionRow.ts, ipc.ts, icons.ts}, src/**/*.test.ts
  src-tauri/Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json, icons/
  src-tauri/src/{main.rs, lib.rs, popup_shell.rs, hotkeys.rs, tray.rs, commands.rs, logging.rs}
docs/testing/manual-checklist.md     (Task 9)
.github/workflows/ci.yml, .github/dependabot.yml, AGENTS.md, .gitignore (app/dist, app/node_modules)
```

---

### Task 1: `lingopop-orchestrator` crate scaffold and `PlatformGate`

**Files:**
- Create: `crates/lingopop-orchestrator/{Cargo.toml, src/lib.rs, src/gate.rs, tests/gate.rs}`
- Modify: `scripts/check-core-deps.sh` (orchestrator guard), `Cargo.lock`, `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: `lingopop_platform::PlatformError`.
- Produces (`lingopop_orchestrator`):
  - `pub struct GateTimeouts { pub acquire: Duration, pub run: Duration }`; `Default` = acquire 2 s, run 3 s.
  - `#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)] pub enum GateError { Busy, TimedOut, Panicked, Platform(PlatformError) }`. `Display` carries no payload: `"platform is busy"`, `"platform call timed out"`, `"platform call panicked"`, `"{0}"` for `Platform`.
  - `#[derive(Clone)] pub struct PlatformGate` with `pub fn new(timeouts: GateTimeouts) -> Self` and `pub async fn run<T, F>(&self, job: F) -> Result<T, GateError> where T: Send + 'static, F: FnOnce() -> Result<T, PlatformError> + Send + 'static`.
  - Semantics: acquire an OWNED `tokio::sync::Mutex` guard within `acquire` (else `Busy`); move the guard INTO the `spawn_blocking` closure so the lock is held until the blocking job really returns, even after the caller timed out; await the `JoinHandle` within `run` (else `TimedOut`); a `JoinError` panic -> `Panicked` (the lock is released because the guard dropped with the closure).

- [ ] **Step 1: Write the failing tests**

`tests/gate.rs` (`#[tokio::test(flavor = "multi_thread")]`, short real durations, e.g. acquire 50 ms / run 100 ms; jobs blocked on std channels; no sleeps longer than needed):
```rust
// gate_runs_job_off_the_calling_thread          (thread id inside the job != the test thread id)
// gate_returns_job_value
// gate_maps_platform_error                      (job Err(ClipboardBusy) -> GateError::Platform(ClipboardBusy))
// gate_serializes_jobs (RF 2)                   (two concurrent run()s; a shared counter never exceeds 1 inside jobs)
// gate_busy_when_lock_held_past_acquire_timeout (job A blocked on a channel; run B -> Busy after ~50 ms; then release A)
// gate_times_out_long_job_and_stays_locked_until_it_finishes (RF 2)
//     (job A blocks past run 100 ms -> TimedOut; run B immediately -> Busy; release A; run B again -> Ok)
// gate_panic_is_reported_and_releases_lock      (job panics -> Panicked; the next run() succeeds)
// gate_error_display_has_no_payload             (exact Display strings above; Platform(..) shows the inner Display)
```

- [ ] **Step 2: Run to verify failure**

Run: hand-write `Cargo.toml` (Step 3) and an empty `src/lib.rs`, then `cargo test -p lingopop-orchestrator --test gate`
Expected: FAIL to compile (the module does not exist yet).

- [ ] **Step 3: Implement**

- `Cargo.toml`: workspace package fields, description "Flow logic for LingoPop: capture, run, popup actions"; dependencies `lingopop-core`, `lingopop-platform` (paths), `tokio = { version = "1", features = ["rt", "sync", "time", "macros"] }`, `thiserror = "2"`, `serde = { version = "1", features = ["derive"] }`, `log = "0.4"`; dev-dependencies `tokio` with `rt-multi-thread`. Run `cargo build -p lingopop-orchestrator` without `--locked` to update `Cargo.lock`.
- `lib.rs`: `#![forbid(unsafe_code)]`, module decls, re-exports.
- `gate.rs`: as described under Produces.
- `scripts/check-core-deps.sh`: new part: direct dependencies of `lingopop-orchestrator` (`cargo metadata`) must not match `tauri*`, `windows*`, `objc*`; `lingopop-core` and `lingopop-platform` must not depend on the orchestrator.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-orchestrator --test gate`
Expected: PASS (8 tests).

- [ ] **Step 5: Prove the guard**

Run: `cargo add -p lingopop-orchestrator tauri@2.12.5 && bash scripts/check-core-deps.sh; echo "exit=$?"`. Expected: error naming `tauri`, `exit=1`. Undo with `cargo remove -p lingopop-orchestrator tauri`, script again: `exit=0`.

- [ ] **Step 6: Documentation, gate and commit**

`AGENTS.md`: Layout bullets for the crate manifest, `lib.rs`, `gate.rs`. Run the task gate.
```bash
git add -A && git commit -m "feat(orchestrator): add lingopop-orchestrator crate with PlatformGate"
```

---

### Task 2: Pure helpers: placement, view model, retry providers, messages

**Files:**
- Create: `crates/lingopop-orchestrator/src/{placement.rs, view_model.rs, providers.rs, messages.rs}`, `tests/{placement.rs, view_model.rs}`
- Modify: `src/lib.rs`, `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: platform `Point`, `Size`, `Rect`, `MonitorInfo`, `monitor_at`, `physical_size`, `clamp_to_work_area`, `PlatformError`; core `ActionRowMode`, `LONG_TEXT_CHARACTER_THRESHOLD`, `AIOperation`, `ProviderType`, `languages`, `FallbackInfo`, `UserMessage`, `TranslationError`, `ConfigSnapshot`.
- Produces:
  - `placement.rs`: `pub fn popup_origin(size_logical: (f64, f64), cursor: Point, monitors: &[MonitorInfo]) -> Point` (physical px). Monitor = `monitor_at(cursor)`; scale its factor via `physical_size`; offset 12 logical px (scaled) right of and below the cursor; if it does not fit below within the work area inset by 8 (scaled), flip above (`cursor.y - offset - h`); if neither fits use the side with more room; then `clamp_to_work_area` with the inset. Port of `Core/PopupPlacement.swift:10-28` with the y axis inverted (Windows y grows downward). No monitors -> cursor + offset.
  - `view_model.rs` (serde, camelCase, sent to the frontend as JSON): `#[serde(tag = "phase")] pub enum PopupPhase { Loading { operation: AIOperation, preview: String }, Ocr { text: String, translate_providers: Vec<ProviderType>, active_translate_provider: Option<ProviderType> }, Result(ResultView), Error(ErrorView) }`; `ResultView { text, operation, provider, provider_name, model, target_language, via_fallback, fallback: Option<FallbackView { primary_name, message: UserMessageView, details: Vec<UserMessageView> }>, truncated, show_replace (operation != ScreenTranslate), show_explain (operation != Explain), quick_languages: Vec<String> (only for Translate: `languages::quick_list` with the current target inserted first if missing), action_row: ActionRowView { preferred: ActionRowMode, candidates: Vec<ActionRowMode> } }`; `ErrorView { message: UserMessageView, details: Vec<UserMessageView> }`; `PopupViewModel { phase: PopupPhase, retry_providers: Vec<RetryProvider { provider, name }> }`; `UserMessageView { key, args }`.
  - Preview = first 120 chars of the original (chars, not bytes). Action row: `button_count` = Copy + Rewrite + (Replace if shown) + (Explain if shown) + (Retry if any) for results; OCR = 2 + (Retry ? 1 : 0); `preferred = ActionRowMode::preferred(chars, LONG_TEXT_CHARACTER_THRESHOLD, button_count)`; `candidates = preferred.candidates()`.
  - Serialization of `ActionRowMode` / `AIOperation` / `ProviderType`: verify at implementation whether the core types already derive `Serialize`; if not, the view model stores raw strings (`AIOperation::raw_value()`, the `ProviderType` raw value) and the mode as `"full" | "mixed" | "compact"`, via wrapper or `#[serde(with)]`.
  - `providers.rs`: `pub fn retry_providers(excluding: ProviderType, operation: AIOperation, configs: &ConfigSnapshot, registry_has: impl Fn(ProviderType) -> bool) -> Vec<ProviderType>` in `ProviderType` declaration order: not excluded, registered, `capabilities().supported_operations` contains the operation, key not required or present, and for ScreenTranslate `supports_vision`. Also `vision_providers(excluding: Option<ProviderType>, configs: &ConfigSnapshot) -> Vec<ProviderType>` and `ocr_translate_providers(configs: &ConfigSnapshot) -> Vec<ProviderType>` (ready for ScreenTranslate). Port of `App/OperationPresenter.swift:63-92`. Verify the exact capability field names in `provider.rs` at implementation.
  - `messages.rs`: `pub fn platform_message(e: &PlatformError) -> UserMessageView`, `pub fn gate_message(e: &GateError) -> UserMessageView`, plus constructors for nothing selected, max input, missing key, no text found, selection too small. `TranslationError` -> `user_summary()` + `user_detail()` (core). Keys (mac English source strings unless marked NEW Windows-only):
    - nothing selected: `Nothing selected. Select some text first, then press the hotkey.`
    - `ClipboardBusy` / `GateError::Busy` / `TimedOut`: `Clipboard didn't update in time. Try again.`
    - max input: `Text is %@ characters, max is %@. Adjust in Settings → Main.`
    - missing key: `API key missing for %@. Open Settings → Providers to add one.`
    - `InputBlocked` NEW: `LingoPop can't reach an app running as administrator. Run LingoPop as administrator, or copy and paste the text manually.`
    - `ForegroundLost` NEW: `Couldn't switch back to the original app, so nothing was replaced. The text is on your clipboard; paste it manually with Ctrl+V.`
    - `OcrUnavailable` NEW: `No OCR language is installed. Add a language with Optical character recognition in Settings → Time & language → Language & region, or use Retry to send the image to AI.`
    - no text found: `No text found in the selected area. Try a larger selection or use Retry to send the image to AI.` when vision providers exist, else `No text found in the selected area. Try a larger selection.`
    - selection too small: `Selection is too small. Try selecting a larger area.`
    - any other `PlatformError` NEW: `Something went wrong on this PC (%@). Try again.` with the error `Display` as the arg (no payload by construction).

- [ ] **Step 1: Write the failing tests**

`tests/placement.rs` (monitors built by hand: 1920x1080 at scale 1.0, a left monitor at x = -1920, a 4K monitor at scale 2.0 to the right):
```rust
// place_below_right_of_cursor                       (cursor (100,100), size 300x200 -> (112,112) on scale 1.0)
// place_flips_above_when_no_room_below              (cursor near the bottom: y == cursor.y - 12 - 200)
// place_uses_side_with_more_room_when_neither_fits  (tall popup on a short monitor: picks the larger side, then clamped)
// place_on_left_monitor_negative_coords             (cursor x = -500: result x >= -1920 + 8, never positive)
// place_mixed_dpi_scales_by_monitor_under_cursor    (same logical size -> twice the physical size on the 2.0 monitor, offset 24)
// place_without_monitors_returns_offset_cursor
```
`tests/view_model.rs`:
```rust
// result_view_hides_replace_for_screen_translate
// result_view_hides_explain_for_explain
// preview_is_first_120_chars                         (multi-byte input: 120 chars, not bytes)
// action_row_counts_buttons_like_mac                 (results and OCR, with and without Retry)
// quick_languages_insert_current_target_first        (Translate only; empty for other operations)
// view_model_json_shape                              (snapshot of the exact JSON for one result and one error)
```
`providers.rs` / `messages.rs` unit tests:
```rust
// retry_excludes_current_and_keyless_providers
// retry_for_screen_translate_needs_vision
// google_free_needs_no_key
// one test per message mapping (platform_message_input_blocked, ..._foreground_lost, ..._ocr_unavailable, ..._other, gate_message_busy_and_timed_out)
// messages_never_contain_selected_text               (arg lists only carry names, counts and error Display)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-orchestrator`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

Signatures above; pinned values: offset 12, inset 8, preview 120, `LONG_TEXT_CHARACTER_THRESHOLD`. JSON is camelCase, enums as raw strings.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-orchestrator`
Expected: PASS.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md`: Layout bullets for the four modules. Run the task gate.
```bash
git add -A && git commit -m "feat(orchestrator): add popup placement, view model, retry providers and message mapping"
```

---

### Task 3: `Orchestrator` text-operation flow

**Files:**
- Create: `crates/lingopop-orchestrator/src/{popup.rs, session.rs, orchestrator.rs, fake.rs}`, `tests/orchestrator_text.rs`
- Modify: `src/lib.rs`, `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: Tasks 1-2; platform `Platform` bundle, `capture_selection`, `CaptureOptions`, fakes; core `OperationRunner`, `RunRequest`, `ConfigSnapshot`, `Settings`, `detector::detect`, `target_language_for`, `HistoryEntry`, `UsageRecord`, `Clock`.
- Produces:
  - `pub trait PopupView: Send + Sync { fn show_at(&self, origin: Point, model: &PopupViewModel); fn update(&self, model: &PopupViewModel); fn hide(&self); fn is_visible(&self) -> bool; }` (the shell measures and resizes itself; `show_at` is called only when not visible, like mac `show` not repositioning a visible popup).
  - `pub trait ConfigSource: Send + Sync { fn settings(&self) -> Settings; fn snapshot(&self) -> ConfigSnapshot; }`; `pub trait Persist: Send + Sync { fn add_history(&self, entry: HistoryEntry, max_count: u32); fn append_usage(&self, record: UsageRecord); }` (the shell implements it over `HistoryStore` / `UsageStore` behind a `Mutex`, logs store errors, never fails the flow).
  - `pub struct Deps { platform: Platform, runner: Arc<OperationRunner>, registry_has: Arc<dyn Fn(ProviderType) -> bool + Send + Sync>, config: Arc<dyn ConfigSource>, stores: Arc<dyn Persist>, popup: Arc<dyn PopupView>, gate: PlatformGate, clock: Arc<dyn Clock>, log: Option<Arc<LogFn>> }`.
  - `#[derive(Clone)] pub struct Orchestrator` (`Arc` inside) with `pub fn new(deps: Deps) -> Self` and `pub async fn trigger(&self, operation: AIOperation)`.
  - Fakes in `fake.rs` (always compiled): `FakePopup` (records `Show/Update/Hide` calls with models, toggles visibility), `FakeConfig`, `FakePersist`.
- Flow (port of `AppDelegate.handleHotkey` / `runOperation`):
  1. ScreenTranslate -> `screen_translate()` (Task 5). Otherwise ONE gate job returning all three: `cursor = monitors.cursor_position()`, the monitor list, `target = foreground.current()`, then `capture_selection(..)` with `CaptureOptions { log, ..Default }`, so nothing else touches the clipboard in between.
  2. `Ok(None)` -> error popup "Nothing selected"; capture error -> mapped error popup at the cursor; `max_input_chars > 0 && chars > limit` -> error.
  3. `detected = detector::detect` only for Translate; target language = `target_language_for`; the session is stored (operation, text, detected, target language, cursor, target window, primary = `settings.active_provider`, image None); popup `show_at(popup_origin(..), Loading)` using the monitor list fetched in the same gate job.
  4. `runner.run(RunRequest { allow_fallback: true, .. }, &snapshot)`; success -> persist history (all operations, `max_history_count`) and usage (if `Some`), update Result with retry providers excluding `result.provider`; error -> update Error with retry providers excluding the primary; a missing API key error uses the "API key missing for %@" key with `display_name()`.
- Cancellation (mac `startOperationTask`): each trigger/action takes a new generation number and aborts the previous async task (`tokio::task::AbortHandle` stored in the session mutex); a superseded run never touches the popup (the generation is checked before every popup call); the blocking gate job of a superseded run still completes.

- [ ] **Step 1: Write the failing tests**

`tests/orchestrator_text.rs` (platform fakes from `lingopop-platform::fake`, `FakeTransport` + `ProviderRegistry` from core, `FakePopup`):
```rust
// translate_happy_path_shows_loading_then_result
// nothing_selected_shows_error_and_no_request
// max_input_exceeded_shows_error_and_no_request
// max_input_zero_means_no_limit
// translate_uses_detected_language_pair
// grammar_fix_uses_target_language_without_detection
// success_persists_history_and_usage
// error_shows_retry_providers_excluding_primary
// missing_key_message_names_provider
// second_trigger_cancels_first_and_only_second_result_is_shown (RF 2)
// trigger_while_capture_hangs_reports_clipboard_busy (RF 2)       (first capture blocked on a channel past the acquire timeout)
// platform_calls_run_off_the_async_thread
// orchestrator_never_calls_focus_on_popup (RF 1)                   (only show_at / update / hide appear in the recorded calls)
// capture_log_hook_receives_no_user_text
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-orchestrator --test orchestrator_text`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

Types and flow above; `session.rs` holds the generation counter and `AbortHandle` behind one `std::sync::Mutex` (never held across `.await`).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-orchestrator --test orchestrator_text`
Expected: PASS (14 tests).

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md`: Layout bullets for `popup.rs`, `session.rs`, `orchestrator.rs`, `fake.rs`. Run the task gate.
```bash
git add -A && git commit -m "feat(orchestrator): add the text-operation flow with cancellation and popup view trait"
```

---

### Task 4: Popup actions

**Files:**
- Create: `crates/lingopop-orchestrator/src/actions.rs`, `tests/orchestrator_actions.rs`
- Modify: `src/lib.rs`, `src/orchestrator.rs` (shared run helper), `AGENTS.md`

**Interfaces:**
- Consumes: Task 3 types; platform `replace_selection`, `ReplaceOptions`, `ReplaceOutcome`, `Clipboard::write_text`.
- Produces on `Orchestrator`: `async fn copy(&self)`, `async fn replace(&self)`, `async fn retry(&self, provider: ProviderType)`, `async fn rewrite(&self)`, `async fn explain(&self)`, `async fn retranslate(&self, target_language: String)`, `async fn translate_ocr(&self, provider: ProviderType)` (Task 5 implements), `async fn close(&self)`, `async fn escape(&self)` (same as `close`).
- Facts from mac:
  - **Copy** writes the shown text via gate `clipboard.write_text` (a normal entry, the user keeps it); the popup stays open.
  - **Replace:** hide the popup FIRST, then gate `replace_selection(fg, clip, keys, sleeper, session.target_window, text, ReplaceOptions { log, .. })`; `Pasted` -> done (session cleared); `ForegroundLost` -> error popup at the session cursor with the Ctrl+V message; `Err(InputBlocked)` -> UIPI message; no target window -> the `ForegroundLost` message.
  - **Rewrite / Explain** run on the DISPLAYED result text with `active_provider`, `allow_fallback: true`, `detected_source: None`, target = session target language, and overwrite the session text (a later Retry reruns that operation on that text).
  - **Retry:** last operation + session text + session target, `primary: provider`, `allow_fallback: false`.
  - **Retranslate:** Translate on the session text to the chosen language, active provider, fallback on.
  - **Close / Esc:** hide, clear the session text and image.
  - All actions bump the generation (cancel the in-flight run).

- [ ] **Step 1: Write the failing tests**

`tests/orchestrator_actions.rs`:
```rust
// copy_writes_clipboard_and_keeps_popup               (FakeClipboard.text() == shown text; no Hide call)
// replace_hides_popup_before_paste                    (recorded order: Hide, then the first paste event)
// replace_targets_window_captured_before_capture (RF 4) (foreground changes after capture; fg.restored() == [window taken before capture])
// replace_elevated_shows_uipi_message (RF 4)          (InputBlocked -> Error popup with the NEW InputBlocked key)
// replace_foreground_lost_shows_manual_paste_message (RF 4)
// rewrite_runs_on_result_text                         (request body contains the displayed text, not the original)
// explain_runs_on_result_text
// retry_uses_chosen_provider_without_fallback         (primary = chosen, allow_fallback false)
// retranslate_runs_translate_on_original_text
// escape_closes_popup_and_clears_session (RF 3)
// action_during_run_cancels_it
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-orchestrator --test orchestrator_actions`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

Signatures and rules above; no code bodies pinned beyond them.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-orchestrator --test orchestrator_actions`
Expected: PASS (11 tests).

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md`: Layout bullet for `actions.rs`. Run the task gate.
```bash
git add -A && git commit -m "feat(orchestrator): add popup actions copy, replace, retry, rewrite, explain, retranslate and close"
```

---

### Task 5: Screen Translate orchestration

**Files:**
- Create: `crates/lingopop-orchestrator/src/screen.rs`, `tests/orchestrator_screen.rs`
- Modify: `src/lib.rs`, `src/orchestrator.rs`, `src/fake.rs` (`FakePicker`), `src/actions.rs` (`translate_ocr`), `AGENTS.md`

**Interfaces:**
- Consumes: Tasks 3-4; platform `Monitors::monitors`, `validate_region`, `ScreenCapture::capture_region`, `Ocr::recognize`, `CapturedImage::to_png`; core `resolve_target_language`, `OperationRunner::run_vision`.
- Produces: `#[async_trait::async_trait] pub trait RegionPicker: Send + Sync { async fn pick(&self) -> Result<Option<Rect>, PlatformError>; }` (the `async_trait` crate is used, as in core's `Transport`; `None` = cancelled, nothing shown); `Deps` gains `picker: Arc<dyn RegionPicker>`.
- Flow (port of `ScreenTranslateFlow.start`):
  1. Target window = `foreground.current()` (gate); pick the region.
  2. Gate: `monitors()` + `validate_region` (use the CLIPPED rect, Phase 3 carry-over) + `capture_region`; anchor = (clipped.right, clipped.bottom); show Loading(ScreenTranslate, "").
  3. Gate: `ocr.recognize(&image)`. Success non-empty -> detect + resolve the target on the OCR text (`resolve_target_language(settings, detect(ocr))`); autoCopyOCR -> gate `write_text(ocr)`; update the Ocr phase with `ocr_translate_providers` + the active provider (if ready) and the vision retry providers. Empty text / `OcrUnavailable` -> Error with vision retry providers and the matching message.
  4. `translate_ocr(provider)` -> `run(ScreenTranslate, ocr text, allow_fallback: provider == active)`, result with Replace hidden, autoCopyOCR copies the translated text.
  5. Vision retry from the Ocr/Error phase -> `image.to_png()` -> `runner.run_vision(provider, png, original = ocr text or "", target)`, no fallback; error -> retry list `vision_providers(excluding provider)`.
  6. The image is dropped on close and on any non-ScreenTranslate run; a history entry is written for every success.

- [ ] **Step 1: Write the failing tests**

`tests/orchestrator_screen.rs`:
```rust
// cancelled_pick_shows_nothing                         (no popup calls, no platform capture)
// capture_uses_clipped_region                          (region partly off-screen: capture rect == clipped)
// ocr_success_shows_ocr_phase_and_auto_copies
// auto_copy_off_does_not_touch_clipboard
// ocr_empty_offers_vision_retry
// ocr_unavailable_message                              (NEW OcrUnavailable key)
// translate_ocr_hides_replace_and_auto_copies_result
// vision_retry_sends_png_and_never_falls_back
// ocr_runs_off_the_async_thread
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-orchestrator --test orchestrator_screen`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

Add `async-trait` to the orchestrator dependencies; flow above.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-orchestrator --test orchestrator_screen`
Expected: PASS (9 tests).

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md`: Layout bullet for `screen.rs`. Run the task gate.
```bash
git add -A && git commit -m "feat(orchestrator): add Screen Translate orchestration with OCR phase and vision retry"
```

---

### Task 6: Platform additions (Windows compiled by CI)

**Files:**
- Create: `crates/lingopop-platform/src/{popup_window.rs, dismiss.rs, windows/popup_window.rs, windows/dismiss.rs, macos/popup_window.rs, fake/popup_window.rs, fake/dismiss.rs}`
- Modify: `crates/lingopop-platform/src/{lib.rs, capture.rs, native.rs, windows/{mod,clipboard,ocr,monitors}.rs, fake/mod.rs}`, `crates/lingopop-platform/Cargo.toml` (windows features as the compiler demands), `crates/lingopop-platform/tests/windows_smoke.rs`, `crates/lingopop-orchestrator/src/{orchestrator.rs, actions.rs}`, `AGENTS.md`

**Interfaces:**
- Consumes: Phase 3 `WindowHandle`, `Rect`, `Point`, `PlatformError`, `Platform`.
- Produces:
  - (a) `popup_window.rs`: `pub trait NoActivateWindow: Send + Sync { fn show_at(&self, window: WindowHandle, rect: Rect) -> Result<(), PlatformError>; fn hide(&self, window: WindowHandle) -> Result<(), PlatformError>; }`. Windows: `SetWindowPos(HWND_TOPMOST, .., SWP_NOACTIVATE | SWP_SHOWWINDOW)` / `ShowWindow(SW_HIDE)`. macOS dev-only: `NSWindow orderFrontRegardless` + `setFrame` via objc2 if it is cheap, else `Unsupported` and the shell falls back to the Tauri show (choose at implementation and state it in AGENTS.md). Fake records calls.
  - (b) `dismiss.rs`: `pub enum DismissEvent { Escape, ClickOutside }`; `pub trait DismissWatcher: Send + Sync { fn start(&self, popup_rect: Rect) -> Result<(), PlatformError>; fn update_rect(&self, popup_rect: Rect); fn stop(&self); fn events(&self) -> crossbeam_channel::Receiver<DismissEvent>; }`; pure `pub fn classify_click(point: Point, popup: Rect) -> Option<DismissEvent>` (outside -> `ClickOutside`). Windows: a dedicated thread with its own message loop installing `WH_KEYBOARD_LL` (VK_ESCAPE key-down -> `Escape`, return 1 to swallow, only while started) and `WH_MOUSE_LL` (`WM_LBUTTONDOWN` / `WM_RBUTTONDOWN` / `WM_MBUTTONDOWN` outside the rect -> `ClickOutside`, never swallowed); hooks removed on `stop` (`UnhookWindowsHookEx`, `PostThreadMessageW(WM_QUIT)`). macOS dev: `Unsupported` / no-op.
  - (c) Phase 3 carry-overs:
    - The Windows clipboard snapshot logs the ids of enumerated-but-unsaved formats through the `log` hook (`"clipboard: skipped format id {id}"`, ids only): add `WindowsClipboard::with_log(log)` (verify at implementation the cleanest seam for `snapshot` to hold a log hook).
    - Pure `pub fn downscale_to_fit(image: &CapturedImage, max_dim: u32) -> CapturedImage` in `capture.rs` (box filter, keeps the aspect ratio, no-op when it fits), used by Windows OCR before `MaxImageDimension`, so `Unsupported("image too large for OCR")` disappears.
    - `OcrEngine` activation failure (class not registered on N/Server editions) -> `OcrUnavailable`.
    - Monitors: a `GetDpiForMonitor` failure for one monitor -> scale 1.0 (96 DPI) instead of failing the list.
  - The `Platform` bundle gains `popup_window: Arc<dyn NoActivateWindow>` and `dismiss: Arc<dyn DismissWatcher>`.
- Orchestrator wiring in this task: on the `show_at` path the orchestrator starts the dismiss watcher with the popup rect; the shell reports resizes via `Orchestrator::popup_resized(rect)` -> `update_rect`; on hide -> `stop`; a std thread forwards `DismissEvent` to `orchestrator.escape()` / `close()`.

- [ ] **Step 1: Write the failing tests**

Platform (run on the Mac, pure):
```rust
// dismiss_click_inside_rect_is_ignored
// dismiss_click_outside_rect_is_reported
// downscale_keeps_aspect_ratio
// downscale_noop_when_fits
// downscale_averages_pixels
```
`tests/windows_smoke.rs` (`#![cfg(windows)]`, `#[ignore = "needs interactive desktop"]`, run in the dedicated CI step):
```rust
// dismiss_watcher_stops_on_hide (RF 3)          (start, stop, events() receives nothing afterwards, hooks removed)
// show_at_does_not_change_foreground (RF 1)     (GetForegroundWindow before == after show_at on a plain test window)
```
Orchestrator (`tests/orchestrator_actions.rs` addition):
```rust
// dismiss_event_closes_popup                    (FakeDismiss emits Escape -> Hide call, session cleared)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-platform` and `cargo test -p lingopop-orchestrator --test orchestrator_actions`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

Signatures above. `unsafe` stays confined to `src/windows/` and `src/macos/`; every block has `// SAFETY:`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-platform` then `cargo test -p lingopop-orchestrator --test orchestrator_actions`
Expected: PASS locally; the Windows leg is verified by CI below.

- [ ] **Step 5: Documentation, gate, push and commit**

`AGENTS.md`: Layout bullets for the new platform modules and the orchestrator wiring. Run the task gate.
```bash
git add -A && git commit -m "feat(platform): add no-activate popup window, dismiss hooks and Phase 3 carry-overs"
```
With the owner's go-ahead: push `phase4-app`, open the DRAFT PR, and wait for green `gh pr checks`.

---

### Task 7: Tauri shell scaffold and CI

**Files:**
- Create: `app/{package.json, package-lock.json, vite.config.ts, tsconfig.json, svelte.config.js, index.html, src/main.ts, src/popup/Popup.svelte}`, `app/src-tauri/{Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json, icons/icon.ico, icons/icon.png, src/{main.rs, lib.rs, popup_shell.rs, hotkeys.rs, tray.rs, commands.rs, logging.rs}}`
- Modify: root `Cargo.toml` (member), `.github/workflows/ci.yml`, `.github/dependabot.yml`, `.gitignore` (`app/dist`, `app/node_modules`), `AGENTS.md` (Layout; Build / test npm steps), `Cargo.lock`

**Interfaces:**
- Consumes: orchestrator `Orchestrator`, `Deps`, `PopupView`, `ConfigSource`, `Persist`; platform `native()`, `GlobalHotkeyService`; core stores, `ReqwestTransport`, `HttpTimeouts`, `OperationRunner`, `ProviderRegistry`.
- Produces:
  - Package `lingopop-app` (bin `lingopop`), `rust-version = "1.90"`. `tauri.conf.json`: `productName` "LingoPop", identifier `app.lingopop.windows` (verify the mac bundle id is not reused), `app.windows: []`, `build.frontendDist: "../dist"`, `devUrl: "http://localhost:5173"`, CSP `default-src 'self'`. Capabilities limited to the `popup` window and our commands.
  - `setup`: the data dir follows the spec's fixed `%APPDATA%\LingoPop` (NOT `app_data_dir()`): `std::env::var_os("APPDATA")` + `LingoPop` on Windows, `~/Library/Application Support/LingoPop-dev` on macOS dev (a tested `data_dir()` helper). Load `Settings`, `SecretStore` (native protector), `HistoryStore`, `UsageStore`; build `ReqwestTransport::new(HttpTimeouts::default())` and `OperationRunner::new(ProviderRegistry::standard(), ..)`.
  - The popup window is created hidden: `WebviewWindowBuilder::new(app, "popup", WebviewUrl::App("index.html".into())).visible(false).focused(false).focusable(false).decorations(false).always_on_top(true).skip_taskbar(true).resizable(false).shadow(true)` (`transparent` only if verified without a white flash; default false).
  - `popup_shell.rs` implements `PopupView`: emit the `popup://model` event with the view-model JSON to the popup webview; `show_at` -> `platform.popup_window.show_at(WindowHandle(hwnd), rect)`; `hide` -> `popup_window.hide` + dismiss `stop`.
  - `hotkeys.rs`: create `GlobalHotkeyService` in `setup`, keep it in a `thread_local!` holder, register `settings.hotkeys` except ScreenTranslate (4b), log conflicts; a forwarder std thread -> `tauri::async_runtime::spawn(orchestrator.trigger(op))`.
  - `tray.rs`: tray icon + menu with one item "Quit LingoPop" (mac key) -> `app.exit(0)`; `ExitRequested { code: None }` -> `prevent_exit`; `tauri-plugin-single-instance` registered first.
  - `logging.rs`: `tauri-plugin-log` 2.x if its MSRV fits (verify the version), else a small file logger in the data dir (`lingopop.log`, rotate at 1 MB); never user text.
  - `commands.rs`: `popup_action(action: PopupAction)` with `#[derive(Deserialize)] #[serde(tag = "type")] pub enum PopupAction { Copy, Replace, Retry { provider }, Rewrite, Explain, Retranslate { language }, TranslateOcr { provider }, Close }`; `popup_resized(width, height)` (logical px from the frontend; the shell converts with the monitor scale, resizes through `show_at` and updates the dismiss rect); `popup_ready()`.
  - CI: the `core` job adds `actions/setup-node` (pinned by SHA, Node 22, npm cache `app/package-lock.json`) + `npm ci --prefix app` + `npm run build --prefix app` before the cargo steps on both OSes; the Windows leg also runs `npm test` / `npm run check` from Task 8; dependabot gets the `npm` entry for `/app` (deferred since Phase 1) with ignores for tauri 3 alphas.

- [ ] **Step 1: Write the failing tests**

`app/src-tauri` (`cargo test -p lingopop-app`; no `tauri::test`):
```rust
// data_dir_windows_uses_appdata_lingopop            (pure helper taking the env value as input)
// data_dir_macos_dev_uses_application_support_lingopop_dev
// popup_action_json_shape                           (every variant: {"type":"retry","provider":"DeepL"}, {"type":"close"}, ...)
```

- [ ] **Step 2: Run to verify failure**

Run: `npm ci --prefix app && npm run build --prefix app && cargo test -p lingopop-app`
Expected: FAIL (crate missing), then compile errors until the shell exists.

- [ ] **Step 3: Implement**

As above. Icons are generated once from `docs/icon.png` and committed.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-app`, then the task gate. Push and check `gh pr checks` (Windows compiles the shell).
Expected: PASS.

- [ ] **Step 5: Documentation and commit**

`AGENTS.md`: Layout bullets for `app/` and `app/src-tauri/`, Build / test with the npm steps.
```bash
git add -A && git commit -m "feat(app): add the Tauri shell with hidden popup window, hotkeys, tray and CI for the frontend"
```

---

### Task 8: Popup frontend

**Files:**
- Create: `app/src/popup/{Popup,LoadingView,ResultView,OcrView,ErrorView,ActionRow}.svelte`, `app/src/lib/{i18n.ts, actionRow.ts, ipc.ts, icons.ts}`, `app/src/**/*.test.ts`
- Modify: `app/package.json` (scripts, dev dependencies), `app/src/main.ts`, `AGENTS.md`

**Interfaces:**
- Consumes: the view-model JSON from Task 2 (`popup://model` event), commands from Task 7.
- Produces:
  - `Popup.svelte` switches on `phase`. `LoadingView`. `ResultView`: header with the provider name, for Translate "`<Provider> →`" + a language menu from `quickLanguages`, an orange "fallback" capsule, a "Why fallback was used" disclosure, selectable text, a "Response was truncated" warning. `OcrView`: "Recognized text", Copy, a Translate primary button + provider chevron menu "Translate with…", Retry menu. `ErrorView`: "Error", details disclosure "Details", Retry menu.
  - `ActionRow.svelte`: buttons in mac order Copy, Replace, Rewrite, Explain, spacer, Retry menu "Retry with %@". Icon-only rules: `full` all labelled, `mixed` index >= 2 icon-only, `compact` all icon-only. ViewThatFits analog: try `candidates` in order and keep the first whose measured row width fits the max popup width 700 px; tooltips via the `title` attribute.
  - Width rules (mac `PopupLayout.dynamicWidth`): min 320; loading/error = min; result/ocr = `min(700, max(320, 200 + chars))` at font 13. After layout send `popup_resized(width, height)`. Esc is handled by the platform, but `keydown` Escape is also handled in case the popup has focus (dev Mac).
  - Icons: lucide-style hand-drawn inline SVGs committed in `src/lib/icons.ts`, no icon-font dependency.
  - `i18n.ts`: `t(key, args)` English pass-through with `%@`, `%lld`, `%1$@` substitution.
  - Scripts: `build` (vite build), `test` (vitest run), `check` (svelte-check).

- [ ] **Step 1: Write the failing tests**

Vitest + `@testing-library/svelte` + jsdom, with `@tauri-apps/api` `invoke` mocked:
```ts
// t_substitutes_positional_and_ordered_args          (%@ in order, %1$@ / %2$@ positional, %lld numbers)
// action_row_mixed_labels_first_two
// action_row_compact_icon_only
// action_row_picks_first_fitting_candidate           (measured widths injected)
// result_hides_replace_for_screen_translate
// result_shows_fallback_disclosure
// error_shows_retry_menu_when_providers
// ocr_translate_uses_active_provider
// clicking_copy_sends_copy_action                    (invoke("popup_action", { action: { type: "copy" } }))
```

- [ ] **Step 2: Run to verify failure**

Run: `npm test --prefix app`
Expected: FAIL (components missing).

- [ ] **Step 3: Implement**

As above.

- [ ] **Step 4: Run to verify pass**

Run: `npm test --prefix app && npm run check --prefix app && npm run build --prefix app`, then the task gate; push and check `gh pr checks`.
Expected: PASS (9 tests).

- [ ] **Step 5: Documentation and commit**

`AGENTS.md`: Layout bullets for the frontend modules.
```bash
git add -A && git commit -m "feat(app): add the popup frontend with action row, result, OCR and error views"
```

---

### Task 9: Manual checklist, docs, PR ready

**Files:**
- Create: `docs/testing/manual-checklist.md`
- Modify: `AGENTS.md` (Build / test, Layout), `CHANGELOG.md` (Unreleased)

- [ ] **Step 1: Write the checklist**

`docs/testing/manual-checklist.md` seeded with the Phase 3 seeds plus 4a items:
- the popup never takes focus (Notepad caret keeps blinking, typing continues after the popup appears); button clicks do not activate the popup;
- Esc closes only while the popup is visible, and Esc reaches apps after close; a click outside closes;
- Replace into Notepad / Word / Chrome / Telegram; elevated Notepad (UIPI message for capture and Replace);
- Excel / Word clipboard formats restored after capture (check the log for skipped ids); menu-bar activation after Alt key-up; console apps;
- mixed-DPI and multi-monitor placement; taskbar on every edge;
- OCR without a language pack (4b, with the overlay);
- hung clipboard owner (Busy message, the app stays responsive).

- [ ] **Step 2: Docs**

`AGENTS.md`: Build / test section gets the npm steps (`npm ci --prefix app && npm run build --prefix app` before cargo); Layout is complete. `CHANGELOG.md`: Unreleased entry for Phase 4a.

- [ ] **Step 3: Final verification and commit**

Run: `npm ci --prefix app && npm run build --prefix app && npm test --prefix app && npm run check --prefix app`, then the task gate; push and check `gh pr checks` (both `core` legs and CodeRabbit green).
```bash
git add -A && git commit -m "docs: add the manual checklist and Phase 4a notes"
```
Mark the PR ready for review. Never merge (owner decides).
