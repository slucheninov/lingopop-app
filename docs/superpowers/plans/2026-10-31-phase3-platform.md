# Phase 3: Platform layer (clipboard, input, capture, OCR, hotkeys, secrets) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `crates/lingopop-platform`: synchronous OS-primitive traits with a real Windows implementation, a minimal macOS dev implementation and an always-compiled fake; OS-agnostic algorithms on top of the primitives (selection capture, text replace, popup placement, OCR result selection, hotkey mapping), all tested on fakes without an OS; plus Windows smoke tests that CI runs on `windows-latest`.

**Architecture:** Low-level primitive traits (`Clipboard`, `KeySynth`, `ForegroundWindow`, `Monitors`, `ScreenCapture`, `Ocr`, `HotkeyService`) each have a `windows/` impl (shipped), a `macos/` impl (dev only) and a `fake/` impl. The behaviour that must be right on every OS (`capture_selection`, `replace_selection`, `clamp_to_work_area`, `select_best`, `to_chord`) is plain safe Rust over those traits plus an injected `Sleeper`, so every scenario runs instantly on fakes. `native()` assembles the per-OS bundle for the Phase 4 app. `unsafe` is confined to `src/windows/` and `src/macos/` by the compiler (`#![deny(unsafe_code)]` at the crate root, `#[allow(unsafe_code)]` only on those two `mod` declarations).

**Tech Stack:** Rust stable (edition 2021), `windows` 0.62.2 (Windows only), `global-hotkey` 0.9.0 (+ `keyboard-types` 0.8.3, `crossbeam-channel` 0.5), `png` 0.18.1, `thiserror` 2; macOS only: `objc2` 0.6, `objc2-app-kit` 0.3, `objc2-foundation`, `core-graphics` 0.25; existing `lingopop-core`.

**Spec:** `docs/superpowers/specs/2026-10-30-lingopop-app-design.md` (Russian): section 5 "crates/lingopop-platform", section 6 "Windows-специфика" (capture steps, UIPI, clipboard safety, popup, replace, capture, OCR, secrets), section 9b "Platform smoke tests", section 10 CI. macOS reference app (read-only): `/Users/lucheninov/Works/slava/lingo-pop` at v0.4.0 (paths like `Core/TextCapture.swift` are relative to it).

## Roadmap

- **Phase 1 (done):** repo bootstrap + CI + `lingopop-core` foundations.
- **Phase 2 (done, PR #1):** HTTP client, 8 providers, operation runner, live smoke tests.
- **Phase 3 (this plan):** `crates/lingopop-platform` (traits, fake, windows, macos dev impl, algorithms, Windows smoke tests).
- **Phase 4:** Tauri app: orchestration (hotkey -> `capture_selection` -> resolve -> `OperationRunner::run` -> popup), popup/settings/history/tray/overlay, localization, usage aggregation, `tauri-plugin-autostart` / `single-instance`. The app calls the Phase 3 traits through `spawn_blocking` and builds `GlobalHotkeyService` on the Tauri event-loop thread.
- **Phase 5:** NSIS installer, `release.yml`, publish-release script, `RELEASING.md`, update checker, docs polish.

Phase 3 runs on branch `phase3-platform` (created from `main` at `6f407de`). Every task commits on it. Windows code cannot be compiled on the dev Mac (no `rustup`, no Windows target), so CI is the compiler for `src/windows/`: a DRAFT pull request is opened at the end of Task 3 (needed for the desktop probe) and every later Windows-touching task is pushed and checked with `gh pr checks` before the next task starts. Task 8 marks the PR ready. `main` is PR-only (ruleset: `build-and-test` + CodeRabbit required, 1 approval). Pushing REQUIRES EXPLICIT GO-AHEAD FROM THE OWNER AT EXECUTION TIME (one go-ahead for "push `phase3-platform` and keep it updated" is enough).

**Deviation from spec section 5:** the trait list there includes `Autostart`. It is NOT a platform trait: section 6 says autostart is HKCU Run through `tauri-plugin-autostart`, which belongs to Phase 4. Likewise the spec names (`SelectionCapture`, `TextInjector`, `ClipboardSnapshot`, `CursorAndMonitors`, `ScreenRegionCapture`) map to: `capture_selection` + `Clipboard` + `KeySynth`, `replace_selection`, `ClipboardSnapshot` (a value type), `Monitors`, `ScreenCapture`.

## Global Constraints

- `lingopop-core` is unchanged except for the guard in `scripts/check-core-deps.sh`; it must not depend on `lingopop-platform` (normal, build or dev edges). `lingopop-platform` depends on `lingopop-core` only for `Hotkey`, `Modifier`, `SecretProtector`, `ProtectError`.
- `unsafe` appears only under `crates/lingopop-platform/src/windows/` and `src/macos/`. Crate root: `#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]` and `#![warn(clippy::undocumented_unsafe_blocks)]` (an error under `-D warnings`); every `unsafe` block has a `// SAFETY:` comment. `fake`, `error`, all traits and all algorithms are safe code.
- Traits are SYNCHRONOUS and `Send + Sync` (blocking OS calls; Phase 4 wraps them in `spawn_blocking`). The one exception is `HotkeyService` (see Dependency decisions: the Windows hotkey manager owns an `HWND` and is `!Send`).
- User text (clipboard contents, selected text, OCR text, screen pixels) never appears in `Display` / `Debug` of any error or value type, nor in anything passed to the `log` hook. Types that hold such data have hand-written `Debug` printing counts / dimensions only.
- The user's clipboard is never lost: every path that changed the clipboard restores the snapshot (including errors and panics), and a restore never overwrites a newer write by another app.
- No desktop session is assumed in default `cargo test`: tests that need an interactive desktop are `#[ignore = "needs interactive desktop"]` (always compiled and linted, unlike a cargo feature) and run only in the dedicated CI step.
- `lingopop-core` rules still apply to core; tests make no external network calls; no secrets in logs, fixtures or issues.
- One pull request from `phase3-platform`; `Cargo.lock` is committed with every dependency change; CI uses `--locked`.
- Each task appends its modules to the Layout section of `AGENTS.md` in the same commit. Conventional commits, no `Co-Authored-By` / "Generated with" footers.
- `cargo test` takes one positional filter: every step uses the whole crate, `--test <name>` (whole binary) or ONE filter; never several.
- Task gate (run at the end of every task, all four must pass locally on the Mac): `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`. For tasks touching `src/windows/` the gate is additionally the green `core` job (windows leg) of the pushed branch.

## Review Focus

Five failure modes the spec implies but does not spell out, most likely first. Each has a pinning test in the owning task (marked `(RF n)`).

1. **The user's clipboard lost or clobbered** (restore skipped on an error path or panic; restore overwriting something copied meanwhile; a restore after "nothing selected" that drops formats the snapshot could not read; reading between the two sequence bumps of "empty, then set"; clipboard locked by another app). Tests: Task 2 `capture_restores_snapshot_when_read_text_fails`, `capture_restores_snapshot_when_read_text_panics`, `capture_nothing_selected_returns_none_and_never_writes_clipboard`, `capture_waits_for_double_sequence_bump`, `replace_does_not_clobber_newer_clipboard_write`, `replace_paste_blocked_restores_snapshot`, `retry_busy_*`; Task 3 `clipboard_roundtrip_text_html_image`.
2. **User text leaking** through error text, `Debug` of snapshots / images / OCR results, or the `log` hook (the skipped-format log must name formats, never contents). Tests: Task 1 `platform_error_display_and_debug_carry_no_payload`, Task 2 `capture_log_never_contains_selected_text`, `clipboard_snapshot_debug_prints_counts_only`, Task 4 `captured_image_debug_prints_dimensions_only`, Task 5 `ocr_text_debug_prints_length_only`.
3. **Elevated target = silent no-op.** `SendInput` is subject to UIPI, and Microsoft documents that neither its return value nor `GetLastError` reports a UIPI block: the spec's "fewer events inserted" check alone does NOT detect an elevated target. Therefore `ForegroundWindow::is_elevated` (integrity of the target process above ours) is checked BEFORE the clipboard is touched; "fewer events than requested" stays as the second line (BlockInput, desktop switches). Tests: Task 2 `capture_elevated_foreground_is_input_blocked_before_touching_clipboard`, `replace_elevated_target_is_input_blocked`, Task 3 `ensure_inserted_*`, `integrity_blocks_*`.
4. **Hotkey modifiers still held** (Ctrl+Alt+T pressed, Ctrl+C sent: the target sees Ctrl+Alt+C or Ctrl+Shift+C, which in Chrome is DevTools). Wait for release, else synthesize key-up, in this order before the copy; releasing Alt or Win alone makes the target activate its menu bar / Start, so a masking tap is sent before such a key-up. Tests: Task 2 `capture_waits_for_modifiers_release_without_synthesizing`, `capture_releases_held_modifiers_after_timeout`, Task 3 `release_events_mask_alt_and_win`.
5. **Popup placement** on a secondary monitor with negative coordinates, a taskbar on a non-bottom edge and mixed DPI (sizes in physical pixels of the monitor under the anchor). Tests: Task 4 `clamp_left_secondary_monitor_negative_coords`, `clamp_monitor_above_negative_y`, `clamp_taskbar_on_left_keeps_left_inset`, `clamp_mixed_dpi_*`, `clamp_anchor_in_gap_uses_nearest_monitor`.

## Dependency decisions (verified while planning)

Versions checked with `cargo info` / the local registry on 2026-10-31; API facts below were read from the crate sources. Anything marked "verify at implementation" was NOT confirmed (the compiler or docs.rs decides).

- **`windows` 0.62.2** (MSRV 1.82), Windows-only target dependency. Feature names confirmed to exist in its `Cargo.toml`: `Win32_Foundation`, `Win32_Security`, `Win32_Security_Cryptography`, `Win32_Graphics_Gdi`, `Win32_System_DataExchange`, `Win32_System_Memory`, `Win32_System_Ole` (holds the `CF_*` constants), `Win32_System_Threading` (`OpenProcess`, `AttachThreadInput`), `Win32_System_WinRT` (`RoInitialize`), `Win32_System_LibraryLoader`, `Win32_UI_HiDpi`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_UI_WindowsAndMessaging`, `Media_Ocr`, `Graphics_Imaging`, `Storage_Streams`, `Globalization`, `Foundation`, `Foundation_Collections`. Functions confirmed present in the sources: `GetClipboardSequenceNumber`, `OpenClipboard`, `EnumClipboardFormats`, `RegisterClipboardFormatW`, `GlobalAlloc`, `CryptProtectData`, `LocalFree` (in `Win32_Foundation`), `SendInput`, `GetAsyncKeyState`, `GetForegroundWindow`, `SetForegroundWindow`, `GetCursorPos`, `EnumDisplayMonitors`, `GetDpiForMonitor`, `SetThreadDpiAwarenessContext`, `BitBlt`, `GetDIBits`, `AttachThreadInput`, `RoInitialize`; WinRT: `OcrEngine::{TryCreateFromUserProfileLanguages, TryCreateFromLanguage, AvailableRecognizerLanguages, MaxImageDimension, RecognizeAsync}`, `OcrResult::{Text, Lines}`, `SoftwareBitmap::CreateCopyFromBuffer`. Async WinRT results are awaited with `IAsyncOperation::join()` (`windows-future` 0.3.2; verify at implementation that it is not named `get`). Which `Win32_*` features are still missing is found by the compiler; add them one by one, never `Win32` wholesale.
- **`global-hotkey` 0.9.0.** `HotKey::new(Option<Modifiers>, Code)` with `keyboard_types::{Code, Modifiers}` re-exported from `global_hotkey::hotkey`; `Code` implements `FromStr` with the W3C names, so core `Hotkey.key` (`"KeyT"`, `"F5"`, `"ArrowUp"`, ...) parses directly. `HotKey.id` = `(mods.bits() << 16) | key as u32`. Errors: `Error::{OsError, UnrecognizedHotKeyCode, FailedToRegister, FailedToUnRegister, AlreadyRegistered, ...}`. Events: one process-global crossbeam channel `GlobalHotKeyEvent::receiver()` (`id`, `state: Pressed | Released`; on Windows the Released event comes from a polling thread). **Platform constraints (read from `platform_impl`):** on Windows the manager creates a hidden window and a win32 message loop must run on the same thread (Tauri's event loop does); the manager holds an `HWND` so it is `!Send`; on macOS the manager is `Send + Sync` but needs the main-thread event loop. Hence `GlobalHotkeyService` is created on the event-loop thread by Phase 4 and `HotkeyService` is the only platform trait without `Send + Sync`. Phase 4 must not also register hotkeys through `tauri-plugin-global-shortcut` (a second manager with the same global channel); decide there.
- **`keyboard-types` 0.8.3**, default features only (`std`), a direct dependency so that the pure mapping `to_chord` compiles and is tested on every OS; cargo unifies it with the copy `global-hotkey` uses.
- **`crossbeam-channel` 0.5.17**: only to name the `Receiver` type `global-hotkey` hands out.
- **`png` 0.18.1** (pure Rust): `Encoder::new(writer, w, h)` then `set_color(Rgba)`, `set_depth(Eight)`, `write_header()?.write_image_data(..)`; `Decoder::new` takes a `BufRead` (used by the roundtrip test). Used for `CapturedImage::to_png` so the vision path and tests share one encoder; core's `mime_type` then sees the PNG signature.
- **macOS dev impl crates:** `objc2` 0.6.5 + `objc2-app-kit` 0.3.2 (feature `NSPasteboard` confirmed) + `objc2-foundation` for the pasteboard (`changeCount`, string get/set, `clearContents`); `core-graphics` 0.25.0 for `CGEvent` (`new_keyboard_event`, `set_flags`, `post(CGEventTapLocation)`), `CGDisplay::active_displays` / `bounds` / `pixels_wide` and `CGEvent::location` (cursor). Chosen over raw `extern "C"` because they are the maintained, typed bindings and the macOS code is dev-only. `core-foundation` may come in transitively here: the "no `core-foundation`" rule is core-only (`scripts/check-core-deps.sh` inspects `lingopop-core`). `CGEventSourceFlagsState` (held modifiers) is not exposed by `core-graphics` 0.25 as far as the sources show (verify at implementation; fallback is a two-line `extern "C"` under `src/macos/`). `AXIsProcessTrusted` (Accessibility) is a one-line `extern "C"` with `#[link(name = "ApplicationServices", kind = "framework")]`.
- **DPAPI without extra entropy**, CurrentUser scope, `CRYPTPROTECT_UI_FORBIDDEN`, exactly what spec section 6 says. Extra application entropy would not stop another process of the same user (it can read the same constant from our binary), so it is not added; the file stays bound to the Windows user and machine profile.
- **`DevPlaintextProtector` (macOS and other non-Windows, never Windows):** identity "encryption". Reason: the macOS build exists only so the owner can run the app on the dev Mac; a Keychain implementation would be real work for a platform that is never shipped, and the mac app has its own store. It is compiled out on Windows (`#[cfg(not(windows))]`), so it cannot be selected in a shipped build by mistake, and its doc comment says NOT FOR SHIPPING.

## File Structure

```
Cargo.toml                                  members stay crates/*; no change (Task 1 only adds the crate)
crates/lingopop-platform/
  Cargo.toml                                (Tasks 1, 3-7)
  src/lib.rs                                lints, module decls, re-exports, LogFn (Tasks 1-7)
  src/error.rs                              PlatformError                                                  (Task 1)
  src/sleeper.rs                            Sleeper, SystemSleeper, retry_busy                              (Tasks 1, 2)
  src/clipboard.rs                          Clipboard, ClipboardSnapshot, ClipboardEntry, ClipboardFormat   (Task 2)
  src/keys.rs                               KeySynth, VkEvent, chord_events, release_events, ensure_inserted (Tasks 2, 3)
  src/foreground.rs                         ForegroundWindow, WindowHandle, integrity_blocks                (Tasks 2, 3)
  src/selection.rs                          CaptureOptions, capture_selection                               (Task 2)
  src/inject.rs                             ReplaceOptions, ReplaceOutcome, replace_selection               (Task 2)
  src/monitors.rs                           Point, Size, Rect, MonitorInfo, Monitors, clamp_to_work_area    (Task 4)
  src/capture.rs                            CapturedImage, ScreenCapture, validate_region                   (Task 4)
  src/ocr.rs                                Ocr, OcrText, OcrCandidate, select_best                         (Task 5)
  src/hotkey.rs                             HotkeyId, HotkeyEvent, HotkeyService, to_chord (+ to_global_hotkey, GlobalHotkeyService on windows/macos) (Task 6)
  src/dev_protector.rs                      DevPlaintextProtector (#[cfg(not(windows))])                    (Task 7)
  src/native.rs                             Platform bundle, native()                                       (Task 7)
  src/fake/{mod,clipboard,keys,foreground,monitors,sleeper}.rs   always compiled                           (Tasks 1, 2, 4)
  src/windows/{mod,util,clipboard,keys,foreground,monitors,capture,ocr,dpapi}.rs   #[cfg(windows)], unsafe allowed   (Tasks 3-7)
  src/macos/{mod,clipboard,keys,monitors}.rs   #[cfg(target_os = "macos")], unsafe allowed                    (Task 7)
  tests/selection.rs, tests/inject.rs, tests/retry.rs, tests/clamp.rs, tests/ocr_select.rs, tests/hotkey_map.rs
  tests/desktop_probe.rs                    #![cfg(windows)], prints the desktop verdict                     (Task 3)
  tests/windows_smoke.rs                    #![cfg(windows)]                                                 (Tasks 3-8)
  tests/fixtures/ocr/hello.png              copy of core's vision fixture                                    (Task 5)
scripts/check-core-deps.sh                  + lingopop-platform guard                                        (Task 1)
.github/workflows/ci.yml                    + Windows desktop step                                           (Tasks 3, 8)
```

## Algorithms the tests do not determine

**`capture_selection(clip, keys, fg, sleeper, opts) -> Result<Option<String>, PlatformError>`** (spec section 6, with Swift `Core/TextCapture.swift` parity: 300 ms poll timeout, 10 ms interval, a final re-check at the deadline so a copy landing right at it is not lost):

1. If `fg.current()` is `Some(w)` and `fg.is_elevated(w)` -> `Err(InputBlocked)` (nothing touched yet).
2. `snapshot = clip.snapshot()?` (failure aborts: we must not continue when the user's clipboard cannot be saved). Log `skipped` format NAMES. From here a `RestoreGuard` owns the snapshot; its `Drop` (so also on panic) restores when `clip.sequence_number() != snapshot.sequence` (something changed the clipboard) and logs a restore failure without hiding the primary result. An unchanged sequence means nothing to undo and, crucially, keeps formats the snapshot could not read (RF 1).
3. `held = keys.modifiers_down()`; poll every 10 ms up to `modifier_release_timeout` (300 ms) until empty; still held -> `keys.release_modifiers(&held)` (RF 4).
4. `before = clip.sequence_number()`; `keys.send_copy()?`.
5. Poll `sequence_number() != before` every 10 ms up to 300 ms, plus one final check after the deadline; unchanged -> `Ok(None)` (nothing selected). Swift's sentinel marker is NOT ported: Windows bumps the sequence number on every write, even with identical text, and writing a marker would reach clipboard managers.
6. Settle: apps that "empty, then set" bump the sequence twice. After the first change keep re-reading until two consecutive reads (10 ms apart) agree, bounded by 100 ms more, then `clip.read_text()?`. `None`, empty or whitespace-only text (an image or file was copied) -> `Ok(None)`.

**`replace_selection(fg, clip, keys, sleeper, target, text, opts) -> Result<ReplaceOutcome, PlatformError>`** (spec section 6, Swift `App/TextReplacer.swift`: 300 ms foreground confirmation at 20 ms steps, 150 ms before restoring):

1. Elevated target -> `Err(InputBlocked)` before anything is touched.
2. `fg.restore(target)`, then poll `fg.current() == Some(target)` up to 300 ms. On failure: `clip.write_text(text)` as an ordinary entry so the user can paste manually (Swift parity), return `Ok(ReplaceOutcome::ForegroundLost)`. This adds one primitive, `Clipboard::write_text`, to the decided list.
3. `snapshot = clip.snapshot()?`; `clip.write_text_transient(text)?`; `own = clip.sequence_number()`; `keys.send_paste()` (error: restore, then return it); `sleeper.sleep(150 ms)`; restore ONLY if `clip.sequence_number() == own` (Swift's changeCount guard: never clobber a newer write). Return `Ok(Pasted)`.

**`clamp_to_work_area(size, anchor, monitors) -> Point`**: `anchor` is the desired top-left in physical virtual-screen pixels; `size` is in physical pixels of the monitor under the anchor (`monitor_at` is public so the caller can scale first; `physical_size(logical_w, logical_h, scale)` rounds up). Monitor = the one whose `bounds` contain the anchor (half-open), else the nearest by squared distance from the anchor to `bounds`, else (no monitors) return the anchor. `x = anchor.x.min(wa.right - size.w).max(wa.left)` (the `max` last: a popup wider than the work area pins to the top-left), same for `y`. No flip above the cursor; Phase 4 adds an offset to the anchor if it wants one.

**`select_best(candidates) -> Option<OcrCandidate>`**: score = number of non-whitespace chars over the candidate's lines; highest wins; ties go to the earlier candidate (user-profile languages are listed first, English last); no candidates -> `None`; an all-empty best candidate is returned as is (the caller treats empty text as "no text found", like Swift's `noTextFound`).

---

### Task 1: Crate scaffold, `PlatformError`, `Sleeper`, `FakeSleeper`, the dependency guard

**Files:**
- Create: `crates/lingopop-platform/{Cargo.toml, src/lib.rs, src/error.rs, src/sleeper.rs, src/fake/mod.rs, src/fake/sleeper.rs}`
- Modify: `scripts/check-core-deps.sh`, `Cargo.lock`, `AGENTS.md` (Layout; Rules: unsafe confinement; "Windows is verified only by CI" stays)

**Interfaces:**
- Produces (`lingopop_platform`):
  - `#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)] enum PlatformError { Unsupported(&'static str), InputBlocked, ClipboardBusy, OcrUnavailable, HotkeyConflict, InvalidHotkey, InvalidRegion, Os { api: &'static str, code: i32 } }`. `InvalidRegion` (empty or off-screen capture rectangle) is one variant added to the decided list. No variant carries a `String` or user data.
  - `type LogFn = dyn Fn(&str) + Send + Sync;` (messages are fixed text plus format names and counts, never contents).
  - `trait Sleeper: Send + Sync { fn now(&self) -> Duration; fn sleep(&self, d: Duration); }` (`now` = monotonic time since an arbitrary start); `SystemSleeper` (`Instant` + `thread::sleep`).
  - `fake::FakeSleeper`: virtual clock; `new()`, `sleep` advances `now` without waiting and records the duration, `slept() -> Vec<Duration>`, `advance(d)`.

- [ ] **Step 1: Write the failing tests**

`error.rs` (`#[cfg(test)]`):
```rust
// platform_error_display_and_debug_carry_no_payload (RF 2): every variant built, Display non-empty and
//   containing no "{" or quote characters beyond the api name; Os { api: "CryptProtectData", code: 5 }
//   displays exactly "CryptProtectData failed (code 5)"; InputBlocked displays
//   "input was blocked: the target app runs with higher privileges (UIPI)".
// platform_error_is_send_sync_clone_eq
```
`sleeper.rs` / `fake/sleeper.rs`:
```rust
// fake_sleeper_advances_virtual_time_without_waiting   (sleep(300ms) -> now() == 300ms, wall time < 50 ms)
// fake_sleeper_records_every_sleep_in_order
// system_sleeper_now_is_monotonic
```

- [ ] **Step 2: Run to verify failure**

Run: first hand-write `crates/lingopop-platform/Cargo.toml` (Step 3) and an empty `src/lib.rs`, then `cargo test -p lingopop-platform`
Expected: FAIL to compile (the modules do not exist yet).

- [ ] **Step 3: Implement**

- `Cargo.toml`: `[package]` with `version.workspace`, `edition.workspace`, `license.workspace`, `repository.workspace`, description "OS integration traits and implementations for LingoPop"; `[dependencies]`: `lingopop-core = { path = "../lingopop-core" }`, `thiserror = "2"`, `png = "0.18.1"`, `keyboard-types = "0.8.3"`. Target tables are added by the tasks that need them (`global-hotkey` and `crossbeam-channel` under `cfg(any(windows, target_os = "macos"))`, `windows` under `cfg(windows)`, the macOS crates under `cfg(target_os = "macos")`). Run `cargo build -p lingopop-platform` without `--locked` to update `Cargo.lock`.
- `lib.rs`: `#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]`, `#![warn(clippy::undocumented_unsafe_blocks)]`, `pub mod error; pub mod fake; pub mod sleeper;`, `pub use` of the public types. Later tasks add `#[cfg(windows)] #[allow(unsafe_code)] mod windows;` and `#[cfg(target_os = "macos")] #[allow(unsafe_code)] mod macos;` (declared `pub(crate)`; the public surface is `native()` and the concrete re-exports).
- `scripts/check-core-deps.sh`: in part (b) add `name == "lingopop-platform"` to the forbidden list, and add part (c): `cargo tree -p lingopop-core -e normal,build,dev --prefix none` must not contain `lingopop-platform` (cargo already rejects normal and build cycles; a DEV-dependency cycle is legal, so this is the real guard).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-platform`
Expected: PASS (5 tests).

- [ ] **Step 5: Prove both guards**

Run: add `unsafe { }` to `src/error.rs` inside a throwaway `fn`, `cargo build -p lingopop-platform`. Expected: error `usage of an unsafe block` (deny, not warn). Revert.
Run: `cargo add -p lingopop-core --dev --path crates/lingopop-platform && bash scripts/check-core-deps.sh; echo "exit=$?"`. Expected: error naming `lingopop-platform`, `exit=1`. Undo: `cargo remove -p lingopop-core --dev lingopop-platform && cargo build -p lingopop-core`, then the script again: `exit=0`.

- [ ] **Step 6: Documentation, gate and commit**

`AGENTS.md`: Layout bullets for the crate manifest, `error.rs`, `sleeper.rs`, `fake/`; Rules: "`unsafe` is allowed only under `crates/lingopop-platform/src/windows/` and `src/macos/` (crate-level `deny(unsafe_code)`, `// SAFETY:` on every block); core stays `forbid(unsafe_code)`".
Run the task gate.
```bash
git switch -c phase3-platform
git add -A && git commit -m "feat(platform): add lingopop-platform crate with PlatformError, Sleeper and dependency guard"
```

---

### Task 2: Clipboard, keys and foreground traits, fakes, `capture_selection` and `replace_selection`

**Files:**
- Create: `crates/lingopop-platform/src/{clipboard.rs, keys.rs, foreground.rs, selection.rs, inject.rs}`, `src/fake/{clipboard.rs, keys.rs, foreground.rs}`, `tests/{selection.rs, inject.rs, retry.rs}`
- Modify: `src/lib.rs`, `src/sleeper.rs` (`retry_busy`), `src/fake/mod.rs`, `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: Task 1 `PlatformError`, `Sleeper`, `FakeSleeper`, `LogFn`; core `Modifier`.
- Produces:
  - `enum ClipboardFormat { UnicodeText, Html, Rtf, Png, Dib, FileList }`; `struct ClipboardEntry { pub format: ClipboardFormat, pub data: Vec<u8> }`; `struct ClipboardSnapshot { pub entries: Vec<ClipboardEntry>, pub skipped: Vec<String>, pub sequence: u64 }` (`Clone`, hand-written `Debug`: `ClipboardSnapshot { entries: 3, bytes: 1204, skipped: 1, sequence: 7 }`).
  - `trait Clipboard: Send + Sync { fn sequence_number(&self) -> u64; fn snapshot(&self) -> Result<ClipboardSnapshot, PlatformError>; fn restore(&self, &ClipboardSnapshot) -> Result<(), PlatformError>; fn read_text(&self) -> Result<Option<String>, PlatformError>; fn write_text_transient(&self, &str) -> Result<(), PlatformError>; fn write_text(&self, &str) -> Result<(), PlatformError>; }`.
  - `trait KeySynth: Send + Sync { fn modifiers_down(&self) -> Vec<Modifier>; fn release_modifiers(&self, &[Modifier]) -> Result<(), PlatformError>; fn send_copy(&self) -> Result<(), PlatformError>; fn send_paste(&self) -> Result<(), PlatformError>; }`.
  - `struct WindowHandle(pub u64)` (`Debug, Clone, Copy, PartialEq, Eq, Hash`); `trait ForegroundWindow: Send + Sync { fn current(&self) -> Option<WindowHandle>; fn restore(&self, WindowHandle) -> Result<(), PlatformError>; fn is_elevated(&self, WindowHandle) -> bool; }` (`is_elevated` = the window's process runs at a higher integrity level than ours, so UIPI would drop our input; added to the decided trait because of RF 3).
  - `struct CaptureOptions { pub modifier_release_timeout: Duration /*300 ms*/, pub copy_timeout: Duration /*300 ms*/, pub poll_interval: Duration /*10 ms*/, pub settle_cap: Duration /*100 ms*/, pub log: Option<Arc<LogFn>> }` (`Default`, `Debug` without the closure); `fn capture_selection(clip: &dyn Clipboard, keys: &dyn KeySynth, fg: &dyn ForegroundWindow, sleeper: &dyn Sleeper, opts: &CaptureOptions) -> Result<Option<String>, PlatformError>`.
  - `struct ReplaceOptions { pub foreground_timeout: Duration /*300 ms*/, pub foreground_poll: Duration /*20 ms*/, pub restore_delay: Duration /*150 ms*/, pub log: Option<Arc<LogFn>> }`; `enum ReplaceOutcome { Pasted, ForegroundLost }`; `fn replace_selection(fg, clip, keys, sleeper, target: WindowHandle, text: &str, opts: &ReplaceOptions) -> Result<ReplaceOutcome, PlatformError>`.
  - `fn retry_busy<T>(attempts: u32, delay: Duration, sleeper: &dyn Sleeper, op: impl FnMut() -> Result<T, PlatformError>) -> Result<T, PlatformError>`: retries only on `ClipboardBusy`, sleeps `delay` between attempts (not after the last), returns the last `ClipboardBusy` when exhausted. The Windows clipboard uses `retry_busy(10, 10 ms, ..)` around `OpenClipboard`.
  - Fakes (`pub`, always compiled): `FakeClipboard::new(sleeper: Arc<FakeSleeper>) -> Arc<Self>` with `with_text(&str)`, `with_entries(Vec<ClipboardEntry>)`, `with_skipped(Vec<String>)`, `set_external_text(&str)` (another app writes: sequence +1), `text()`, `entries()`, `sequence()`, `calls() -> Vec<ClipboardCall>` (`Snapshot, Restore, ReadText, WriteTransient, Write`), `fail_next(FakeOp, PlatformError)`, `panic_on_read_text()`, `schedule(after: Duration, Mutation)` (applies when virtual time reaches it; `sequence_number()` and every read apply due mutations first). `FakeKeys::new(clip: Arc<FakeClipboard>, sleeper: Arc<FakeSleeper>)` with `on_copy(CopyBehavior)` (`Select(String)`, `SelectAfter(Duration, String)`, `ClearThenSetLate { first: Duration, second: Duration, text: String }`, `NothingSelected`, `ImageOnly`, `Blocked`), `hold_modifiers(Vec<Modifier>, release_after: Option<Duration>)`, `block_paste()`, `on_paste(Box<dyn Fn(&FakeClipboard) + Send + Sync>)`, `events() -> Vec<KeyCall>` (`ReleaseModifiers(Vec<Modifier>)`, `SendCopy`, `SendPaste`), `pasted() -> Vec<Option<String>>` (clipboard text at each paste). `FakeForeground::new(current: Option<WindowHandle>)` with `set_elevated(WindowHandle, bool)`, `fail_restore(bool)`, `foreground_after_polls(n)`, `restored() -> Vec<WindowHandle>`.

- [ ] **Step 1: Write the failing tests**

`tests/selection.rs` (all with `FakeSleeper`, instant; helper `rig()` returns `(clip, keys, fg, sleeper)` with clipboard text `"ORIGINAL"` and a normal foreground window):
```rust
// capture_returns_selected_text_and_restores_original_clipboard   (Select("hello") -> Some("hello"); clip.text() == "ORIGINAL")
// capture_nothing_selected_returns_none_and_never_writes_clipboard (RF 1: NothingSelected -> None; calls() has Snapshot but no Restore/Write*; entries unchanged)
// capture_identical_text_is_still_detected_via_sequence            (clipboard already "same", Select("same") -> Some("same"); Swift needed a sentinel)
// capture_waits_for_slow_copy_within_300ms                         (SelectAfter(250ms) -> Some)
// capture_gives_up_after_300ms                                     (SelectAfter(400ms) -> None, virtual elapsed <= 320 ms)
// capture_final_recheck_catches_copy_landing_at_deadline           (SelectAfter(300ms) -> Some)
// capture_waits_for_double_sequence_bump                           (RF 1: ClearThenSetLate{20ms, 60ms,"x"} -> Some("x"), never Some(""))
// capture_non_text_copy_returns_none_and_restores                  (ImageOnly -> None; clipboard back to original entries)
// capture_empty_or_whitespace_text_is_none                         (Select("  \n\t") -> None)
// capture_waits_for_modifiers_release_without_synthesizing         (RF 4: hold [Ctrl,Alt] released after 80ms -> no ReleaseModifiers event, SendCopy after >= 80ms)
// capture_releases_held_modifiers_after_timeout                    (RF 4: held forever -> events == [ReleaseModifiers([Ctrl,Alt]), SendCopy], elapsed ~300ms)
// capture_elevated_foreground_is_input_blocked_before_touching_clipboard (RF 3: Err(InputBlocked); calls() empty; no key events)
// capture_send_copy_blocked_is_input_blocked_and_clipboard_intact  (RF 3: Blocked -> Err(InputBlocked); clipboard == original)
// capture_restores_snapshot_when_read_text_fails                   (RF 1: copy changes clipboard, fail_next(ReadText, ClipboardBusy) -> Err(ClipboardBusy); Restore called; text() == "ORIGINAL")
// capture_restores_snapshot_when_read_text_panics                  (RF 1: catch_unwind(AssertUnwindSafe(..)) is Err; text() == "ORIGINAL": the guard restores on unwind)
// capture_restore_failure_keeps_text_and_logs                      (fail_next(Restore, ClipboardBusy): Ok(Some("hello")); log has "restore failed")
// capture_snapshot_failure_aborts_before_copy                      (fail_next(Snapshot, ClipboardBusy) -> Err; no SendCopy event)
// capture_logs_skipped_format_names_not_contents                   (RF 2: with_skipped(["format 49161"]): log contains "49161")
// capture_log_never_contains_selected_text                         (RF 2: Select("SECRET-SELECTION-42"); no log line contains it, nor does format!("{:?}", snapshot))
// clipboard_snapshot_debug_prints_counts_only                      (RF 2: entries with b"SECRET" -> Debug lacks "SECRET", contains "entries: 1")
```
`tests/inject.rs`:
```rust
// replace_pastes_text_and_restores_snapshot                        (RF 1: Pasted; pasted() == [Some("NEW")]; clip.text() == "ORIGINAL")
// replace_writes_transient_not_plain_text                          (calls() has WriteTransient, no Write)
// replace_restores_foreground_before_paste                         (restored() == [target]; foreground event precedes SendPaste)
// replace_confirms_foreground_by_polling                           (foreground_after_polls(3) -> Pasted; 2 polls of 20 ms slept)
// replace_foreground_failure_leaves_plain_text_and_reports         (fail_restore(true) -> Ok(ForegroundLost); clip.text() == "NEW"; calls() has Write, no WriteTransient; no SendPaste)
// replace_elevated_target_is_input_blocked                         (RF 3: Err(InputBlocked); clipboard untouched)
// replace_paste_blocked_restores_snapshot                          (RF 1: block_paste() -> Err(InputBlocked); clip.text() == "ORIGINAL")
// replace_does_not_clobber_newer_clipboard_write                   (RF 1: on_paste sets_external_text("USER-COPIED") -> no Restore; text() == "USER-COPIED")
// replace_waits_restore_delay_after_paste                          (slept contains 150ms after the paste; pasted() == [Some("NEW")])
```
`tests/retry.rs`:
```rust
// retry_busy_returns_first_success_without_sleeping
// retry_busy_retries_until_success                    (RF 1: Busy x3 then Ok -> Ok, 3 sleeps of 10 ms)
// retry_busy_gives_up_after_attempts_with_clipboard_busy (10 attempts -> Err(ClipboardBusy), 9 sleeps)
// retry_busy_does_not_retry_other_errors              (Os{..} returned after 1 call)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-platform --test selection`
Expected: FAIL to compile (types and functions missing).

- [ ] **Step 3: Implement**

- Traits and value types as listed; the algorithms exactly as written in "Algorithms the tests do not determine". `RestoreGuard<'a> { clip, snapshot, log }` implements `Drop`; `capture_selection` does its work in an inner fn so the guard is dropped after the result is computed and on unwind.
- Log messages are literals plus counts and format names: `"clipboard snapshot skipped formats: <names>"`, `"clipboard restore failed: <PlatformError Display>"`, `"input blocked: target runs elevated"`. Never format a string that came from the clipboard.
- `FakeClipboard` internals: `Mutex<State { entries, skipped, seq, scheduled: Vec<(Duration, Mutation)>, fail: HashMap<FakeOp, PlatformError>, calls }>`; `Mutation::{SetText(String), Clear, SetImage}` each bumps `seq`; `restore` replaces `entries` and bumps `seq` like Windows; `snapshot` returns `sequence: seq`; `read_text` returns the `UnicodeText` entry decoded as UTF-8 (the fake stores UTF-8, the real one UTF-16). `FakeKeys::send_copy` schedules mutations at `sleeper.now() + delay`; `Select(t)` = `Clear` then `SetText` both at +0 (two bumps, like real apps).
- `retry_busy` as specified; the Windows clipboard passes its own `Arc<dyn Sleeper>`.
- `lib.rs`: `pub mod clipboard, keys, foreground, selection, inject;` and re-exports.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-platform`
Expected: PASS (about 40 tests).

- [ ] **Step 5: Run the task gate and commit**

`AGENTS.md` Layout: one bullet per new module (signatures, the timing constants, the restore guard, the sequence-based "nothing selected", the RF 3 elevation check and why `SendInput` alone cannot detect UIPI).
Run the task gate.
```bash
git add -A && git commit -m "feat(platform): add clipboard, keys, foreground traits, fakes and selection capture/replace algorithms"
```

---

### Task 3: Windows clipboard, `KeySynth`, foreground, the desktop probe

**Files:**
- Create: `src/windows/{mod.rs, util.rs, clipboard.rs, keys.rs, foreground.rs}`, `tests/desktop_probe.rs`, `tests/windows_smoke.rs`
- Modify: `Cargo.toml` (`[target.'cfg(windows)'.dependencies] windows = { version = "0.62.2", features = [Win32_Foundation, Win32_Security, Win32_System_DataExchange, Win32_System_Memory, Win32_System_Ole, Win32_System_Threading, Win32_UI_Input_KeyboardAndMouse, Win32_UI_WindowsAndMessaging] }`; the same `windows` as a `cfg(windows)` dev-dependency with `Win32_System_LibraryLoader` and `Win32_Graphics_Gdi` for the test windows), `src/keys.rs`, `src/foreground.rs`, `src/lib.rs`, `.github/workflows/ci.yml`, `AGENTS.md`

**Interfaces:**
- Consumes: Task 2 traits.
- Produces: pure and tested on every OS (`pub`, `#[doc(hidden)]`): `struct VkEvent { pub vk: u16, pub up: bool, pub extended: bool }`; consts `VK_CONTROL = 0x11`, `VK_MENU = 0x12`, `VK_SHIFT = 0x10`, `VK_LWIN = 0x5B`, `VK_RWIN = 0x5C`, `VK_C = 0x43`, `VK_V = 0x56`, `VK_MASK = 0xE8` (unassigned key used as the menu-suppression tap; verify at implementation); `fn chord_events(modifier_vk: u16, key_vk: u16) -> [VkEvent; 4]` (mod down, key down, key up, mod up: ONE `SendInput` call, so no foreign event interleaves); `fn release_events(&[Modifier]) -> Vec<VkEvent>`; `fn ensure_inserted(requested: usize, inserted: u32) -> Result<(), PlatformError>` (`inserted < requested` -> `InputBlocked`); `fn integrity_blocks(target_elevated: bool, self_elevated: bool) -> bool` (`target && !self`). Windows only: `WindowsClipboard::new(sleeper: Arc<dyn Sleeper>)`, `WindowsKeys`, `WindowsForeground`, all implementing the Task 2 traits.

- [ ] **Step 1: Write the failing tests**

Pure (in `keys.rs` / `foreground.rs` `#[cfg(test)]`, run on the Mac):
```rust
// chord_events_orders_mod_down_key_down_key_up_mod_up
// chord_events_for_copy_and_paste_use_vk_c_and_vk_v
// release_events_for_ctrl_shift_release_both_sides   (Ctrl -> VK_LCONTROL 0xA2 up and VK_RCONTROL 0xA3 up with extended=true; Shift likewise)
// release_events_mask_alt_and_win                    (RF 4: before the first Alt/Win key-up a VK_MASK down+up tap is emitted; none for Ctrl/Shift only)
// release_events_empty_for_no_modifiers
// ensure_inserted_ok_when_all_events_inserted        / ensure_inserted_fewer_events_is_input_blocked (RF 3) / ensure_inserted_zero_is_input_blocked
// integrity_blocks_only_when_target_is_elevated_and_we_are_not (RF 3: 4 combinations)
```
`tests/windows_smoke.rs` (`#![cfg(windows)]`; helper `desktop_test_window()` creates a topmost `EDIT` control window with `WS_POPUP|WS_VISIBLE` and pumps messages briefly; all clipboard tests serialize on a static `Mutex` because the clipboard is global):
```rust
// clipboard_roundtrip_text_html_image          #[ignore = "needs interactive desktop"]  (RF 1: put CF_UNICODETEXT "héllo Привет 🙂", "HTML Format", "PNG" and CF_DIB on the clipboard via the test helper; snapshot; write_text_transient("x"); restore(&snapshot); every format byte-identical; skipped is empty)
// clipboard_write_text_transient_sets_exclusion_formats  #[ignore]  (the three formats are available; CanIncludeInClipboardHistory and CanUploadToCloudClipboard hold a DWORD 0)
// clipboard_sequence_number_increments_on_write          #[ignore]
// clipboard_read_text_none_for_image_only_clipboard      #[ignore]
// clipboard_restore_of_empty_snapshot_clears             #[ignore]
// send_copy_copies_selection_from_edit_window            #[ignore]  (set EDIT text "selected-123", select all via EM_SETSEL, SetForegroundWindow, capture_selection(...) == Some("selected-123"), original clipboard restored)
// foreground_current_and_restore_roundtrip               #[ignore]  (create two windows, restore the first, current() == it)
// modifiers_down_is_empty_when_no_key_is_pressed         #[ignore]
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-platform --lib keys::` Expected: FAIL to compile. (The Windows tests fail only in CI.)

- [ ] **Step 3: Implement**

- `windows/util.rs`: `wide(&str) -> Vec<u16>`, `last_error() -> i32`, `os_err(api, code)`; RAII `ClipboardGuard` (`OpenClipboard` via `retry_busy(10, 10 ms)`; `CloseClipboard` in `Drop`); `HGlobalCopy` helpers: `read_hglobal(handle) -> Vec<u8>` (`GlobalSize` + `GlobalLock` + copy + `GlobalUnlock`), `alloc_hglobal(&[u8]) -> HGLOBAL` (`GMEM_MOVEABLE`; on `SetClipboardData` failure `GlobalFree`, on success ownership moves to the system).
- `windows/clipboard.rs`: formats read in this order: `CF_UNICODETEXT` (13), registered `"HTML Format"`, `"Rich Text Format"`, `"PNG"`, `CF_DIB` (8), `CF_HDROP` (15). `CF_TEXT` / `CF_OEMTEXT` / `CF_LOCALE` / `CF_DIBV5` are NOT stored: Windows synthesizes them from the stored ones (restoring both would duplicate; a DIBV5-only source loses alpha, verify at implementation). A format present but unreadable (null handle, zero size, lock failure) goes to `skipped` as `"format <id or name> unreadable"`; note that `GetClipboardData` renders delayed formats synchronously in the owner process and can stall on an unresponsive owner (manual-checklist item with Office). `snapshot` = `ClipboardGuard` + read all + `GetClipboardSequenceNumber`. `restore` = guard, `EmptyClipboard`, `SetClipboardData` per entry in captured order. `write_text_transient`: guard with a message-only owner window created on the CALLING thread for this write only (`CreateWindowExW("STATIC", HWND_MESSAGE)`, destroyed after `CloseClipboard`; all data is set immediately, never delayed, so no `WM_RENDERFORMAT` reaches a dead window; verify at implementation whether an owner-less `OpenClipboard(None)` works too, then drop the window), `EmptyClipboard`, `CF_UNICODETEXT` (UTF-16 + NUL), `ExcludeClipboardContentFromMonitorProcessing` (1 byte), `CanIncludeInClipboardHistory` (DWORD 0), `CanUploadToCloudClipboard` (DWORD 0). `write_text` = the same without the three formats. `read_text` returns `None` when `CF_UNICODETEXT` is absent; decode with `String::from_utf16_lossy`.
- `windows/keys.rs`: `modifiers_down` = `GetAsyncKeyState(vk) as u16 & 0x8000` for Ctrl, Alt, Shift, Win (either side). `release_modifiers` = `SendInput(release_events(..))` + `ensure_inserted`. `send_copy` / `send_paste` = `SendInput(chord_events(VK_CONTROL, VK_C | VK_V))` + `ensure_inserted`; `INPUT` built with `wVk` (scan code and `KEYEVENTF_EXTENDEDKEY` only for the extended keys in `VkEvent.extended`).
- `windows/foreground.rs`: `current` = `GetForegroundWindow` (null -> `None`); `restore` = `IsWindow` check, `IsIconic` -> `ShowWindow(SW_RESTORE)`, `SetForegroundWindow`, on failure the `AttachThreadInput` trick with the current foreground thread, then `Err(Os { api: "SetForegroundWindow", .. })`; `is_elevated(h)` = `GetWindowThreadProcessId` -> `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` -> `OpenProcessToken(TOKEN_QUERY)` -> `GetTokenInformation(TokenElevation)`, compared with our own token through `integrity_blocks`; if `OpenProcess` fails with access denied while we are not elevated, report `true` (an elevated process we cannot even open); any other failure `false` (never block on a guess).
- `tests/desktop_probe.rs` (`#![cfg(windows)]`, one `#[test] #[ignore = "diagnostic: prints whether the runner has an interactive desktop"]`, never asserts): prints `OpenClipboard` result + last error; `GetClipboardSequenceNumber` (0 = no window-station access); a write/read of `CF_UNICODETEXT`; `GetForegroundWindow` (null?); `GetCursorPos`; `GetSystemMetrics(SM_CMONITORS / SM_CXVIRTUALSCREEN)`; the window-station name via `GetProcessWindowStation` + `GetUserObjectInformationW(UOI_NAME)` (`WinSta0` = interactive); a harmless `SendInput` (key-up of `VK_MASK`) count; `RegisterHotKey(None, 1, MOD_ALT|MOD_NOREPEAT, VK_F9)` result and `UnregisterHotKey`. Last line: `PROBE interactive=<true|false> clipboard=<ok|fail> hotkey=<ok|fail> sendinput=<n>`.
- `ci.yml`: after the `Check core dependencies` step add
```yaml
      - name: Windows desktop tests (diagnostic probe and desktop smoke)
        if: runner.os == 'Windows'
        continue-on-error: true
        shell: bash
        run: cargo test -p lingopop-platform --locked --test desktop_probe --test windows_smoke -- --ignored --nocapture --test-threads=1
```
  (`cargo test` accepts several `--test` flags; this is the one allowed multi-target form because it names whole binaries, not filters.)

- [ ] **Step 4: Run to verify pass, then probe the runner**

Run on the Mac: `cargo test -p lingopop-platform` Expected: PASS (pure tests; `src/windows` is not compiled).
Then push the branch and open a DRAFT PR (go-ahead required): `git push -u origin phase3-platform && gh pr create --draft --base main --head phase3-platform --title "Phase 3: platform layer" --body "Draft: work in progress, see docs/superpowers/plans/2026-10-31-phase3-platform.md"`. Read CI once: `gh pr checks` (no `gh run watch`), then `gh run view <id> --log | grep -E "PROBE|test result"` for the windows leg. The windows leg must compile; fix compile errors reported there before continuing.
Record the verdict in the PR description and in the plan under Task 8: **probe positive** (`interactive=true`, clipboard ok): the desktop step stays and the tests that passed in it are promoted in Task 8; **probe negative**: the step is removed, the `#[ignore]` tests are run by the owner on a Windows machine, and only the no-desktop tests below stay in the default run.

- [ ] **Step 5: Run the task gate and commit**

`AGENTS.md` Layout: bullets for `windows/{util,clipboard,keys,foreground}.rs`, the pure key helpers, `desktop_probe.rs`, `windows_smoke.rs`, the CI desktop step.
```bash
git add -A && git commit -m "feat(platform): add Windows clipboard, SendInput key synthesis, foreground control and desktop probe"
git push
```

---


**Desktop probe verdict (2026-10-31):** positive on `windows-latest` (`PROBE interactive=true clipboard=ok hotkey=ok sendinput=1`, WinSta0, 1 monitor 1024x768). All 15 `--ignored` desktop tests pass; the CI desktop step is REQUIRED (no `continue-on-error`) and the tests stay `#[ignore]`.

### Task 4: Monitors, popup clamping, screen capture

**Files:**
- Create: `src/monitors.rs`, `src/capture.rs`, `src/fake/monitors.rs`, `src/windows/{monitors.rs, capture.rs}`, `tests/clamp.rs`
- Modify: `Cargo.toml` (`windows` features += `Win32_Graphics_Gdi`, `Win32_UI_HiDpi`), `src/lib.rs`, `src/fake/mod.rs`, `tests/windows_smoke.rs`, `AGENTS.md`

**Interfaces:**
- Produces:
  - `struct Point { pub x: i32, pub y: i32 }`, `struct Size { pub width: i32, pub height: i32 }`, `struct Rect { pub x: i32, pub y: i32, pub width: i32, pub height: i32 }` (`left()`, `top()`, `right()`, `bottom()` exclusive, `contains(Point)` half-open, `intersect(Rect) -> Option<Rect>`); `struct MonitorInfo { pub bounds: Rect, pub work_area: Rect, pub scale_factor: f64, pub primary: bool }` (physical virtual-screen pixels; `scale_factor` = effective DPI / 96).
  - `trait Monitors: Send + Sync { fn cursor_position(&self) -> Result<Point, PlatformError>; fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError>; }`; pure `fn monitor_at(p: Point, monitors: &[MonitorInfo]) -> Option<&MonitorInfo>` (containing, else nearest), `fn physical_size(logical_width: f64, logical_height: f64, scale_factor: f64) -> Size` (rounds up), `fn clamp_to_work_area(size: Size, anchor: Point, monitors: &[MonitorInfo]) -> Point`, `fn virtual_screen(monitors: &[MonitorInfo]) -> Rect` (union of bounds).
  - `struct CapturedImage { pub width: u32, pub height: u32, pub rgba: Vec<u8> }` (hand-written `Debug`: `CapturedImage { 400x120 }`); `CapturedImage::new(width, height, rgba) -> Result<Self, PlatformError>` (length must equal `width*height*4`, checked multiply; else `InvalidRegion`); `to_png(&self) -> Result<Vec<u8>, PlatformError>` (8-bit RGBA; encoder failure -> `Unsupported("png encoding failed")`); `trait ScreenCapture: Send + Sync { fn capture_region(&self, region: Rect) -> Result<CapturedImage, PlatformError>; }`; `fn validate_region(region: Rect, virtual_screen: Rect) -> Result<Rect, PlatformError>` (empty or non-positive size, or no intersection -> `InvalidRegion`; a partly outside region is clipped to the intersection).
  - `fake::FakeMonitors::new(cursor: Point, monitors: Vec<MonitorInfo>)`; a `FakeCapture` returning a solid-colour image of the requested size and recording regions (`regions()`).

- [ ] **Step 1: Write the failing tests**

`tests/clamp.rs` (default: monitor A 1920x1080 at (0,0), work area 1920x1040 (taskbar bottom), scale 1.0; popup 400x300):
```rust
// clamp_keeps_anchor_when_popup_fits
// clamp_shifts_left_near_right_edge                   (anchor (1900,100) -> x = 1520)
// clamp_shifts_up_near_bottom_edge_using_work_area    (anchor (100,1000) -> y = 740, not 780)
// clamp_taskbar_on_left_keeps_left_inset              (RF 5: work area x=48 -> anchor (10,10) -> (48,10))
// clamp_left_secondary_monitor_negative_coords        (RF 5: B bounds (-1920,0,1920,1080); anchor (-10,500) -> x = -400; anchor (-1919,5) unchanged)
// clamp_monitor_above_negative_y                      (RF 5: C bounds (0,-1080,1920,1080); anchor (100,-20) -> y = -300)
// clamp_anchor_picks_monitor_containing_it            (two side by side; anchor on the second clamps against the second work area)
// clamp_anchor_in_gap_uses_nearest_monitor            (RF 5: monitors with a 100 px gap; anchor in the gap clamps to the nearer one)
// monitor_at_is_half_open                             (x == bounds.right belongs to the next monitor)
// clamp_mixed_dpi_uses_scaled_size_of_target_monitor  (RF 5: A at 1.0 and D at 1.5 (bounds (1920,0,2560,1440)); physical_size(400.0,300.0,1.5) == 600x450; anchor (4470,100) -> x = 3880)
// physical_size_rounds_up                             (100.5 * 1.25 -> 126)
// clamp_popup_larger_than_work_area_pins_to_top_left
// clamp_no_monitors_returns_anchor
// virtual_screen_is_union_of_bounds_including_negative_origin
// validate_region_rejects_empty_and_zero_size         (InvalidRegion)
// validate_region_rejects_fully_off_screen            (InvalidRegion)
// validate_region_clips_partially_outside_region      (region sticking out left of a negative-origin screen is clipped)
// captured_image_new_rejects_wrong_length             (InvalidRegion; w*h*4 overflow also InvalidRegion)
// captured_image_to_png_roundtrips_pixels             (2x2 known pixels -> png::Decoder -> same width, height, bytes; output starts with 89 50 4E 47)
// captured_image_debug_prints_dimensions_only         (RF 2: Debug has "2x2", never the pixel bytes)
// fake_capture_records_regions
```
`tests/windows_smoke.rs` additions:
```rust
// monitors_report_at_least_one_primary_with_positive_scale     #[ignore = "needs interactive desktop"]  (scale_factor >= 1.0, work_area inside bounds)
// cursor_position_is_inside_virtual_screen                     #[ignore]
// capture_region_returns_known_window_colour                   #[ignore]  (topmost WS_POPUP window 200x100 at (100,100) with a solid magenta brush; capture its centre 50x50; every pixel == [255,0,255,255]; alpha forced to 255)
// capture_region_out_of_screen_is_invalid_region
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-platform --test clamp` Expected: FAIL to compile.

- [ ] **Step 3: Implement**

- Pure code exactly as in "Algorithms": `clamp` uses `monitor_at` then `x = anchor.x.min(wa.right() - size.width).max(wa.left())`. `to_png` with the `png` crate (Task 1 dependency).
- `windows/monitors.rs`: wrap every call in a thread-scoped `SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)` guard that restores the previous context on drop, so results are physical pixels even when the host process has another awareness (Tauri's manifest sets PMv2 in Phase 4; the guard makes tests and the probe independent of it). `GetCursorPos`; `EnumDisplayMonitors` with a callback pushing `GetMonitorInfoW` (`rcMonitor`, `rcWork`, `MONITORINFOF_PRIMARY`) and `GetDpiForMonitor(MDT_EFFECTIVE_DPI)` / 96.
- `windows/capture.rs`: same awareness guard; `validate_region` against `virtual_screen(monitors)`; `GetDC(None)`, `CreateCompatibleDC`, `CreateCompatibleBitmap`, `BitBlt(SRCCOPY | CAPTUREBLT)` from `(region.x, region.y)` in virtual-screen coordinates (negative values are valid), `GetDIBits` into a top-down 32-bit BGRA buffer (`biHeight` negative), swap to RGBA, set alpha 255 (GDI leaves 0). Release in reverse order with an RAII guard (also on error). Mixed-DPI correctness of `BitBlt` from the screen DC under PMv2 is the spec's open point: verify at implementation on a mixed-DPI setup (manual checklist), CI only has one monitor. `Windows.Graphics.Capture` is not used (needs a consent-free-capture capability story and a window/monitor item; GDI is enough for a rectangle).
- Mac: `Monitors` and `ScreenCapture` for macOS come in Task 7 (`ScreenCapture` there returns `Unsupported`).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-platform` Expected: PASS (about 60 tests). Push and check the windows leg (`gh pr checks`): it must compile and the desktop step output must show the new smoke tests.

- [ ] **Step 5: Run the task gate and commit**

`AGENTS.md` Layout: `monitors.rs`, `capture.rs`, `fake/monitors.rs`, `windows/{monitors,capture}.rs` bullets (physical pixels, the half-open rules, the PMv2 thread guard, alpha forced to 255).
```bash
git add -A && git commit -m "feat(platform): add monitors, popup clamping and GDI screen capture"
git push
```

---

### Task 5: OCR (pure best-result selection and Windows.Media.Ocr)

**Files:**
- Create: `src/ocr.rs`, `src/windows/ocr.rs`, `tests/ocr_select.rs`, `crates/lingopop-platform/tests/fixtures/ocr/hello.png` (copy of `crates/lingopop-core/tests/fixtures/vision/hello.png`; a copy so the platform crate has no cross-crate path)
- Modify: `Cargo.toml` (`windows` features += `Media_Ocr`, `Graphics_Imaging`, `Storage_Streams`, `Globalization`, `Foundation`, `Foundation_Collections`, `Win32_System_WinRT`), `src/lib.rs`, `tests/windows_smoke.rs`, `AGENTS.md`

**Interfaces:**
- Produces: `struct OcrText { pub text: String, pub language: Option<String> }` (hand-written `Debug`: `OcrText { chars: 11, language: Some("en-US") }`; `is_empty()` = no non-whitespace char); `struct OcrCandidate { pub language: String, pub lines: Vec<String> }`; `fn select_best(candidates: Vec<OcrCandidate>) -> Option<OcrText>` (score and tie rule in "Algorithms"; text = non-empty lines joined with `"\n"`, Swift joins observations with `"\n"` top to bottom; `OcrResult.Lines` already come in reading order); `trait Ocr: Send + Sync { fn recognize(&self, image: &CapturedImage) -> Result<OcrText, PlatformError>; }`; `WindowsOcr::new()`; a `FakeOcr` (scripted result).

- [ ] **Step 1: Write the failing tests**

`tests/ocr_select.rs`:
```rust
// select_best_picks_candidate_with_most_non_whitespace_chars   ("en" 5 chars vs "uk" 9 chars -> uk)
// select_best_ignores_whitespace_when_scoring                  (lines of spaces do not win)
// select_best_tie_goes_to_earlier_candidate                    (user-profile language first)
// select_best_none_for_no_candidates
// select_best_returns_empty_text_when_every_candidate_is_empty (Some(text "") so the caller shows "no text found")
// select_best_joins_non_empty_lines_with_newline
// select_best_counts_scalars_not_bytes                         (Cyrillic and emoji)
// ocr_text_debug_prints_length_only                            (RF 2: Debug lacks the text)
// fake_ocr_returns_scripted_result_and_error
```
`tests/windows_smoke.rs` additions (`#![cfg(windows)]`; an `ocr_or_skip()` helper maps `Err(OcrUnavailable)` to `eprintln!("skipped: no OCR language pack installed on this runner")` and returns, so a runner without packs is not a failure):
```rust
// ocr_recognizes_fixture_png                    #[ignore = "needs OCR language pack"]  (decode tests/fixtures/ocr/hello.png into a CapturedImage; recognize; lowercase text contains "hello")
// ocr_without_language_packs_is_ocr_unavailable_or_text   (never panics; either Ok or Err(OcrUnavailable))
// ocr_image_larger_than_max_dimension_is_unsupported      (OcrEngine::MaxImageDimension() + 1 wide -> Err(Unsupported))
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-platform --test ocr_select` Expected: FAIL to compile.

- [ ] **Step 3: Implement**

- `select_best`, `OcrText`, `OcrCandidate`, `Ocr`, `FakeOcr` pure.
- `windows/ocr.rs`: per calling thread call `RoInitialize(RO_INIT_MULTITHREADED)` once (ignore `S_FALSE` and `RPC_E_CHANGED_MODE`; the Phase 4 app calls through `spawn_blocking`, never from the STA UI thread, because `join()` on an STA thread can deadlock). Engines: `OcrEngine::TryCreateFromUserProfileLanguages()` (ignore a null / error result), then English via `AvailableRecognizerLanguages()` (tag starts with `en`) + `TryCreateFromLanguage` when its tag differs from the first engine's; no engine at all -> `Err(OcrUnavailable)` (Phase 4 shows the Settings-app hint and offers vision retry). Image: `width`/`height` above `OcrEngine::MaxImageDimension()` -> `Err(Unsupported("image too large for OCR"))`. RGBA -> BGRA in place, `DataWriter::new()` + `WriteBytes` + `DetachBuffer`, `SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w, h)`, then for each engine `RecognizeAsync(&bitmap)?.join()?` and collect `Lines()` text into an `OcrCandidate { language: engine.RecognizerLanguage().LanguageTag() }`; an engine that errors is skipped (the others still count), all failing -> the last error as `Os { api: "OcrEngine::RecognizeAsync", code }`. Result = `select_best`.
- No confidence-based selection: the spec allows it "if available", and `Windows.Media.Ocr` exposes no per-line confidence (verify at implementation), so only the character-count rule is implemented.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-platform` Expected: PASS. Push; the windows leg must compile; the desktop step prints either the recognized text result or the skip line.

- [ ] **Step 5: Run the task gate and commit**

`AGENTS.md` Layout: `ocr.rs`, `windows/ocr.rs`, the fixture.
```bash
git add -A && git commit -m "feat(platform): add OCR trait, best-result selection and Windows.Media.Ocr engine"
git push
```

---

### Task 6: Hotkeys (pure mapping and the `global-hotkey` service)

**Files:**
- Create: `src/hotkey.rs`, `tests/hotkey_map.rs`
- Modify: `Cargo.toml` (`[target.'cfg(any(windows, target_os = "macos"))'.dependencies] global-hotkey = "0.9.0"`, `crossbeam-channel = "0.5.17"`), `src/lib.rs`, `tests/windows_smoke.rs`, `AGENTS.md`

**Interfaces:**
- Produces (all OSes): `struct HotkeyId(pub u32)`; `struct HotkeyEvent { pub id: HotkeyId }` (key pressed; releases are dropped); `trait HotkeyService { fn register(&self, hotkey: &Hotkey) -> Result<HotkeyId, PlatformError>; fn unregister(&self, id: HotkeyId) -> Result<(), PlatformError>; }` (NOT `Send + Sync`, see Dependency decisions); `fn to_chord(hotkey: &Hotkey) -> Result<(keyboard_types::Modifiers, keyboard_types::Code), PlatformError>` (`Ctrl` -> `CONTROL`, `Alt` -> `ALT`, `Shift` -> `SHIFT`, `Win` -> `META` (`SUPER` is deprecated in `keyboard-types`; `global-hotkey` maps both to MOD_WIN / Cmd); key via `Code::from_str`; `!is_valid()` or an unknown code -> `InvalidHotkey`).
- Produces (windows and macos): `fn to_global_hotkey(&Hotkey) -> Result<global_hotkey::hotkey::HotKey, PlatformError>`; `GlobalHotkeyService::new() -> Result<Self, PlatformError>` (must run on the thread that runs the OS event loop), `events(&self) -> HotkeyEvents` (a clone of the global crossbeam receiver; `recv`, `recv_timeout(Duration)`, `try_recv`, each returning only `Pressed` events of ids this service registered, mapped to `HotkeyEvent`). `FakeHotkeyService` (always compiled: registers, `fire(id)`, conflict simulation).
- Error mapping: `AlreadyRegistered` and an `OsError` with raw code 1409 (`ERROR_HOTKEY_ALREADY_REGISTERED`) -> `HotkeyConflict`; `UnrecognizedHotKeyCode`, `FailedToRegister` (unknown VK) -> `InvalidHotkey`; other -> `Os { api: "RegisterHotKey", code }`.

- [ ] **Step 1: Write the failing tests**

`tests/hotkey_map.rs` (`to_chord` tests run on every OS, `to_global_hotkey` ones on windows and macos):
```rust
// to_chord_maps_default_translate_hotkey              (Ctrl+Alt+KeyT -> (CONTROL | ALT, Code::KeyT))
// to_chord_maps_win_to_meta                           (Win+Shift+Digit1 -> (META | SHIFT, Code::Digit1))
// to_chord_accepts_every_key_core_accepts             (loop over KeyA-KeyZ, Digit0-9, F1-F24 and every named key core accepts: Ok; a key core accepts without a Code is named in the failure message)
// to_chord_rejects_hotkey_without_modifier            (InvalidHotkey)
// to_chord_rejects_unknown_key                        (InvalidHotkey for "KeyÄ", "", "Numpad9")
// to_global_hotkey_id_is_stable_and_distinct          (same hotkey twice -> same id; Ctrl+Alt+T vs Ctrl+Alt+Y differ)
// fake_hotkey_service_register_conflict_and_unregister (second register of the same chord -> HotkeyConflict; after unregister it succeeds)
// fake_hotkey_service_fire_delivers_event
```
`tests/windows_smoke.rs` additions:
```rust
// hotkey_register_conflict_and_unregister       #[ignore = "needs interactive desktop"] until Task 8 promotes it: a thread creates GlobalHotkeyService; register Ctrl+Alt+F9 -> Ok; a second service on that thread registering the same chord -> Err(HotkeyConflict); unregister; register again -> Ok
// hotkey_press_delivers_event                   #[ignore]  (register, SendInput the chord, pump messages with a PeekMessage loop for 500 ms, events().recv_timeout(1 s) yields the id)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-platform --test hotkey_map`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

- `GlobalHotkeyService` holds the `GlobalHotKeyManager` (created in `new`; `!Send` on Windows, so the struct is `!Send` and the compiler enforces "stay on the event-loop thread") and a `Mutex<HashMap<u32, Hotkey>>` of registered ids; `register` = `to_global_hotkey` + `manager.register` + record; the id is `HotKey::id()`. `events()` filters `GlobalHotKeyEvent` by `state == Pressed` and a registered id. Never call `GlobalHotKeyEvent::set_event_handler` (it replaces the channel for the whole process).
- On macOS some keys (for example F20-F24) may lack a Carbon key code in `global-hotkey` 0.9.0 and then map to `InvalidHotkey` through the table above (verify at implementation which; the Windows table is complete, F24 confirmed in `key_to_vk`).
- Settings-UI validation stays in core (`Hotkey::is_valid`); this task only maps.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-platform`
Expected: PASS. Push; the windows leg compiles and reports the two hotkey tests in the desktop step.

- [ ] **Step 5: Run the task gate and commit**

`AGENTS.md` Layout: `hotkey.rs` (threading rule, `Pressed` only, id = the global-hotkey id).
```bash
git add -A && git commit -m "feat(platform): add hotkey mapping and global-hotkey service"
git push
```

---

### Task 7: DPAPI protector, macOS dev implementation, `native()` bundle

**Files:**
- Create: `src/windows/dpapi.rs`, `src/dev_protector.rs`, `src/macos/{mod.rs, clipboard.rs, keys.rs, monitors.rs}`, `src/native.rs`, `tests/macos_dev.rs`
- Modify: `Cargo.toml` (`windows` features += `Win32_Security_Cryptography`; `[target.'cfg(target_os = "macos")'.dependencies] objc2 = "0.6.5"`, `objc2-app-kit = { version = "0.3.2", features = ["NSPasteboard", ...] }`, `objc2-foundation`, `core-graphics = "0.25.0"`), `src/lib.rs`, `tests/windows_smoke.rs`, `AGENTS.md`

**Interfaces:**
- Produces:
  - `WindowsProtector: SecretProtector`: `protect` = `CryptProtectData(&in, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)`, `unprotect` = `CryptUnprotectData` with the same flags; copy the `DATA_BLOB` output into a `Vec`, then `LocalFree(out.pbData)` in a guard so it is freed on every path; failures are `ProtectError::Failed("CryptProtectData failed (code N)")`, never including plaintext.
  - `DevPlaintextProtector` (`#[cfg(not(windows))]`, identity; doc comment `NOT FOR SHIPPING`, with the reason from Dependency decisions).
  - macOS: `MacClipboard` (text-only snapshot: a `UnicodeText` entry from `stringForType(NSPasteboardTypeString)`; `sequence_number` = `changeCount`; `write_text_transient` also sets `org.nspasteboard.TransientType` / `ConcealedType` like `Core/PasteboardSnapshot.swift`; `restore` = `clearContents` then `setString`, or only the clear for an empty snapshot), `MacKeys` (`CGEvent` Cmd+C = key code 8, Cmd+V = 9, `.cgAnnotatedSessionEventTap`, `combinedSessionState`, exactly like `Core/KeyboardSimulator.swift`; `send_*` return `InputBlocked` when `AXIsProcessTrusted()` is false, the doc says to grant Accessibility to the terminal or app; `modifiers_down` from the combined-session flags state with `maskCommand` mapped to `Win`), `MacForeground` (`current()` -> `None`, `restore` -> `Ok`, `is_elevated` -> `false`: dev only, Replace pastes into whatever is frontmost), `MacMonitors` (`CGDisplay::active_displays`, `bounds` used as both `bounds` and `work_area`, scale = `pixels_wide / bounds.width`; points not physical pixels, acceptable for dev), `ScreenCapture` / `Ocr` returning `Err(Unsupported("screen capture on macOS"))` / `Err(Unsupported("OCR on macOS"))`.
  - `struct Platform { pub clipboard: Arc<dyn Clipboard>, pub keys: Arc<dyn KeySynth>, pub foreground: Arc<dyn ForegroundWindow>, pub monitors: Arc<dyn Monitors>, pub capture: Arc<dyn ScreenCapture>, pub ocr: Arc<dyn Ocr>, pub protector: Arc<dyn SecretProtector>, pub sleeper: Arc<dyn Sleeper> }` and `#[cfg(any(windows, target_os = "macos"))] pub fn native() -> Platform`. The hotkey service stays separate (thread rule). On other OSes `native()` is not provided and the crate builds with the fake only.

- [ ] **Step 1: Write the failing tests**

`tests/windows_smoke.rs` (`#![cfg(windows)]`; these need NO desktop and run in the default `cargo test` on the windows leg):
```rust
// dpapi_roundtrip_ascii_unicode_and_empty_payload      (protect then unprotect returns the bytes for {"apiKey.Claude":"sk-ant-FIXTURE","n":"Привет"} and b""; verify at implementation that DPAPI accepts empty input, else pin the error)
// dpapi_protected_bytes_do_not_contain_plaintext       (windows(..).any(|w| w == b"sk-ant-FIXTURE") is false)
// dpapi_tampered_ciphertext_fails_to_unprotect         (flip the last byte, and truncate by one: both Err(ProtectError::Failed(_)); the message has no plaintext)
// dpapi_garbage_input_fails_to_unprotect               (b"not dpapi data" -> Err)
// dpapi_works_through_secret_store                     (SecretStore::open in a tempdir with WindowsProtector and SystemClock, set a key, reload: same key, the file bytes lack the plaintext)
// native_bundle_constructs                             (native() returns without a desktop; its protector roundtrips)
```
`tests/macos_dev.rs` (`#![cfg(target_os = "macos")]`, runs on the macos leg and the Mac):
```rust
// dev_protector_is_identity
// native_bundle_on_macos_has_unsupported_capture_and_ocr  (capture_region -> Unsupported; recognize -> Unsupported)
// mac_monitors_report_a_main_display                      (>= 1 monitor, scale_factor >= 1.0, non-empty bounds; may be #[ignore] if the macos runner is headless: decide from CI)
// mac_clipboard_roundtrip_text                            #[ignore = "touches the real pasteboard"]  (snapshot, write_text_transient("x"), restore: original text back; owner runs it locally)
```
Pure (any non-Windows OS): `dev_protector_roundtrips_bytes`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-platform --test macos_dev`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

As listed. `native()` on Windows: `WindowsClipboard::new(sleeper.clone())`, `WindowsKeys`, `WindowsForeground`, `WindowsMonitors`, `WindowsCapture`, `WindowsOcr`, `WindowsProtector`, `SystemSleeper`. Every `unsafe` block carries `// SAFETY:` (clippy enforces it). The `#[cfg(not(windows))]` gate is itself the guard that a Windows build cannot name `DevPlaintextProtector`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-platform`
Expected: PASS on the Mac. Push; the windows leg runs the DPAPI tests in the default run (they must pass, not just compile); the macos leg runs `macos_dev`.

- [ ] **Step 5: Run the task gate and commit**

`AGENTS.md` Layout: `windows/dpapi.rs`, `dev_protector.rs`, `macos/*`, `native.rs`; Rules: "`DevPlaintextProtector` is dev-only and compiled out on Windows".
```bash
git add -A && git commit -m "feat(platform): add DPAPI protector, macOS dev implementation and native bundle"
git push
```

---

### Task 8: Finalize smoke tests and CI, documentation, mark the PR ready

**Files:**
- Modify: `tests/windows_smoke.rs`, `.github/workflows/ci.yml`, `AGENTS.md`, `CHANGELOG.md`, `README.md`, `CONTRIBUTING.md`, this plan (record the probe verdict)

- [ ] **Step 1: Decide the desktop gating from the real CI logs**

Run: `gh pr checks`, then for the newest windows run `gh run view <id> --log | grep -E "PROBE|test .* \.\.\. (ok|FAILED|ignored)"`.
- **Probe positive** (`interactive=true`): keep the `Windows desktop tests` step (`--ignored`, `continue-on-error: true`). Promote to the default run (remove `#[ignore]`) only tests that passed in it on two consecutive pushes AND need no foreground window or cursor: `clipboard_sequence_number_increments_on_write`, `clipboard_roundtrip_text_html_image`, `hotkey_register_conflict_and_unregister`. Keep `#[ignore]` on everything that depends on focus or the cursor (`send_copy_*`, `foreground_*`, `hotkey_press_*`, `capture_region_*`).
- **Probe negative:** delete the step, keep all desktop tests `#[ignore]`, keep `tests/desktop_probe.rs` for the owner, and add to `CONTRIBUTING.md`: "Platform checks: on a Windows machine with a desktop run `cargo test -p lingopop-platform --test windows_smoke -- --ignored --nocapture --test-threads=1`".
Either way the DPAPI tests, `native_bundle_constructs`, the skip-aware `ocr_*` tests and all pure tests stay in the default run. Write the verdict and date into the Task 3 section of this plan and the PR description.

- [ ] **Step 2: Documentation**

- `AGENTS.md`: re-read every Layout bullet added by Tasks 1-7 against the final file names; new section "Platform checks" (what runs where, the `#[ignore]` rule, how to run the desktop tests, the Accessibility note for the macOS dev impl: System Settings, Privacy & Security, Accessibility for the terminal or the built app, otherwise `send_copy` returns `InputBlocked`).
- `CHANGELOG.md` under `## Unreleased`: "Platform layer for Windows (selection capture, replace, clipboard safety, screen capture, OCR, global hotkeys, DPAPI secrets) and a macOS dev implementation (not user-visible)".
- `README.md`: status line becomes "pre-alpha, Phase 3 (platform layer)". No version bump.
- Seeds for Phase 4's `docs/testing/manual-checklist.md` (listed here so they are not lost): menu-bar activation after a synthesized Alt key-up (Notepad, Word); console apps where Ctrl+C does not copy; elevated target (expect `InputBlocked`); Excel / Word clipboard formats after a capture (skipped formats); mixed-DPI capture; taskbar on every edge; OCR with no language pack.

- [ ] **Step 3: Run the final gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh && git status --short`
Expected: all pass; `git status` shows only Task 8 files. Run the Task 1 Step 5 guard proofs once more. Record the `cargo test --workspace` counts and the windows-leg counts for the PR body.

- [ ] **Step 4: Commit, push, mark the pull request ready (REQUIRES EXPLICIT GO-AHEAD FROM THE OWNER)**

```bash
git add -A && git commit -m "docs(platform): finalize smoke test gating, CI and platform documentation"
git push
cat > /tmp/phase3-pr-body.md <<'EOF'
## Summary
- New crate `lingopop-platform`: synchronous OS-primitive traits (`Clipboard`, `KeySynth`, `ForegroundWindow`, `Monitors`, `ScreenCapture`, `Ocr`, `HotkeyService`), a Windows implementation (SendInput, clipboard with history exclusion, GDI capture, Windows.Media.Ocr, `global-hotkey`, DPAPI), a minimal macOS dev implementation and an always-compiled fake.
- OS-agnostic algorithms tested on fakes: `capture_selection` (restore on every path, modifier release, sequence-number polling), `replace_selection`, `clamp_to_work_area`, `select_best`, `to_chord`.
- `unsafe` confined to `src/windows` and `src/macos` by `#![deny(unsafe_code)]`; `check-core-deps.sh` also forbids core -> platform edges.
- Autostart is not a platform trait (HKCU Run via `tauri-plugin-autostart` in Phase 4): a deliberate deviation from spec section 5.
- UIPI: `SendInput` cannot report a UIPI block, so the target's integrity level is checked before the clipboard is touched.

## Tests
- `cargo test --workspace --locked`: <N> passed, <M> ignored (desktop). Windows leg: DPAPI and pure tests in the default run; desktop probe verdict: <result>.

## Note
`main` is PR-only (ruleset: `build-and-test` and CodeRabbit required, 1 approval).
EOF
gh pr edit --body-file /tmp/phase3-pr-body.md && gh pr ready
```
Fill `<N>`, `<M>`, `<result>` with real values first; the body must not contain any "Generated with Claude Code" footer. Expected: the PR leaves draft state. Check CI once with `gh pr checks`; likely Windows-only failures: a missing `windows` feature (the compiler names it), a desktop test left un-ignored by mistake, a message-pump hang in a hotkey test (add a hard timeout).
