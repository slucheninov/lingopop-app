# Phase 4b: Settings and History windows, tray menu, Screen Translate overlay, localization, usage, autostart, update check — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** everything the macOS status menu offers, on Windows: a full Settings window (Main / Providers / Shortcuts / Usage / Magic / About, API keys entered through the UI and never shown to the webview), a History window, the full tray menu with idle / working / error icons, a native Win32 region overlay that enables the Screen Translate hotkey, en / pl / uk / de localization generated from the mac catalog, usage statistics with the mac pricing table, launch at login through `HKCU\...\Run`, and a GitHub release check in About.

**Architecture:** pure logic goes to `lingopop-core` (`ui_prefs`, `updates`, `provider_draft`, `usage_pricing`, `usage_stats`, `i18n`, hotkey duplicates) and is tested on the Mac. New OS seams are traits in `lingopop-platform` with fakes and Windows impls (`SystemInfo`, `LoginItem`, `LastForeground`, `RegionOverlay`); macOS dev impls are minimal. The orchestrator gains a `StatusSink` (tray state) and a `TriggerSource` (tray-triggered text operations restore the last external window first). The Tauri shell (`app/src-tauri`) adds two ordinary focusable windows (`settings`, `history`) served by the same Svelte bundle (route by window label, lazy-loaded), async-only commands (the IPC contract below), events, a rebuildable tray menu, hotkey re-registration on the event-loop thread, and the overlay-backed `RegionPicker`. The frontend adds the Settings and History apps, a real catalog behind the existing `t()` and a popup font-size parameter.

**Tech Stack:** as Phase 4a (tauri 2.12.5, Svelte 5.57.3, Vite 8.3.5, Vitest 5.0.5, TypeScript 6.0.3 host + `@typescript/native` 7.0.2, `windows` 0.62.2 in the platform crate only). New: `chrono` (core, `default-features = false`, features `["std", "clock"]` NOT enabled: core gets the UTC offset as a parameter) for calendar math; Python 3 (standard library only) for the localization generator; no new Tauri plugins.

**Spec:** `docs/superpowers/specs/2026-11-01-phase4b-windows-ui-design.md` (Russian; owner decisions and assistant calls are marked there and are not reopened here). Overall design: `docs/superpowers/specs/2026-10-30-lingopop-app-design.md`. The macOS app `/Users/lucheninov/Works/slava/lingo-pop` (read-only) is the behavioural spec; `file:line` references below with a Swift path are into that repo. Phase 4a plan (format reference): `docs/superpowers/plans/2026-10-31-phase4a-orchestration-popup.md`.

## Global Constraints

- `lingopop-core` stays OS-agnostic: `#![forbid(unsafe_code)]`, no `windows*` / `tauri*` / `objc*` / platform TLS, no direct `tokio`; `scripts/check-core-deps.sh` must pass after every task. `lingopop-orchestrator` has no `tauri*` / `windows*` dependency and `#![forbid(unsafe_code)]`.
- `unsafe` only under `crates/lingopop-platform/src/windows/` and `src/macos/`, every block with `// SAFETY:`.
- Every Tauri command is `async` (the main thread services the cross-thread `SetWindowPos` of the popup); no blocking I/O or window waits on the main thread; `GlobalHotkeyService` stays in the `thread_local!` of the event-loop thread and is touched only through `AppHandle::run_on_main_thread`.
- Windows are NEVER created inside a synchronous callback: `WebviewWindowBuilder::build` deadlocks on Windows when called from sync commands and event handlers (tauri 2.12.5 `webview/webview_window.rs:55-58`). The single-instance callback, tray / menu handlers and window-event handlers only `tauri::async_runtime::spawn` and return; `block_on` is never used in a callback. Window creation is serialized per label (Task 4 `windows.rs`).
- No `std::sync::Mutex` of the shell or orchestrator is held across `run_on_main_thread`, an `.await`, or a callback that may run inline on the event-loop thread (a reentrant lock is a deadlock).
- The API key never reaches the webview: no command response, event, log line, `Debug` output or component state carries it; the webview sends it exactly once in `provider_save` and clears the field.
- `settings.json` keeps the exact 12-field `SettingsStore.Persisted` shape; every Windows-only value lives in `ui.json`. `settings.json` is written only by `Settings::save`, `secrets.dat` only by `SecretStore`, `history.json` / `usage.json` only by their stores; a store that failed to open (`None`) is never overwritten.
- Popup rules of Phase 4a are untouched: the popup is never activated; Settings / History / overlay must not change dismiss-hook behaviour.
- No external network in tests (`FakeTransport` / loopback only); live checks stay `#[ignore]`. No secrets or user text in logs, fixtures, issues or error `Display`.
- Frozen raw values (`ProviderType`, `AIOperation`, dates, UUIDs) are unchanged; UI text uses `display_name()` / `english_name()`; language and provider names are never localized.
- Windows is verified only by CI (`windows-latest`); the dev machine is a Mac. Every Windows-only behaviour has a CI test, an `#[ignore = "needs interactive desktop"]` smoke test in `crates/lingopop-platform/tests/windows_smoke.rs` (run by the CI step `Windows desktop tests`), or a line in `docs/testing/manual-checklist.md`. Each task below states what is verified where.
- Task gate (run before every commit): `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`, preceded by `npm ci --prefix app && npm run build --prefix app` (tauri-codegen needs `app/dist`) and, from Task 3 on, `npm test --prefix app && npm run check --prefix app` and `python3 scripts/gen-localizations.py check`.
- Each task appends its new modules to the `AGENTS.md` Layout in the same commit; `CHANGELOG.md` Unreleased is updated in Task 14. Conventional commits, no footers. `cargo test` takes at most one filter.
- Version pins of Phase 4a stay (Dependabot ignores are unchanged).

## IPC contract (all tasks refer to this; names are final)

Commands (all `async`, all return `Result<_, String>` where the `String` is a catalog key, never user text or a key):

| Command | Args | Returns | Task |
|---|---|---|---|
| `ui_bootstrap` | – | `UiBootstrap { language, launchedLanguage, catalog, version, popupFontSize, initialTab }` | 4 |
| `settings_get` | – | `SettingsView` | 4 |
| `settings_update` | `patch: SettingsPatch` | `SettingsView` | 4 |
| `ui_update` | `patch: UiPatch` | `SettingsView` | 4 |
| `open_url` | `url: String` | `()` (allowlist) | 4 |
| `app_relaunch` | – | `()` | 5 |
| `provider_state` | `provider: ProviderType` | `ProviderState` | 7 |
| `provider_validate` | `provider, fields: ProviderFields` (key-free) | `DraftErrors` | 7 |
| `provider_save` | `provider, draft: ProviderDraft` | `ProviderState` | 7 |
| `provider_clear_key` | `provider` | `ProviderState` | 7 |
| `hotkeys_suspend` | `suspended: bool` | `()` | 8 |
| `usage_get` | `period: "day" \| "days7" \| "days30"` | `UsageView` | 9 |
| `usage_reset` | – | `()` | 9 |
| `updates_check` | – | `Option<String>` (newer version or null) | 10 |
| `history_list` | – | `Vec<HistoryRow>` | 11 |
| `history_remove` | `id: String` | `()` | 11 |
| `history_clear` | – | `()` | 11 |
| `clipboard_write` | `text: String` | `()` | 11 |

Events (emitted by the shell): `popup://model` (exists), `settings://changed` (`SettingsView`, after every settings / hotkey-status change), `ui://changed` (`{ popupFontSize, appLanguage }`), `history://changed` (no payload), `usage://changed` (no payload), `settings://tab` (`"about"`, only to an ALREADY OPEN settings window; a cold open gets the tab through `UiBootstrap.initialTab`). Every `Err(String)` of a command and every message-bearing field of a response is a catalog key (plus args where stated) that the frontend renders through `t()`.

Window labels: `popup` (exists), `settings` ("LingoPop Settings", 720x560, not resizable), `history` ("Translation History", 540x480, min 480x360, resizable).

## Review Focus

Five failure modes the spec implies but no single feature test covers; each has a pinning test in the owning task (marked `(RF n)`).

1. **An API key leaking through a side channel** (a `settings://changed` payload built from `ConfigSnapshot`, a `Debug` of a command argument in an error log, a `provider_save` error echoing the draft). Tests: Task 7 `provider_save_response_and_logs_never_contain_key`, `provider_draft_debug_redacts_key`; Task 4 `settings_view_json_has_no_key_material`.
2. **`settings.json` silently gaining or losing fields** when the UI writes it (a Windows-only value slipping in, a lenient-decoded field dropped on re-save). Tests: Task 1 `ui_prefs_live_in_ui_json_only`; Task 4 `settings_update_round_trips_the_12_fields_only`.
3. **The user left without hotkeys** after a re-registration that partly fails, or after recording was started and the Settings window closed without finishing. Tests: Task 8 `apply_registers_the_rest_when_one_conflicts`, `suspend_then_resume_restores_all_registrations`, `settings_window_close_resumes_hotkeys` (shell unit test on the window-event handler through `FakeHotkeyService`).
4. **A tray-triggered operation capturing from LingoPop's own window** (the Settings window is the foreground when the user clicks the tray) or clobbering a foreign clipboard write during the foreground switch. Tests: Task 12 `tray_trigger_restores_remembered_window_before_capture`, `tray_trigger_without_remembered_window_shows_nothing_selected`; `windows_smoke::tracker_ignores_own_process_windows`.
5. **An overlay left on screen or the mouse left captured** when the pick is cancelled by a second hotkey, an error, or an app exit. Tests: Task 13 `second_screen_translate_cancels_first_pick` (orchestrator, `FakePicker` cancel counter), `overlay_pick_returns_none_when_cancel_sender_drops` and `windows_smoke::overlay_destroys_window_on_cancel`.

## Branching and pull requests

Branch `phase4b-windows-ui` from `main`. `main` is PR-only. Recommendation: THREE pull requests from the same branch lineage, each independently green and shippable: **PR A** Tasks 0-3 (pure core modules, localization catalogs; no behaviour change), **PR B** Tasks 4-10 (Settings window and everything it needs), **PR C** Tasks 11-14 (History, tray, tracker, overlay, docs). If the owner prefers one PR, the order is unchanged. Pushing requires the owner's go-ahead at execution time; never merge.

## File Structure

```
i18n/                                   repo-root localization data (Task 3)
  SOURCE.txt                            mac commit the catalogs were generated from
  exclusions.txt                        explicit list of mac keys that are not generated
  generated/{pl,uk,de}.json             generated: { "<english key>": "<translation>" }   (never hand-edited)
  windows.json                          hand-maintained Windows-only keys: { "<key>": { "pl": .., "uk": .., "de": .. } }
scripts/gen-localizations.py            generate | check (Task 3)
scripts/make-tray-icons.swift           tray frames generator, macOS only (Task 11)
crates/lingopop-core/src/
  ui_prefs.rs        UiPrefs (ui.json), AppLanguage, Language, resolve_system   (Task 1)
  updates.rs         version parse / compare, fetch_latest, cache rule           (Task 1)
  provider_draft.rs  ProviderDraft, DraftErrors, validate, apply                 (Task 1)
  usage_pricing.rs   PRICING table, estimated_cost                                (Task 2)
  usage_stats.rs     Period, Totals, totals, totals_by_provider, chart_buckets, format_tokens / format_cost (Task 2)
  i18n.rs            Language catalogs (include_str! of i18n/*.json), lookup, format (Task 3)
  hotkey.rs          + duplicate_of                                               (Task 8)
  paths.rs           + ui_json()                                                  (Task 1)
crates/lingopop-platform/src/
  system.rs          SystemInfo trait (ui_languages, open_url, wait_for_process_exit) (Task 4)
  login_item.rs      LoginItem trait                                              (Task 5)
  last_foreground.rs LastForeground trait                                         (Task 12)
  overlay.rs         RegionOverlay trait + pure overlay geometry helpers           (Task 13)
  fake/{system,login_item,last_foreground,overlay}.rs
  windows/{system,login_item,last_foreground,overlay}.rs
  macos/{system,login_item,last_foreground,overlay}.rs
crates/lingopop-platform/tests/
  foreground_helper.rs       harness = false helper process for the tracker smoke tests (Task 12)
  windows_desktop_input.rs   input-driven overlay tests, NOT in the required CI step (Task 13)
crates/lingopop-orchestrator/src/
  status.rs          TrayStatus, StatusSink                                       (Task 11)
  orchestrator.rs    + TriggerSource, trigger_from, status calls                  (Tasks 11, 12)
app/src-tauri/src/
  windows.rs         open_settings(app, tab), open_history(app), labels, sizes    (Task 4)
  settings_cmds.rs   ui_bootstrap, settings_get, settings_update, ui_update, open_url, app_relaunch (Tasks 4, 5)
  provider_cmds.rs   provider_state / validate / save / clear_key                 (Task 7)
  hotkeys.rs         HotkeyRegistry: apply, suspend (idempotent), statuses, MainThread trait (Task 8)
  usage_cmds.rs      usage_get, usage_reset                                       (Task 9)
  updates.rs         UpdateState: startup check, 3 h timer, updates_check         (Task 10)
  history_cmds.rs    history_list / remove / clear, clipboard_write               (Task 11)
  tray.rs            TrayController: rebuild, set_status, icon frames             (Task 11)
  picker.rs          OverlayPicker: RegionPicker over RegionOverlay (replaces NoPicker) (Task 13)
  state.rs           ShellConfig writers, ShellStores readers + change hooks, UiState (Task 4)
app/src/
  main.ts            route by window label; lazy import of settings / history     (Task 6)
  lib/i18n.ts        catalog lookup behind t(); setLanguage                       (Task 3)
  lib/settingsIpc.ts, lib/settingsTypes.ts                                        (Task 6)
  settings/{Settings,Tabs,MainTab,ProvidersTab,ShortcutsTab,HotkeyRecorder,UsageTab,TokenChart,MagicTab,AboutTab,LanguagePairs}.svelte (Tasks 6-10)
  history/{History,HistoryRow}.svelte, lib/historySearch.ts                       (Task 11)
docs/testing/manual-checklist.md, CHANGELOG.md, AGENTS.md                         (every task / Task 14)
```

---

### Task 0: Branch and design documents

**Files:**
- Modify (already edited in the working tree): `docs/superpowers/specs/2026-10-30-lingopop-app-design.md`
- Add: `docs/superpowers/specs/2026-11-01-phase4b-windows-ui-design.md`, `docs/superpowers/plans/2026-11-01-phase4b-windows-ui.md` (this file)

- [ ] **Step 1: Create the branch**

Run: `git checkout -b phase4b-windows-ui`
Expected: `Switched to a new branch 'phase4b-windows-ui'`.

- [ ] **Step 2: Commit the design and the plan**

```bash
git add docs/superpowers/specs/2026-10-30-lingopop-app-design.md docs/superpowers/specs/2026-11-01-phase4b-windows-ui-design.md docs/superpowers/plans/2026-11-01-phase4b-windows-ui.md
git commit -m "docs: add the Phase 4b design and implementation plan"
```

---

### Task 1: Core: `ui.json` prefs, app language, update-version rules, provider draft validation

**Files:**
- Create: `crates/lingopop-core/src/{ui_prefs.rs, updates.rs, provider_draft.rs}`
- Modify: `crates/lingopop-core/src/{lib.rs, paths.rs}`, `AGENTS.md`
- Test: unit tests in each module (`cargo test -p lingopop-core`)

**Interfaces:**
- Consumes: `fsio::{atomic_write, read_optional, quarantine, strip_bom, is_blank, LoadOutcome, StoreError}`, `Clock`, `Transport`, `HttpRequest::get`, `perform`, `ProviderType`, `ProviderCapabilities::settings_fields`, `ProviderConnectionSettings`.
- Produces:
  - `paths.rs`: `pub fn ui_json(&self) -> PathBuf` (`ui.json`).
  - `ui_prefs.rs`:
    - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "lowercase")] pub enum Language { En, Pl, Uk, De }` with `pub fn code(self) -> &'static str` (`"en"` …), `pub fn native_name(self) -> &'static str` (`"English"`, `"Polski"`, `"Українська"`, `"Deutsch"`), `pub const ALL: [Language; 4]`.
    - `#[derive(..)] #[serde(rename_all = "lowercase")] pub enum AppLanguage { System, En, Pl, Uk, De }` with `pub fn effective(self, system_preferences: &[String]) -> Language` and `pub fn needs_relaunch(self, launched: Language, system_preferences: &[String]) -> bool` (`effective(..) != launched`, port of `Core/AppLanguage.swift:116-120`).
    - `pub fn resolve_system(preferences: &[String]) -> Language`: first preference whose primary subtag (before `-` or `_`, ASCII case-insensitive) is one of `en pl uk de`, else `En` (port of `AppLanguage.resolve`, `:106-109`).
    - `pub const POPUP_FONT_RANGE: RangeInclusive<u32> = 11..=20; pub const DEFAULT_POPUP_FONT_SIZE: u32 = 13;`
    - `#[derive(Debug, Clone, PartialEq)] pub struct UiPrefs { pub popup_font_size: u32, pub app_language: AppLanguage, pub latest_version: Option<String>, pub last_update_check: Option<f64> }` (`Default` = 13 / System / None / None). JSON names: `popupFontSize`, `appLanguage`, `latestVersion`, `lastUpdateCheck` (Unix seconds). Lenient per-field decode like `Settings`; out-of-range font size clamped into `POPUP_FONT_RANGE`; unknown language = `System`; a top-level non-object = corrupt.
    - `pub fn load(path: &Path, clock: &dyn Clock) -> Result<(UiPrefs, LoadOutcome), StoreError>` (missing / blank = defaults + `Missing`; BOM stripped; unparsable = `quarantine` + defaults + `CorruptBackedUp`; never writes), `pub fn save(&self, path: &Path) -> Result<(), StoreError>` (`atomic_write`, not private).
  - `updates.rs`:
    - `pub const RELEASES_API_URL: &str = "https://api.github.com/repos/slucheninov/lingopop-app/releases/latest"; pub const RELEASES_PAGE_URL: &str = "https://github.com/slucheninov/lingopop-app/releases/latest"; pub const CHECK_INTERVAL_SECONDS: f64 = 10_800.0;`
    - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct Version { pub major: u64, pub minor: u64, pub patch: u64, pub prerelease: bool }`; `pub fn parse_version(text: &str) -> Version` (strip one leading `v`; `prerelease` = a `-` is present; take the part before the first `-`, split on `.`, keep the components that parse as integers, pad to three, ignore extras: `Core/UpdateChecker.swift:69-80`).
    - `pub fn is_newer(latest: &str, current: &str) -> bool`: compare `(major, minor, patch)` strictly; when equal, `true` only if `current.prerelease && !latest.prerelease` (the spec's Windows deviation).
    - `pub fn should_check(last_check_unix: Option<f64>, now_unix: f64) -> bool` (`None` or `now - last >= CHECK_INTERVAL_SECONDS`).
    - `pub const UPDATE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);` `pub async fn fetch_latest_version(transport: &dyn Transport) -> Option<String>`: `HttpRequest::get(Url::parse(RELEASES_API_URL).expect("const url"), &[("Accept", "application/vnd.github+json")])` (`get` takes a `url::Url`), 2xx only, JSON `tag_name` through `parse_tag`; anything else `None` (silent). The 10 s limit is enforced in core without tokio: `pub async fn fetch_latest_version_with(transport, deadline: impl Future<Output = ()>) -> Option<String>` races the request against `deadline` (hand-rolled `select` over two pinned futures, no new dependency); the shell passes `tokio::time::sleep(UPDATE_REQUEST_TIMEOUT)`; the plain function passes a never-completing future. `pub fn parse_tag(tag: &str) -> String` strips one leading `v`.
  - `provider_draft.rs`:
    - `#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)] #[serde(rename_all = "camelCase")] pub struct ProviderFields { pub base_url: String, pub model: String, pub referer: String, pub region: String }` (key-free: the only shape `provider_validate` accepts, its `Debug` goes through `errors::debug_url` for `base_url` / `referer`) and `#[derive(Clone, Default, Deserialize)] #[serde(rename_all = "camelCase")] pub struct ProviderDraft { pub api_key: String, #[serde(flatten)] pub fields: ProviderFields }`; hand-written `Debug` prints `api_key: "<redacted>"` and the fields' `Debug`. `validate` / `apply` live on `ProviderFields`; `ProviderDraft` forwards.
    - `#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)] #[serde(rename_all = "camelCase")] pub struct DraftErrors { pub base_url: Option<&'static str>, pub referer: Option<&'static str>, pub region: Option<&'static str> }` + `pub fn is_empty(&self) -> bool`.
    - `ProviderFields::validate(&self, provider: ProviderType) -> DraftErrors`: only fields in `capabilities().settings_fields`; all trimmed; messages exactly `"Base URL must start with https://"` (non-empty and lowercase does not start with `https://`), `"Referer must not contain control characters"` (`char::is_control`), `"Region may only contain letters, digits, '-' and '_'"` (ASCII only: `c.is_ascii_alphanumeric() || c == '-' || c == '_'`; `ProviderDraft.swift:36-56`). The three messages are catalog keys: the frontend renders them through `t()`.
    - `ProviderFields::apply(&self, provider: ProviderType, into: &mut ProviderConnectionSettings)`: trimmed `base_url` / `model` written as is; `referer` / `region` empty = `None`; the key is NOT applied here (the shell writes it to `SecretStore`); `ProviderDraft::trimmed_api_key(&self) -> Option<&str>` (non-empty after trim).

- [ ] **Step 1: Write the failing tests** (unit tests in the three modules)

```rust
// ui_prefs:
// defaults_when_missing                     (load of a missing path -> (UiPrefs::default(), LoadOutcome::Missing))
// clamps_font_size_and_defaults_unknown_language ({"popupFontSize":40,"appLanguage":"fr"} -> 20, System)
// lenient_fields_keep_the_rest              ({"popupFontSize":"big","latestVersion":"0.3.0"} -> font 13, latest Some("0.3.0"))
// corrupt_file_is_quarantined_and_not_written ("[1,2" -> CorruptBackedUp, a *.bak-* sibling exists, the original path is no longer there)
// bom_is_accepted
// save_then_load_round_trips                (all four fields, lastUpdateCheck 1_700_000_000.5)
// ui_prefs_live_in_ui_json_only (RF 2)      (save(ui.json) in a tempdir next to a settings.json: settings.json bytes unchanged)
// resolve_system_picks_first_supported      (["uk-UA","en-US"] -> Uk; ["fr-FR"] -> En; [] -> En; ["PL"] -> Pl; ["de_DE"] -> De)
// needs_relaunch_compares_effective         (System with ["de"] launched De -> false; En launched De -> true)
// updates:
// parse_version_strips_v_and_suffix         ("v1.2.3-beta.4" -> 1.2.3 prerelease; "1.2" -> 1.2.0; "1.x.3" -> 1.3.0 (non-numeric dropped, like compactMap))
// newer_major_minor_patch                   (("0.3.0","0.2.9") true; ("0.2.0","0.2.0") false; ("0.2.1","0.2.1-alpha.5") true)
// prerelease_current_sees_stable            (("0.2.0","0.2.0-alpha.5") true; ("0.2.0-alpha.6","0.2.0-alpha.5") false; ("0.2.0-rc1","0.2.0") false)
// should_check_respects_three_hours         (None -> true; last = now - 10_799 -> false; now - 10_800 -> true)
// fetch_latest_reads_tag_name               (FakeTransport::json(200, r#"{"tag_name":"v0.3.0"}"#) -> Some("0.3.0"); the request has Accept application/vnd.github+json and the exact URL)
// fetch_latest_is_none_on_404_and_network   (FakeTransport::json(404, ..) -> None; FakeTransport::new(|_| Err(Network)) -> None)
// fetch_latest_gives_up_at_deadline         (a FakeTransport handler that never resolves (std::future::pending in an async handler, or a oneshot never sent) + a deadline that completes immediately -> None; no tokio timer, no network)
// provider_draft:
// base_url_must_be_https                    ("http://x" -> Some("Base URL must start with https://"); "  HTTPS://x " -> None; "" -> None)
// referer_rejects_control_chars             (GoogleCloud, "https://a/\n" -> the exact message; Claude ignores referer)
// region_charset                            (MicrosoftTranslator, "east us" -> message; "eastus-2_a" -> None; "ä" and "я" -> message)
// apply_trims_and_nones_empty               (referer "  " -> None, base_url " https://x " -> "https://x")
// provider_draft_debug_redacts_key (RF 1)   (format!("{:?}", draft) contains "<redacted>" and not the key)
// provider_draft_json_flattens_fields       ({"apiKey":"k","baseUrl":"https://x","model":"m","referer":"","region":""} -> fields.base_url "https://x"; the same object without apiKey deserializes as ProviderFields)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-core ui_prefs` (then `updates`, `provider_draft`)
Expected: FAIL to compile (modules missing).

- [ ] **Step 3: Implement** the signatures above; `lib.rs` re-exports `ui_prefs::{AppLanguage, Language, UiPrefs, resolve_system, DEFAULT_POPUP_FONT_SIZE, POPUP_FONT_RANGE}`, `updates::*`, `provider_draft::{ProviderDraft, DraftErrors}`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-core`
Expected: PASS (all existing + 24 new).

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md` Layout bullets for `ui_prefs.rs`, `updates.rs`, `provider_draft.rs`, `paths.rs` (`ui.json`). Run the task gate.
```bash
git add -A && git commit -m "feat(core): add ui.json prefs, app language resolution, update version rules and provider draft validation"
```

Verification: entirely on the Mac.

---

### Task 2: Core: usage pricing, aggregation and formatting

**Files:**
- Create: `crates/lingopop-core/src/{usage_pricing.rs, usage_stats.rs}`
- Modify: `crates/lingopop-core/{Cargo.toml, src/lib.rs}`, `AGENTS.md`

**Interfaces:**
- Consumes: `UsageRecord` (`provider: String` raw, `model`, `input_tokens: i64`, `output_tokens: i64`, `timestamp: Timestamp`), `ProviderType`, `chrono` (`NaiveDateTime`, `FixedOffset`, `Datelike`, `Timelike`, `Weekday`; features `["std"]` only).
- Produces:
  - `usage_pricing.rs`: `pub const PRICING: &[(&str, f64, f64)]` = the mac table copied verbatim from `Core/UsagePricing.swift` in source order (per-1M input, per-1M output); **the mac table has 41 rows, not the 37 the spec quotes: copy all rows as they are** (decision, see report). `pub fn price(model: &str) -> Option<(f64, f64)>` (exact match), `pub fn estimated_cost(model: &str, input_tokens: i64, output_tokens: i64) -> Option<f64>` = `(in*pi + out*po) / 1e6`.
  - `usage_stats.rs`:
    - `pub const TRACKED_PROVIDERS: [ProviderType; 4] = [Claude, OpenAI, Gemini, Grok]`; `pub fn usage_display_name(p: ProviderType) -> &'static str` (`Claude`, `OpenAI`, `Gemini`, `Grok`, `Google Free`, `Google Cloud`, `DeepL`, `Microsoft`).
    - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")] pub enum Period { Day, Days7, Days30 }` with `pub fn cutoff_unix(self, now_unix: f64, utc_offset_seconds: i32) -> f64` = local start of today for `Day`, today minus 6 / 29 days for `Days7` / `Days30` (`Core/UsageStore.swift:167-176`). Every filter and bucket uses `record.timestamp.unix_seconds()` (Apple epoch + 978 307 200), never the raw `Timestamp` value.
    - `#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")] pub struct Totals { pub input_tokens: i64, pub output_tokens: i64, pub calls: u64, pub cost: Option<f64> }` (`cost` = sum over priced records only; `None` when no record is priced: `Core/UsageStore.swift:70-76`).
    - `pub fn totals(records: &[UsageRecord], period: Period, now_unix: f64, utc_offset_seconds: i32) -> Totals`; `pub fn totals_by_provider(..same..) -> BTreeMap<String, Totals>` (keyed by raw provider, only providers with records).
    - `#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")] pub struct Bucket { pub start_unix: f64, pub label: String, pub by_provider: BTreeMap<String, (i64, i64)> }`; `pub fn chart_buckets(records, period, now_unix, utc_offset_seconds, language: Language) -> Vec<Bucket>`: `Day` = 24 hourly slots of the local calendar day of `now`, label `"0"`, `"6"`, `"12"`, `"18"` else `""`; `Days7` = 7 local days ending today, label = localized weekday abbreviation (`Mon`.. / `pon`.. / `пн`.. / `Mo`..: a fixed 4x7 table in this module); `Days30` = 30 days, slot 29 = today (`start_unix` = local start of today), label = day-of-month of the slot's start when `i % 5 == 0` (indices 0, 5, ..., 25; slot 29 is `""`), else `""` (`UsageStore.swift:90-122`, `UI/Settings/TokenBarChartView.swift:12-22`). Day boundaries use the fixed `utc_offset_seconds` (deviation: no DST transitions inside the window; documented).
    - `pub fn format_tokens(n: i64) -> String` (`>= 1000` → one decimal + `K`, else integer) and `pub fn format_cost(cost: f64) -> String` (`>= 0.01` → `$X.XX`; `>= 0.0001` → `$X.XXXX`; `> 0` → `<$0.0001`; else `$0.00`), `UsageFormatting.swift:9-18`.

- [ ] **Step 1: Write the failing tests**

```rust
// pricing_table_has_every_mac_row            (PRICING.len() == 41; price("claude-haiku-5-5") == Some((0.10, 0.50)); price("grok-build-0.1") == Some((1.00, 2.00)))
// estimated_cost_formula                     (("claude-haiku-5-5", 1000, 2000) -> Some(0.0011) within 1e-12; unknown model -> None)
// format_tokens_matches_mac                  (999 -> "999", 1000 -> "1.0K", 1500 -> "1.5K", 1_234_567 -> "1234.6K")
// format_cost_matches_mac                    (0.012 -> "$0.01", 0.0011 -> "$0.0011", 0.00005 -> "<$0.0001", 0.0 -> "$0.00")
// totals_sum_only_priced_costs               (two records, one unknown model -> cost == Some(cost of the known one), calls 2)
// totals_cost_none_when_nothing_priced
// totals_by_provider_omits_absent_providers  (records only for Claude -> map has exactly "Claude")
// period_cutoff_matches_mac                  (now = 2026-11-01T10:30Z, offset +3600: Day -> 2026-10-31T23:00Z; Days7 -> 6 days earlier; Days30 -> 29 days earlier)
// records_filtered_on_unix_seconds           (records built with Timestamp::from_unix_seconds(now - 60) and (cutoff - 1): the first counted, the second not; a record whose raw Apple value equals now_unix (i.e. 31 years in the future if misread) is also counted, which fails if the raw value is compared)
// buckets_day_has_24_hourly_slots_and_labels (labels at 0/6/12/18; a record at local 13:30 lands in slot 13)
// buckets_days7_end_today_with_weekday_labels (utc_offset +7200, now = a known Wednesday -> last label "Wed" for En, "śr" for Pl)
// buckets_days30_label_every_fifth           (30 slots; non-empty labels exactly at indices 0,5,...,25; slot 29 label "" and slot 29 start_unix == local start of today)
// bucket_boundary_uses_offset                (a record at 23:30 UTC with offset +3600 lands on the next local day)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-core usage`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**; add `chrono = { version = "0.4", default-features = false, features = ["std"] }` to core (verify `scripts/check-core-deps.sh` stays green: `chrono` without `clock` pulls no OS crate).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-core usage` then `bash scripts/check-core-deps.sh`
Expected: PASS (13 tests); `exit=0`.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md` Layout bullets for both modules (state the 41-row table and the fixed-offset deviation).
```bash
git add -A && git commit -m "feat(core): add usage pricing table, aggregation buckets and mac usage formatting"
```

Verification: Mac.

---

### Task 3: Localization: generator script, catalogs, Rust and frontend lookup

**Files:**
- Create: `scripts/gen-localizations.py`, `i18n/{SOURCE.txt, exclusions.txt, windows.json, generated/pl.json, generated/uk.json, generated/de.json}`, `crates/lingopop-core/src/i18n.rs`, `app/src/lib/i18n.test.ts`, `app/src/lib/catalogCoverage.test.ts`
- Modify: `app/src/lib/i18n.ts`, `app/vite.config.ts` (`server.fs.allow: ['..']`), `app/tsconfig.json` (`resolveJsonModule` if not set), `.github/workflows/ci.yml`, `crates/lingopop-orchestrator/tests/view_model.rs` (coverage test), `AGENTS.md`

**Interfaces:**
- Consumes: the mac `App/Localizable.xcstrings` at `$LINGOPOP_MAC_REPO` (default `~/Works/slava/lingo-pop`), `scripts/check-localizations.py` rules; core `Language`.
- Produces:
  - `scripts/gen-localizations.py generate` (macOS/dev only: reads the mac catalog, writes `i18n/generated/{pl,uk,de}.json` sorted by key with 2-space indent and `ensure_ascii=False`, writes `i18n/SOURCE.txt` = `git -C <mac repo> rev-parse HEAD`); `check` (no mac repo needed, CI): (a) every key in each generated file and in `windows.json` passes the `check-localizations.py` signature rule against its key (specifier multiset of `%(?:\d+\$)?(?:lld|ld|d|@|%)`, `\n` count, counts of `…`, `→`, `{{targetLanguage}}`; `⌘` is additionally forbidden in every Windows value), (b) `windows.json` has all three languages for every key, (c) no key appears both in `windows.json` and a generated file, (d) no excluded key appears in a generated file; exit 1 with one `KEY | lang | problem` line per issue; on success `check` prints one line `ok: <n> generated keys, <m> windows keys, <x> excluded`. Keys with `shouldTranslate: false` are skipped on generation. `i18n/exclusions.txt`: one key per line (`\n` inside a key written as the two characters `\n`), exactly these 26 mac keys (the design doc says 18; the count was taken from the real catalog):
    `Accessibility` · `Accessibility access required` · `After granting access, fully quit and relaunch LingoPop.` · `Click a shortcut field, then press a key combination. Use any of ⌃ ⌥ ⇧ ⌘ plus a key. Esc cancels recording. ✕ removes the shortcut.` · `Could not capture the screen. Check that Screen Recording permission is granted.` · `Couldn't switch back to the original app, so nothing was replaced. The text is on your clipboard; paste it manually with ⌘V.` · `Granted` · `Hide LingoPop` · `LingoPop can't read selected text yet.` · `Open Settings` · `Quit LingoPop from the menu bar and open it again to apply the new language.` · `Re-check` · `Request` · `Required for Screen Translate.` · `Screen Recording` · `Screen Recording access required` · `Steps:\n1. Open System Settings → Privacy & Security → %@\n2. Make sure LingoPop is in the list and the toggle is ON\n3. Quit LingoPop completely (menu bar → Quit) and relaunch\n\nmacOS only reads the new permission after a full restart of the app.` · `Select text anywhere on your Mac, press a shortcut, and get an instant AI translation, grammar fix, or rewrite in a popup near your cursor.` · `Supports Claude, OpenAI, Gemini, Grok, Google Translate, DeepL, and Microsoft Translator. API keys and provider settings are stored locally in an encrypted file. Usage statistics, and optionally translation history, can sync between Macs via iCloud Drive.` · `Sync translation history to iCloud` · `Sync usage statistics via iCloud Drive` · `Syncs usage statistics and, if enabled, translation history between Macs signed into the same Apple ID. Settings, provider settings and API keys stay local on each Mac.` · `iCloud Drive available` · `iCloud Drive sync` · `iCloud Drive unavailable` · `⚠ Re-check Accessibility…`.
    The generator fails when an excluded key is absent from the mac catalog (a stale exclusion).
  - `i18n/windows.json` initial keys (English key = text; pl / uk / de written by hand): the existing `KEY_INPUT_BLOCKED`, `KEY_FOREGROUND_LOST`, `KEY_OCR_UNAVAILABLE`, `KEY_PLATFORM_GENERIC`, `KEY_SCREEN_TIMEOUT` texts; the loading labels and `Close` already used by the popup `t()` that are not mac keys; `Click a shortcut field, then press a key combination. Use Ctrl, Alt or Shift plus a key. Esc cancels recording. ✕ removes the shortcut.`; `Stored locally, encrypted with Windows Data Protection for your user account.`; `This shortcut is already used by another app.`; `This shortcut can't be registered.`; `Already used by %@.`; `Select text in any app, press a shortcut, and get an instant AI translation, grammar fix, or rewrite in a popup near your cursor.`; `Supports Claude, OpenAI, Gemini, Grok, Google Translate, DeepL, and Microsoft Translator. API keys and provider settings are stored locally, encrypted for your Windows account.`; `LingoPop Settings`; `Translation History`; `Couldn't save the API key. Check that the data folder is writable.`; `Provider settings can't be saved because secrets.dat is unreadable. Remove or rename it and enter your keys again.`; `Nothing to translate.` is NOT added (not needed). Later tasks add keys here as they introduce UI text.
  - `crates/lingopop-core/src/i18n.rs`: `pub fn lookup<'a>(language: Language, key: &'a str) -> Cow<'a, str>` (En = `Borrowed(key)`; otherwise the generated catalog, then `windows.json`, then `Borrowed(key)`), `pub fn format(template: &str, args: &[&str]) -> String` (ordered `%@` / `%lld` and positional `%1$@`; a missing arg leaves the placeholder; `%%` → `%`), `pub fn t(language: Language, key: &str, args: &[&str]) -> String`, `pub fn catalog(language: Language) -> &'static HashMap<String, String>` (merged generated + windows entries, empty for En; the shell ships it in `ui_bootstrap`). Catalogs embedded with `include_str!("../../../i18n/generated/pl.json")` etc. and parsed once per language into `OnceLock<HashMap<String, String>>` with OWNED strings (serde cannot borrow a JSON string that contains escapes such as `\n` or `\"`, and most catalog values contain them).
  - `app/src/lib/i18n.ts`: same `t(key, args)` signature as today; add `export function setLanguage(code: 'en' | 'pl' | 'uk' | 'de', catalog: Record<string, string>): void` and `export function currentLanguage(): string`; lookup = catalog then key; substitution unchanged. The catalog is delivered by `ui_bootstrap` (Task 4), not imported into the bundle (keeps the popup bundle small).
  - CI: a step `Check localization catalogs` → `python3 scripts/gen-localizations.py check` before `Test frontend` in `.github/workflows/ci.yml`, preceded by `actions/setup-python@v5` with `python-version: "3.12"` on both legs (`python3` is not a reliable command name on `windows-latest` without it).

- [ ] **Step 1: Write the failing tests**

```rust
// core i18n.rs unit tests:
// english_is_pass_through                   (lookup(En, "Quit LingoPop") == "Quit LingoPop")
// polish_lookup_from_generated              (lookup(Pl, "Quit LingoPop") == the value in i18n/generated/pl.json, read by the test from the same include_str)
// windows_only_key_is_looked_up             (lookup(De, KEY_INPUT_BLOCKED text) != key)
// escaped_key_and_value_round_trip          (windows.json carries the entry "Line one\nLine \"two\"" (a Windows-only key reserved for this test, with pl/uk/de values that also contain `\n` and `"`): lookup(Pl, "Line one\nLine \"two\"") returns the Polish value with its newline and quotes intact, not the key)
// unknown_key_falls_back_to_key             (and the returned Cow borrows the caller's key)
// format_ordered_positional_and_percent     (format("Text is %@ characters, max is %@.", ["3","2"]); "%2$@ %1$@"; "100%%" -> "100%")
// orchestrator tests/view_model.rs addition:
// every_message_key_has_all_translations    (for each KEY_* const in messages.rs and each Language in [Pl, Uk, De]: lookup != key)
```
```ts
// app/src/lib/i18n.test.ts:
// t_uses_catalog_after_setLanguage           (setLanguage('pl', {'Copy': 'Kopiuj'}); t('Copy') === 'Kopiuj'; t('Missing') === 'Missing')
// t_keeps_substitution_rules                 (the Phase 4a cases still pass)
// app/src/lib/catalogCoverage.test.ts:
// every_t_literal_in_src_has_a_translation   (regex over src/**/*.{svelte,ts} for t('...') / t("...") literals; each key is in i18n/generated/pl.json, uk, de or i18n/windows.json; read with node fs relative to import.meta.url)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-core i18n` and `npm test --prefix app`
Expected: FAIL (module / files missing; the coverage test fails on the popup's Windows-only literals until `windows.json` exists).

- [ ] **Step 3: Implement**

Run `python3 scripts/gen-localizations.py generate` once (mac repo present on the dev Mac), commit the output; write `windows.json`; implement `i18n.rs`, `i18n.ts`, the CI step.

- [ ] **Step 4: Run to verify pass**

Run: `python3 scripts/gen-localizations.py check && cargo test -p lingopop-core i18n && cargo test -p lingopop-orchestrator --test view_model && npm test --prefix app`
Expected: `check` prints nothing and exits 0; all PASS.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md`: Layout bullets for `i18n/`, the script, `i18n.rs`; "Mac fixtures" section gets a sibling "Localization catalogs" paragraph (`generate` only when the mac catalog changes, `check` in CI). Run the task gate.
```bash
git add -A && git commit -m "feat(i18n): generate pl/uk/de catalogs from the mac xcstrings and add Rust and frontend lookup"
```

Verification: Mac (generation + checks); CI runs `check` on both legs.

---

### Task 4: Shell foundation: windows, UI state, settings read / write commands, events, `open_url`, second instance

**Files:**
- Create: `app/src-tauri/src/{windows.rs, settings_cmds.rs}`, `crates/lingopop-platform/src/{system.rs, fake/system.rs, windows/system.rs, macos/system.rs}`
- Modify: `app/src-tauri/src/{lib.rs, state.rs, commands.rs}`, `app/src-tauri/capabilities/default.json`, `crates/lingopop-platform/src/{lib.rs, native.rs, fake/mod.rs, windows/mod.rs, macos/mod.rs}`, `crates/lingopop-platform/Cargo.toml` (`windows` features `Win32_UI_Shell`, `Win32_Globalization`; `objc2-foundation` feature `NSLocale`), every `Platform { .. }` literal (`crates/lingopop-orchestrator/src/orchestrator.rs` `test_platform`, `tests/orchestrator_text.rs`, `tests/orchestrator_screen.rs`, `tests/orchestrator_actions.rs`, both `native()` bundles), `AGENTS.md`. The same list applies to every later task that adds a `Platform` field (5, 12, 13).

**Interfaces:**
- Consumes: Task 1 (`UiPrefs`, `AppLanguage`, `Language`, `resolve_system`), Task 3 (`i18n::lookup`), core `Settings` (12 fields, `save`, `set_fallback`, `set_hotkey`), `HistoryStore::{entries, remove, clear, trim}`, `UsageStore::{records, reset}`.
- Produces:
  - Platform `system.rs`: `pub trait SystemInfo: Send + Sync { fn ui_languages(&self) -> Vec<String>; fn open_url(&self, url: &str) -> Result<(), PlatformError>; fn wait_for_process_exit(&self, pid: u32, timeout: Duration) -> bool; }`. Windows: `GetUserPreferredUILanguages(MUI_LANGUAGE_NAME)` (Win32, no WinRT activation; the double-NUL list split into tags), `ShellExecuteW(NULL, "open", url, NULL, NULL, SW_SHOWNORMAL)` (result `<= 32` → `Os { api: "ShellExecuteW", code }`), `OpenProcess(SYNCHRONIZE)` + `WaitForSingleObject` (a missing process = `true`). macOS dev: `NSLocale::preferredLanguages`, `Command::new("open").arg(url)` (no shell), poll `Command::new("kill").args(["-0", pid])` every 100 ms. `FakeSystem::new(languages: Vec<String>)`, `opened() -> Vec<String>`, `set_process_exits(bool)`. `Platform` gains `pub system: Arc<dyn SystemInfo>`.
  - `state.rs`:
    - `ShellConfig` adds `pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> Result<Settings, SettingsError>` (mutate under the lock, `save` to `settings_json`, on error restore the previous value and return the error), `pub fn set_api_key(&self, provider, key: &str) -> Result<(), SecretsWriteError>`, `pub fn set_connection(&self, provider, settings: ProviderConnectionSettings) -> Result<(), SecretsWriteError>` (READ-MODIFY-WRITE: `SecretStore::set_provider_settings` replaces the whole map (`secrets.rs:78-80`), so take the `Loaded` map, or `default_providers()` when `Absent`, replace the one provider, write the whole map back; `Corrupt` → `SecretsWriteError::Corrupt`), `pub fn has_api_key(&self, provider) -> bool`, `pub fn connection(&self, provider) -> ProviderConnectionSettings`, `pub fn secrets_available(&self) -> bool`, `pub fn connections_corrupt(&self) -> bool`. `pub enum SecretsWriteError { Unavailable, Corrupt, Store(SecretsErrorKind) }` where `SecretsErrorKind { Io(io::ErrorKind), Protect, Encode }` — the serde message of `SecretsError::Encode` is never carried, logged or shown (it may quote entry text); `Display` is fixed text.
    - `ShellStores` adds `history_entries() -> Vec<HistoryEntry>`, `remove_history(id: Uuid)`, `clear_history()`, `trim_history(max: u32)`, `usage_records() -> Vec<UsageRecord>`, `reset_usage()` and ONE `pub fn set_on_change(&self, f: Arc<dyn Fn(StoreKind) + Send + Sync>)` with `pub enum StoreKind { History, Usage }`, installed once in `setup` (Task 4 installs the emitter of `history://changed` / `usage://changed`; Task 11 extends the same closure with the debounced tray rebuild). Every mutating method (including the existing `add_history` / `append_usage`) clones the hook `Arc` under the store lock, DROPS the guard, then calls it, so a hook may call `history_entries()`; the hook only signals (emit / channel send), never does I/O.
    - `pub struct UiState { prefs: Mutex<UiPrefs>, path: PathBuf, pub launched: Language, pending_tab: Mutex<Option<String>> }` with `prefs() -> UiPrefs`, `update(f) -> Result<UiPrefs, StoreError>` (save after mutate), `pub fn language(&self) -> Language` (= `launched`, fixed for the process), `take_pending_tab() -> Option<String>`. `state::load` returns it as a third value, reading `ui.json`, computing `launched = prefs.app_language.effective(&platform.system.ui_languages())`.
  - `windows.rs`: `pub const SETTINGS_LABEL: &str = "settings"; pub const HISTORY_LABEL: &str = "history";` `pub fn open_settings(app: &AppHandle, tab: Option<&str>)` and `pub fn open_history(app: &AppHandle)` RETURN IMMEDIATELY: they store the requested tab in `UiState.pending_tab` (settings only) and `tauri::async_runtime::spawn(open_window(app, label))`. `async fn open_window(app, label)`: a per-label `tokio::sync::Mutex<()>` in `ShellState.window_locks` (held across the `.await` of creation, so two rapid requests build one window); if the window exists → `set_focus()` and, for settings with a pending tab, `emit_to(SETTINGS_LABEL, "settings://tab", tab)` (the window is live, the listener registered after `ui_bootstrap`); else build it (`WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))` with title `t(lang, "LingoPop Settings")`, `inner_size(720.0, 560.0)`, `resizable(false)`, `center()`; history: `t(lang, "Translation History")`, `inner_size(540.0, 480.0)`, `min_inner_size(480.0, 360.0)`, `resizable(true)`) — the pending tab is delivered by `ui_bootstrap` (`UiBootstrap.initialTab`, `take_pending_tab()`), never by an event a not-yet-listening webview would miss. `WebviewWindowBuilder::build` runs ONLY inside this spawned task (never in the single-instance, tray or window-event callbacks: tauri 2.12.5 `webview/webview_window.rs:55-58` deadlock). Windows are destroyed on close (Tauri default); `on_window_event` `Destroyed` of `settings` spawns `hotkeys.suspend(app, false)` (Task 8). Capabilities: `windows: ["popup", "settings", "history"]`, permissions unchanged (`core:event:default`).
  - `settings_cmds.rs` (all `async`, registered in `generate_handler!`):
    - `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct SettingsView { active_provider: ProviderType, fallbacks: BTreeMap<ProviderType, ProviderType>, target_language: String, hotkeys: BTreeMap<AIOperation, HotkeyView { key, modifiers, display }>, hotkey_status: BTreeMap<AIOperation, HotkeyStatus>, launch_at_login: bool, max_input_chars: u32, max_history_count: u32, menu_history_count: u32, use_auto_detect: bool, language_pairs: BTreeMap<String, String>, auto_copy_ocr: bool (serde `autoCopyOCR`), magic_prompt: String, quick_languages: Vec<&'static str>, popup_font_size: u32, app_language: AppLanguage, launched_language: Language, needs_relaunch: bool, latest_version: Option<String>, providers: Vec<ProviderSummary { provider, name, has_key, requires_key, fallback: Option<ProviderType> }> }`; `pub fn settings_view(state: &ShellState) -> SettingsView` (pure over the state; `HotkeyStatus` is `#[serde(rename_all = "lowercase")] enum { Registered, Conflict, Invalid, None }`, filled from Task 8's registry, `None` until then).
    - `#[derive(Deserialize)] #[serde(tag = "field", content = "value", rename_all = "camelCase")] pub enum SettingsPatch { TargetLanguage(String), UseAutoDetect(bool), LanguagePairs(BTreeMap<String, String>), MaxInputChars(u32), MaxHistoryCount(u32), MenuHistoryCount(u32), LaunchAtLogin(bool), AutoCopyOcr(bool) (tag `autoCopyOCR`), MagicPrompt(String), ActiveProvider(ProviderType), Fallback { provider: ProviderType, fallback: Option<ProviderType> }, Hotkey { operation: AIOperation, hotkey: Option<Hotkey> } }` with a hand-written `Debug` that prints `MagicPrompt(<N chars>)` and `LanguagePairs(<N pairs>)` (user text never reaches a log); `#[derive(Deserialize, Debug)] #[serde(tag = "field", content = "value", rename_all = "camelCase")] pub enum UiPatch { PopupFontSize(u32), AppLanguage(AppLanguage) }`.
    - `settings_update`: applies the patch through `update_settings` (`MagicPrompt` passes `sanitize_magic_prompt`; `MaxHistoryCount` additionally `trim_history(n)`; `LaunchAtLogin` and `Hotkey` call hooks filled by Tasks 5 / 8: `pub trait SettingsHooks { fn launch_at_login(&self, bool); fn hotkeys(&self, &BTreeMap<AIOperation, Hotkey>); }` held in `ShellState.hooks: Arc<dyn SettingsHooks>`, a no-op impl until then), then emits `settings://changed` with the fresh view and returns it; on a save error returns `Err("Couldn't save settings.")` (new `windows.json` key) and logs `settings save failed: <io kind>`.
    - `ui_update`: `PopupFontSize` clamped into `POPUP_FONT_RANGE`; after save emits `ui://changed { popupFontSize, appLanguage }` to all windows and returns the view.
    - `ui_bootstrap(window: tauri::Window) -> UiBootstrap { language: &'static str, launched_language, catalog: &'static HashMap<String, String> (`i18n::catalog(lang)`), version: lingopop_core::VERSION, popup_font_size, initial_tab: Option<String> (`take_pending_tab()` when `window.label() == "settings"`, else `None`) }`.
    - `open_url(url)`: `pub fn url_allowed(url: &str) -> bool` (pure: exactly `RELEASES_PAGE_URL`, or a prefix `https://github.com/slucheninov/lingopop-app/blob/main/` with no `..`, whitespace or control chars) then `platform.system.open_url`; a disallowed URL returns `Err("Link not allowed.")` and logs the fixed line `open_url refused` (no URL).
  - `lib.rs`: the single-instance callback becomes `|app, _args, _cwd| windows::open_settings(app, None)` (which only spawns); `ShellState` gains `config`, `stores`, `ui`, `platform`, `hooks`, `window_locks`.

- [ ] **Step 1: Write the failing tests**

`app/src-tauri` unit tests (`cargo test -p lingopop-app`, no `tauri::test`; `ShellState`-free helpers take the structs directly):
```rust
// settings_patch_json_shape                       ({"field":"fallback","value":{"provider":"Claude","fallback":null}}, {"field":"hotkey","value":{"operation":"translate","hotkey":{"key":"KeyT","modifiers":["ctrl","alt"]}}}, {"field":"autoCopyOCR","value":false})
// ui_patch_json_shape                             ({"field":"popupFontSize","value":16} -> PopupFontSize(16); {"field":"appLanguage","value":"uk"} -> AppLanguage(Uk); {"field":"appLanguage","value":"system"} -> System)
// settings_patch_debug_hides_user_text            (format!("{:?}", MagicPrompt("secret words".into())) == "MagicPrompt(12 chars)")
// settings_update_round_trips_the_12_fields_only (RF 2) (ShellConfig over a tempdir: apply TargetLanguage("German"); re-read settings.json as serde_json::Value: exactly the 12 wire keys, targetLanguage == "German", magicPrompt unchanged)
// update_settings_restores_previous_on_save_error (path in a read-only dir -> Err, settings() == before)
// set_connection_refuses_when_corrupt             (SecretStore over a fake protector whose providerSettings.v1 is garbage -> Err(Corrupt); the file bytes are unchanged)
// set_connection_keeps_other_providers            (set Claude base_url "https://c"; then set OpenAI model "m": connection(Claude).base_url still "https://c"; the file decodes to a Loaded map with both)
// secrets_write_error_display_is_fixed_text       (Store(Encode) displays "secrets could not be encoded", no serde text)
// settings_view_json_has_no_key_material (RF 1)   (config with key "sk-SECRET" for Claude; serde_json::to_string(settings_view) does not contain "SECRET"; providers[Claude].hasKey == true)
// url_allowed_rules                               (RELEASES_PAGE_URL true; ".../blob/main/docs/api-keys.md#claude" true; "https://evil.com" false; ".../blob/main/../x" false; "file:///C:/x" false)
// stores_change_hook_fires_on_every_mutation      (ShellStores over a tempdir, hook counter == 4 after add / remove / trim / clear, and 2 after append_usage / reset_usage)
// stores_change_hook_may_read_the_store           (a hook that calls stores.history_entries() inside the callback returns without deadlock and sees the new entry)
// load_computes_launched_language_from_system     (FakeSystem(["uk-UA"]) + ui.json appLanguage system -> UiState.launched == Uk; appLanguage "de" -> De)
// pending_tab_is_taken_once                       (open_settings stores "about"; take_pending_tab() == Some("about") then None)
```
Platform (`crates/lingopop-platform/tests/system.rs`, runs everywhere): `fake_system_records_opened_urls`. Windows smoke (`#[ignore = "needs interactive desktop"]`): `ui_languages_non_empty_on_desktop`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-app`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** the signatures above; update the orchestrator test rigs' `Platform { .. }` literals with `system: FakeSystem::new(vec![])`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-app && cargo test -p lingopop-platform && cargo test -p lingopop-orchestrator`
Expected: PASS.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md`: Layout bullets for `system.rs` (+ fakes / impls), `windows.rs`, `settings_cmds.rs`, the `state.rs` changes, the IPC contract summary and the window labels; `lib.rs` bullet: second launch opens Settings. Run the task gate; push and check `gh pr checks` (Windows compiles `windows/system.rs`).
```bash
git add -A && git commit -m "feat(app): add settings and history windows, settings read/write commands, ui.json state and events"
```

Verification: Mac for everything except `windows/system.rs` (CI compile + smoke `ui_languages_non_empty_on_desktop`).

---

### Task 5: Launch at login (`HKCU\...\Run`) and Relaunch

**Files:**
- Create: `crates/lingopop-platform/src/{login_item.rs, fake/login_item.rs, windows/login_item.rs, macos/login_item.rs}`
- Modify: `crates/lingopop-platform/{Cargo.toml (Win32_System_Registry), src/lib.rs, src/native.rs, src/fake/mod.rs}`, `crates/lingopop-platform/tests/windows_smoke.rs`, `app/src-tauri/src/{lib.rs, settings_cmds.rs, state.rs}`, orchestrator test rigs (`login_item`), `AGENTS.md`

**Interfaces:**
- Consumes: Task 4 (`SettingsHooks`, `SystemInfo::wait_for_process_exit`, `settings_update`), core `Settings.launch_at_login`.
- Produces:
  - `login_item.rs`: `pub trait LoginItem: Send + Sync { fn set_enabled(&self, enabled: bool, exe: &Path) -> Result<(), PlatformError>; }`; pure `pub fn run_value(exe: &Path) -> String` = `"\"<path>\""` (quoted). Windows `WindowsLoginItem::new()` / `#[doc(hidden)] pub fn with_subkey(subkey: &str)` (default `Software\Microsoft\Windows\CurrentVersion\Run`, value name `LingoPop`): enabled = `RegSetValueExW(REG_SZ, run_value)` (`RegCreateKeyExW`), disabled = `RegDeleteValueW` (`ERROR_FILE_NOT_FOUND` is success); errors `Os { api: "RegSetValueExW" | "RegDeleteValueW" | "RegCreateKeyExW", code }`. macOS dev: `MacLoginItem` no-op `Ok(())`. `FakeLoginItem::new()`, `calls() -> Vec<(bool, PathBuf)>`, `fail_next()`. `Platform` gains `pub login_item: Arc<dyn LoginItem>`.
  - Shell: `ShellHooks: SettingsHooks` (in `lib.rs` or `state.rs`) whose `launch_at_login(enabled)` calls `platform.login_item.set_enabled(enabled, &std::env::current_exe()?)` on a `spawn_blocking`, logging `login item update failed: <PlatformError>` on error (no UI, the toggle stays). `setup` applies it once at startup with the loaded `settings.launch_at_login` (`AppDelegate.swift:76-80`).
  - `app_relaunch` command: spawn `std::env::current_exe()` with args `["--wait-for-pid", <current pid>]` detached (`Command::spawn`; on Windows `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS` via `std::os::windows::process::CommandExt::creation_flags`), then `app.exit(0)`; spawn failure → `Err("Could not relaunch LingoPop")` (mac key). `run()` parses `--wait-for-pid <pid>` BEFORE the Tauri builder: `platform.system.wait_for_process_exit(pid, 10 s)`; `false` → log `previous instance still running` and continue (single-instance then exits us, like the mac helper giving up). Pure `pub fn parse_wait_pid(args: &[String]) -> Option<u32>`.

- [ ] **Step 1: Write the failing tests**

```rust
// platform tests/login_item.rs (every OS): run_value_quotes_the_path ("C:\\Program Files\\LingoPop\\lingopop.exe" -> "\"C:\\Program Files\\LingoPop\\lingopop.exe\""); fake_login_item_records_calls
// windows_smoke.rs, run by DEFAULT (no desktop needed, HKCU only), subkey "Software\\LingoPopTest\\<test name>-<pid>" (unique per test, so parallel tests never share a key):
//   login_item_sets_quoted_value_and_removes_it   (set true -> RegQueryValueExW == run_value; set false -> ERROR_FILE_NOT_FOUND; the test deletes the test key at the end and never touches the real Run key)
//   login_item_disable_when_absent_is_ok
// app: parse_wait_pid_reads_flag (["lingopop","--wait-for-pid","4242"] -> Some(4242); [] -> None; bad number -> None)
// app: startup_applies_launch_at_login_once      (ShellHooks over FakeLoginItem + Settings { launch_at_login: true } -> calls() == [(true, exe)])
// app: toggle_calls_login_item_and_keeps_setting_on_error (FakeLoginItem.fail_next(); settings_update LaunchAtLogin(true) -> Ok, settings().launch_at_login == true)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-platform --test login_item && cargo test -p lingopop-app`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** as above.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-platform && cargo test -p lingopop-app`; push; `gh pr checks` green (the registry tests run in the default Windows `cargo test`).

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md`: `login_item.rs` bullets, the `--wait-for-pid` flag in the `lib.rs` bullet. Manual checklist line (Task 14 collects them): "Enable Launch at login, sign out and in: LingoPop starts; disable: it does not; move the exe and start it: the Run value is rewritten to the new path."
```bash
git add -A && git commit -m "feat(platform,app): add launch at login through HKCU Run and the relaunch helper flag"
```

Verification: pure tests on the Mac; registry tests in Windows CI (default run); sign-in behaviour manual.

---

### Task 6: Frontend: window routing, Settings frame, Main tab, popup font size

**Files:**
- Create: `app/src/settings/{Settings.svelte, Tabs.svelte, MainTab.svelte, LanguagePairs.svelte, Relaunch.svelte}`, `app/src/lib/{settingsIpc.ts, settingsTypes.ts, bootstrap.ts}`, `app/src/settings/settings.test.ts`, `app/src/settings/mainTab.test.ts`
- Modify: `app/src/main.ts`, `app/src/popup/{Popup.svelte, ResultView.svelte, OcrView.svelte}`, `app/src/lib/actionRow.ts` (`panelWidth(model, fontSize)`), `app/src/lib/icons.ts` (tab icons: `sliders`, `cpu`, `command`, `chart`, `sparkles`, `info`), `app/src/popup/popup.test.ts`, `i18n/windows.json`, `AGENTS.md`

**Interfaces:**
- Consumes: Task 3 `setLanguage`, Task 4 commands `ui_bootstrap`, `settings_get`, `settings_update`, `ui_update`, `app_relaunch`, events `settings://changed`, `ui://changed`, `settings://tab`.
- Produces:
  - `main.ts`: `const label = getCurrentWindow().label` (`@tauri-apps/api/window`); `popup` mounts `Popup` immediately (bundle unchanged); `settings` / `history` `await import('./settings/Settings.svelte')` / `'./history/History.svelte'` after `bootstrap()`. Every lazily imported module that a LATER task fills (`history/History.svelte` in Task 11; `settings/{ProvidersTab,ShortcutsTab,UsageTab,MagicTab,AboutTab}.svelte` in Tasks 7-10) is created in THIS task as a placeholder component rendering an empty `<section>` (no text, no invoke), so `npm run build` / `check` stay green at every intermediate commit; each later task replaces its placeholder. `lib/bootstrap.ts`: `export async function bootstrap(): Promise<UiBootstrap>` (invokes `ui_bootstrap`, calls `setLanguage(language, catalog)`, sets `document.documentElement.lang`; on rejection logs nothing with user text, leaves English and returns `FALLBACK_BOOTSTRAP` (`en`, empty catalog, `popupFontSize` 13, `initialTab` null)). The popup calls `bootstrap()` before `popup_ready` (so popup strings are localized; a failed bootstrap still reaches `popup_ready`) and listens to `ui://changed` for `popupFontSize`. `Settings.svelte` opens on `initialTab` when set.
  - `settingsTypes.ts`: TS mirrors of `SettingsView`, `SettingsPatch`, `UiPatch`, `UiBootstrap`, `HotkeyStatus`, `ProviderSummary` (exact camelCase names from Task 4). `settingsIpc.ts`: `getSettings()`, `updateSetting(patch)`, `updateUi(patch)`, `relaunch()`, `openUrl(url)`, each a typed `invoke`.
  - `Settings.svelte`: tab bar in mac order with icons and `t()` titles `Main`, `Providers`, `Shortcuts`, `Usage`, `Magic`, `About` (mac keys); active tab state defaults to `main`; `settings://tab` event selects a tab; each tab body is a lazy `{#await import(..)}`; the `SettingsView` is loaded once and refreshed on `settings://changed`; prop `view` + callback `onPatch(patch)` to tabs.
  - `MainTab.svelte` (`MainSettingsTab.swift:32-107`, strings are mac keys): sections `Target language` (toggle `Auto-detect input language` → `useAutoDetect`; select labelled `Default target` when auto-detect else `Target`, options `quickLanguages` + current if missing → `targetLanguage`; `LanguagePairs` when auto-detect on), `Popup` (`Result font size` range 11..20 step 1 + `%lld pt` label → `ui_update popupFontSize`), `Language` (select `System` (t) / native names from a fixed list → `ui_update appLanguage`; `Relaunch.svelte` shows `Restart LingoPop to apply the new language.` + button `Relaunch` when `view.needsRelaunch`; a failed `app_relaunch` shows the returned key through `t()`), `Limits` (`Max input characters` number ≥ 0 → `maxInputChars` on change; `History size` 0..100 → `maxHistoryCount`; `Display` 0..100 → `menuHistoryCount`), `Startup` (`Launch at login`, `Auto-copy OCR/translation results to clipboard`). Every control sends one `settings_update` with its field immediately on change (no Save).
  - `LanguagePairs.svelte` (`:193-237`): rows sorted by key `from → to` with a remove button; add row with two selects over `quickLanguages`, defaults `Russian` → `Ukrainian`, add button; footnote `If detected source has no pair, the default target is used.`; every change sends `LanguagePairs(whole map)`.
  - Popup font size: `Popup.svelte` keeps `fontSize` state (bootstrap value, updated by `ui://changed`), sets CSS variable `--result-font-size: <n>px` on `<main>`; `ResultView` / `OcrView` text uses it; `panelWidth(model, measure, fontSize = 13)` keeps its existing `measure` parameter and adds `fontSize` as the third: the text-width term becomes `widthBase + scalars * (fontSize / 13)` (`UI/PopupLayout.swift:57-69`: `fontFactor = fontSize / referenceFontSize`, applied to the per-character term only; the 320..700 clamp and the row-width term are unchanged), and the row labels are measured at the same font size.

- [ ] **Step 1: Write the failing tests** (Vitest, `invoke` / `listen` / `getCurrentWindow` mocked)

```ts
// settings.test.ts:
// tabs_render_in_mac_order_and_default_main      (['Main','Providers','Shortcuts','Usage','Magic','About']; Main panel visible)
// settings_tab_event_selects_about               (fire 'settings://tab' with 'about' -> About panel visible)
// bootstrap_sets_language_before_mount           (invoke('ui_bootstrap') resolves pl catalog -> tab title t('Main') uses the catalog)
// bootstrap_initial_tab_opens_about              (ui_bootstrap resolves { initialTab: 'about' } -> About panel visible without any event)
// placeholder_tabs_mount_without_invoke          (Providers..About placeholders render an empty section and call no invoke)
// mainTab.test.ts:
// auto_detect_toggle_sends_patch                 (invoke called with 'settings_update', { patch: { field: 'useAutoDetect', value: true } })
// target_select_includes_current_when_not_quick  (targetLanguage 'Latin' appears first in options)
// font_slider_sends_ui_patch_and_label           ({ field: 'popupFontSize', value: 16 }; label '16 pt')
// language_select_shows_relaunch_when_needed     (view.needsRelaunch true -> text and button; click -> invoke('app_relaunch'))
// relaunch_failure_shows_message                 (invoke rejects 'Could not relaunch LingoPop' -> text rendered)
// history_size_sends_patch                       ({ field: 'maxHistoryCount', value: 5 })
// language_pairs_add_and_remove                  (add Russian→Ukrainian -> patch with the full map; remove -> map without the key)
// popup.test.ts addition:
// popup_applies_font_size_from_ui_changed        (fire 'ui://changed' { popupFontSize: 18 } -> main style contains '--result-font-size: 18px'; panelWidth receives 18)
// popup_bootstrap_failure_falls_back_to_english  (invoke('ui_bootstrap') rejects -> popup_ready still invoked, t('Close') === 'Close', font size 13)
// panelWidth_scales_text_term_with_font_size     (a 300-scalar result at 13 vs 26: the width grows by exactly 300 px before the 700 clamp; row term unchanged)
```

- [ ] **Step 2: Run to verify failure**

Run: `npm test --prefix app`
Expected: FAIL (components missing).

- [ ] **Step 3: Implement** as above; add any new Windows-only strings to `i18n/windows.json` (`Couldn't save settings.`).

- [ ] **Step 4: Run to verify pass**

Run: `npm test --prefix app && npm run check --prefix app && npm run build --prefix app && python3 scripts/gen-localizations.py check`
Expected: PASS; `dist/` contains separate chunks for settings (lazy).

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md` bullets for the new frontend files and the popup font-size parameter. Run the task gate.
```bash
git add -A && git commit -m "feat(app): route windows by label, add the Settings frame with the Main tab and popup font size"
```

Verification: Mac (Vitest; the Tauri dev build on macOS opens the window: `npm run tauri dev --prefix app`); window chrome and DPI manual on Windows.

---

### Task 7: Providers tab and secret-safe provider commands

**Files:**
- Create: `app/src-tauri/src/provider_cmds.rs`, `app/src/settings/{ProvidersTab.svelte, ProviderSidebar.svelte, ProviderForm.svelte}`, `app/src/settings/providers.test.ts`
- Modify: `app/src-tauri/src/lib.rs`, `app/src/lib/{settingsTypes.ts, settingsIpc.ts, icons.ts}`, `i18n/windows.json`, `AGENTS.md`

**Interfaces:**
- Consumes: Task 1 `ProviderDraft`, `DraftErrors`; Task 4 `ShellConfig::{set_api_key, set_connection, has_api_key, connection, secrets_available, connections_corrupt, update_settings}`, `settings://changed`; core `ProviderType::ALL`, `capabilities()` (`requires_api_key`, `settings_fields`, `available_models`, `default_base_url`, `auto_base_url`, `supported_operations`), `AIOperation::english_name`.
- Produces:
  - `provider_cmds.rs`: `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct ProviderState { provider, name, has_key: bool, requires_key: bool, base_url: String, model: String, referer: String, region: String, default_base_url: &'static str, auto_base_url: bool, models: &'static [&'static str], fields: &'static [ProviderField], operations: Vec<&'static str> (english names, declaration order), is_active: bool, fallback: Option<ProviderType>, secrets_unavailable: bool, connections_corrupt: bool, key_guide_url: String }`; `pub fn key_guide_url(p: ProviderType) -> String` = `https://github.com/slucheninov/lingopop-app/blob/main/docs/api-keys.md#` + anchor (`claude`, `openai`, `gemini`, `grok`, `google-free`, `google-translate`, `deepl`, `microsoft-translator`); commands `provider_state`, `provider_validate(provider, fields: ProviderFields)` (= `fields.validate(provider)`; the key is never part of this call), `provider_save(provider, draft: ProviderDraft)` (validate; non-empty errors → `Err("Invalid provider settings.")`; `connections_corrupt` → `Err(<the secrets.dat unreadable key>)`; apply the fields onto `connection(provider)` and `set_connection` (read-modify-write, Task 4); then, if `trimmed_api_key()` is `Some`, `set_api_key` (failure → `Err("Couldn't save the API key. Check that the data folder is writable.")`, connection already saved); the `ProviderDraft` value is never logged or formatted), `provider_clear_key` (= `set_api_key(provider, "")`). Every `Err` is a catalog key rendered by `t()` in the frontend. All three end by emitting `settings://changed`.
  - `ProvidersTab.svelte` (`ProviderSettingsTab.swift`): sidebar 220 px with all eight providers (`ProviderSidebar.svelte`: icon + colour from the mac `iconColor` table, `display_name`, star on the active one, status `No key needed` / `Key set` / `No key`, `· ↳ <fallback>`); selection defaults to the active provider; selecting another provider discards the draft (component keyed by provider). `ProviderForm.svelte`: header (icon, name, operations joined by ` · `, `Active` label or `Set as active` → `settings_update activeProvider`); `API key` section when `requiresKey` (`A key is stored` / `No key set`, `Clear` when stored → `provider_clear_key`; password input placeholder `Enter a new key to replace` / `Paste your API key`, `autocomplete="off"`; the DPAPI storage caption (windows key); `How to get an API key` → `open_url(keyGuideUrl)`; error line from a rejected `provider_save` / `provider_clear_key`; when `secretsUnavailable` the field is replaced by the error text and Save is disabled for the key); keyless: `No key needed` + `About this provider (no key needed)` link; `Endpoint` section when `fields` has `baseUrl` (placeholder `Auto (free/pro chosen by key)` when `autoBaseUrl` else `Base URL`; help text `Default: %@` or the DeepL text; reset button to default / empty); `Model` when `models` non-empty (`Model name` + preset menu); `HTTP Referer (optional)` with its caption; `Region (optional)` with its caption and placeholder `e.g. eastus`; `Fallback` (`If %@ fails`, `Don't fall back` + the other seven → `settings_update fallback`, applied immediately; caption `Triggered on 429 (rate limit), 5xx (server error), and network errors.`); `Save` (disabled while `provider_validate` reports errors; it is re-invoked on every change of the four non-key fields, debounced 150 ms, with the key-free `ProviderFields` only; a key keystroke triggers no invoke) + `Saved` label for 1.5 s. The key lives in one local `$state` string bound to the password input; on Save the `ProviderDraft` is built, then the key state is cleared to `''` SYNCHRONOUSLY before the `invoke` promise is awaited (so a pending or rejected save never leaves the key in component state or the DOM); after the response the draft is re-read from `provider_state`; the `DraftErrors` strings and every rejection key are rendered with `t()`.

- [ ] **Step 1: Write the failing tests**

```rust
// app provider_cmds tests (ShellConfig over a tempdir + a plaintext test protector):
// provider_save_writes_key_and_connection          (draft with api_key "sk-1", base_url "https://x" -> has_api_key true, connection.base_url "https://x")
// provider_save_response_and_logs_never_contain_key (RF 1) (serde_json::to_string(&state) and the captured log lines lack "sk-1"; the Err of a failing protector lacks it too)
// provider_save_rejects_invalid_draft              ("http://x" -> Err("Invalid provider settings."); nothing written)
// save_blocked_when_provider_settings_corrupt      (corrupt providerSettings.v1 -> Err(the unreadable key); file bytes unchanged)
// clear_key_removes_entry                          (has_key false afterwards)
// key_guide_anchors                                (all eight anchors exact)
```
```ts
// providers.test.ts:
// sidebar_lists_eight_in_order_with_status        (Claude..Microsoft Translator; Google (Free) shows 'No key needed'; a provider with hasKey shows 'Key set')
// selecting_active_by_default
// save_sends_draft_once_and_clears_key_field      (invoke('provider_save', { provider: 'Claude', draft: { apiKey: 'sk-1', ... } }); input.value === '' BEFORE the mocked invoke resolves (pending promise), and still '' after a rejection; no 'sk-1' in the DOM)
// validation_calls_never_carry_the_key            (type 'sk-SENTINEL' into the key field, then edit the model: every invoke('provider_validate', ..) argument serialized with JSON.stringify lacks 'SENTINEL' and has no apiKey property; typing the key alone triggers no invoke)
// save_disabled_on_validation_error               (provider_validate mock returns { baseUrl: 'Base URL must start with https://' } -> button disabled, message rendered through t(): with a pl catalog mapping that key, the Polish text appears)
// fallback_select_sends_settings_patch            ({ field: 'fallback', value: { provider: 'Claude', fallback: 'OpenAI' } })
// secrets_unavailable_disables_key_entry
// how_to_get_key_opens_allowlisted_url            (invoke('open_url', { url: '.../docs/api-keys.md#deepl' }))
// switching_provider_discards_draft
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-app provider && npm test --prefix app -- providers`
Expected: FAIL.

- [ ] **Step 3: Implement** as above.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-app && npm test --prefix app && npm run check --prefix app && python3 scripts/gen-localizations.py check`
Expected: PASS.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md` bullets (`provider_cmds.rs`: the key path, the Corrupt rule). Checklist lines: save a key → `Key set`; restart → still `Key set`; `lingopop.log` has no key.
```bash
git add -A && git commit -m "feat(app): add the Providers tab with secret-safe save, clear and validation commands"
```

Verification: Mac (Rust unit tests with a test protector, Vitest); DPAPI path is covered by the existing Windows DPAPI tests; end-to-end manual.

---

### Task 8: Shortcuts tab, hotkey registry with live re-registration, statuses, suspension, Screen Translate hotkey

**Files:**
- Create: `app/src/settings/{ShortcutsTab.svelte, HotkeyRecorder.svelte}`, `app/src/lib/{hotkeyRecorder.ts, ipcError.ts, ipcError.test.ts}`, `app/src/settings/shortcuts.test.ts`
- Modify: `app/src-tauri/src/{hotkeys.rs, lib.rs, settings_cmds.rs, windows.rs}`, `crates/lingopop-core/src/hotkey.rs`, `i18n/windows.json`, `AGENTS.md`

**Interfaces:**
- Consumes: Task 4 (`SettingsHooks::hotkeys`, `HotkeyStatus`, `settings://changed`), platform `HotkeyService`, `FakeHotkeyService` (`block`, `fire`), `GlobalHotkeyService`, `PlatformError::{HotkeyConflict, InvalidHotkey}`, core `Hotkey::{new, is_valid, display_string}`, `Modifier`.
- Produces:
  - core `hotkey.rs`: `pub fn duplicate_of(hotkeys: &BTreeMap<AIOperation, Hotkey>, candidate: &Hotkey, for_operation: AIOperation) -> Option<AIOperation>` (another operation with an equal hotkey; `Hotkey` equality is after `new`'s normalization).
  - `hotkeys.rs` rework. `pub trait MainThread: Send + Sync { fn run(&self, job: Box<dyn FnOnce() + Send>); }` — the shell impl calls `AppHandle::run_on_main_thread`, the test impl (`InlineMainThread`) runs the job immediately on the caller's thread (so every test below runs through exactly the code path that executes inline on the event-loop thread). `pub struct HotkeyRegistry { inner: Arc<Mutex<RegistryState { by_id: HashMap<HotkeyId, AIOperation>, status: BTreeMap<AIOperation, HotkeyStatus>, wanted: BTreeMap<AIOperation, Hotkey>, suspended: bool }>>, main: Arc<dyn MainThread>, on_change: Arc<dyn Fn() + Send + Sync> }` (managed in `ShellState`; `on_change` emits `settings://changed`). The service is reached only inside a `main.run` job through `SERVICE.with(..)` (the `thread_local!` of the event-loop thread; tests install a `FakeHotkeyService` there via `#[doc(hidden)] pub fn install_service_for_tests(Box<dyn HotkeyService>)`). LOCK RULE: the registry mutex is locked to read the plan (`wanted`, `suspended`, `by_id`) and released BEFORE `main.run`; the job re-locks only to store its result; it is never held across `main.run`. Pure helpers over the trait: `pub(crate) fn register_all(service: &dyn HotkeyService, hotkeys: &BTreeMap<AIOperation, Hotkey>) -> (HashMap<HotkeyId, AIOperation>, BTreeMap<AIOperation, HotkeyStatus>)` (every operation in `AIOperation::ALL` INCLUDING Screen Translate; `HotkeyConflict` → `Conflict`, `InvalidHotkey` / `Unsupported` → `Invalid`, other `Os` → `Invalid` + log; registration continues after a failure) and `pub(crate) fn unregister_all(service: &dyn HotkeyService, by_id: HashMap<HotkeyId, AIOperation>)` (consumes the map: after it the active map is EMPTY). Transitions: `pub fn apply(&self, hotkeys: BTreeMap<AIOperation, Hotkey>)` (store `wanted`; if not suspended: job = `unregister_all(active)` then `register_all(wanted)`, store `by_id` / `status`; then `on_change`); `pub fn suspend(&self, suspended: bool)` is IDEMPOTENT: a call that does not change the flag is a no-op (no job, no event); `true` → job = `unregister_all(active)`, `by_id` cleared, statuses kept; `false` → job = `unregister_all(active)` (defensive, the map is normally empty) then `register_all(wanted)`. `pub fn install(registry: &HotkeyRegistry, orchestrator: Orchestrator, hotkeys: &BTreeMap<AIOperation, Hotkey>)` (called from `setup` on the event-loop thread: creates the `thread_local!` `GlobalHotkeyService`, `apply(hotkeys)`, starts the forwarder thread which maps each event through the CURRENT `by_id` under the registry lock); `pub fn status(&self) -> BTreeMap<AIOperation, HotkeyStatus>`. The test `screen_translate_is_not_registered` is replaced by `screen_translate_is_registered`. `ShellHooks::hotkeys` calls `apply`. `windows.rs`: the `settings` window's `Destroyed` and `Focused(false)` events call `registry.suspend(false)` (idempotent, so a blur after a finished recording does nothing). Command `hotkeys_suspend(suspended: bool)`.
  - `settings_update Hotkey { operation, hotkey: Some(h) }` rejects `!h.is_valid()` with `Err("This shortcut can't be registered.")` and a duplicate (`duplicate_of(..)`) with `Err("Already used by %@.|<english_name>")`: the error string is `key` + `|` + args joined by `|` (pure `pub fn error_with_args(key: &str, args: &[&str]) -> String` in `settings_cmds.rs`; the frontend `lib/ipcError.ts` `renderError(e: string)` splits on `|` and calls `t(key, args)`); the frontend shows it inline and does not save; `None` clears (`set_hotkey(op, None)`).
  - `lib/hotkeyRecorder.ts` (pure, port of `HotkeyRecorderField.swift:59-76`): `export type RecorderOutcome = { kind: 'ignored' } | { kind: 'cancel' } | { kind: 'record'; key: string; modifiers: Modifier[] }`; `export function classifyKeydown(e: { code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }): RecorderOutcome` (modifier-only codes → `ignored`; `Escape` without modifiers → `cancel`; any key without modifiers → `ignored`; else `record` with modifiers in `[ctrl, alt, shift, win]` order). Validity (`is_valid`) is decided by Rust: the recorder sends `settings_update Hotkey` and shows the returned error.
  - `ShortcutsTab.svelte` (`HotkeySettingsTab.swift:10-51`): one row per `AIOperation::ALL` (icon + english name key), `HotkeyRecorder` 160x24 (click toggles recording; title `Press a key combo…` while recording else the display string; `keydown` → `classifyKeydown`; `hotkeys_suspend(true)` on recording start, `(false)` on stop / blur / unmount), ✕ button visible only when set (→ `Hotkey { hotkey: null }`); under the field a red line for `hotkeyStatus` `conflict` (`This shortcut is already used by another app.`) / `invalid` (`This shortcut can't be registered.`) or the duplicate error; the Windows footnote key from Task 3.

- [ ] **Step 1: Write the failing tests**

```rust
// core: duplicate_of_finds_other_operation (Translate Ctrl+Alt+T; candidate Ctrl+Alt+T for Explain -> Some(Translate); for Translate itself -> None)
// app hotkeys.rs (FakeHotkeyService in the thread_local, InlineMainThread):
// register_all_includes_screen_translate            (statuses for all six; Screen Translate Registered when given a hotkey)
// screen_translate_is_registered                    (replaces the 4a test: default settings register Translate; adding ScreenTranslate registers it too)
// apply_registers_the_rest_when_one_conflicts (RF 3) (block(Ctrl+Alt+E) -> Explain Conflict, Translate Registered)
// invalid_chord_is_reported_not_fatal               (Hotkey::new("", []) -> Invalid)
// suspend_then_resume_restores_all_registrations (RF 3) (after suspend(true) fire() delivers nothing and the fake has zero registrations; after suspend(false) all statuses Registered and the ids fire again)
// suspend_is_idempotent                             (suspend(false) twice from the registered state: the fake's register count unchanged, no Conflict, on_change not called; suspend(true) twice: one unregister pass)
// blur_then_destroyed_keep_everything_registered (RF 3) (recording: suspend(true); blur -> suspend(false); Destroyed -> suspend(false): every status Registered, fake registration count == number of hotkeys)
// settings_window_close_resumes_hotkeys (RF 3)      (the pure handler on_settings_window_event(Destroyed) calls suspend(false))
// apply_does_not_hold_lock_across_main_thread       (an InlineMainThread whose job calls registry.status() inside: returns, no deadlock)
// forwarder_uses_current_map_after_apply            (apply a new chord for Translate; lookup(new id) == Some(Translate), lookup(old id) == None)
// settings_update_hotkey_rejects_duplicate          (Err == "Already used by %@.|Translate"; settings unchanged)
// settings_update_hotkey_rejects_invalid            (Hotkey { key: "KeyT", modifiers: [] } -> Err("This shortcut can't be registered."); settings unchanged)
// error_with_args_round_trip                        (error_with_args("Already used by %@.", ["Translate"]) == "Already used by %@.|Translate")
```
```ts
// shortcuts.test.ts:
// classifyKeydown_rules                              (Ctrl+KeyT -> record ['ctrl'] 'KeyT'; Escape alone -> cancel; KeyT alone -> ignored; ControlLeft -> ignored; Ctrl+Escape -> record)
// recorder_suspends_while_recording                  (invoke('hotkeys_suspend', { suspended: true }) on click, false after a key is recorded, false on blur)
// recorded_chord_sends_hotkey_patch                  ({ field: 'hotkey', value: { operation: 'explain', hotkey: { key: 'KeyE', modifiers: ['ctrl','alt'] } } })
// clear_button_only_when_set_and_sends_null
// status_lines_rendered                              (conflict and invalid texts; the rejected invoke 'Already used by %@.|Translate' renders as t('Already used by %@.', ['Translate']) == 'Already used by Translate.')
// renderError_splits_key_and_args                    (lib/ipcError.test.ts: 'Key %@ %@|a|b' -> t('Key %@ %@', ['a','b']); a plain key passes through)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-core duplicate_of && cargo test -p lingopop-app hotkeys && npm test --prefix app -- shortcuts`
Expected: FAIL.

- [ ] **Step 3: Implement** as above; `picker.rs` keeps `NoPicker` until Task 13 (a Screen Translate hotkey now runs the flow and cancels silently at the pick, which is acceptable for the intermediate commit and noted in the commit message).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-app && cargo test -p lingopop-core && npm test --prefix app`; push; `gh pr checks`.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md`: `hotkeys.rs` bullet rewritten (registry, statuses, suspend, Screen Translate registered), `hotkey.rs` `duplicate_of`. Checklist lines: record a chord; occupied chord shows the conflict line; duplicate line; Translate chord pressed while recording records instead of translating.
```bash
git add -A && git commit -m "feat(app): add the Shortcuts tab with live hotkey re-registration, statuses, suspension and the Screen Translate hotkey"
```

Verification: Mac (all unit tests on `FakeHotkeyService`); real `RegisterHotKey` conflicts manual only.

---

### Task 9: Usage tab

**Files:**
- Create: `app/src-tauri/src/usage_cmds.rs`, `app/src/settings/{UsageTab.svelte, TokenChart.svelte}`, `app/src/settings/usage.test.ts`
- Modify: `app/src-tauri/src/{lib.rs, state.rs}`, `app/src/lib/{settingsTypes.ts, settingsIpc.ts}`, `AGENTS.md`

**Interfaces:**
- Consumes: Task 2 (`Period`, `Totals`, `totals`, `totals_by_provider`, `chart_buckets`, `format_tokens`, `format_cost`, `TRACKED_PROVIDERS`, `usage_display_name`), Task 4 (`ShellStores::{usage_records, reset_usage}`, `usage://changed` emitted from the store hook), `UiState.language()`.
- Produces:
  - `usage_cmds.rs`: `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct UsageView { period: Period, totals: Totals, cost_label: String (`format_cost` or `"~$?"`), tokens_label: String, in_out_label: String (`t(lang, "in %@ · out %@", ..)`), calls_label: String (`t(lang, "avg %@/call", ..)` or `t(lang, "no activity")`; the average is integer division), period_label: String (`t` of `today` / `last 7 days` / `last 30 days`), chart_title: String (`t` of `Hourly tokens · today` / `Daily tokens · 7 days` / `Daily tokens · 30 days`), buckets: Vec<Bucket>, rows: Vec<UsageRow { provider, name, colour: &'static str (`red` / `green` / `blue` / `purple`), input_label, output_label, calls: u64, cost_label }> (always the four tracked providers in order; absent = `—`, `—`, `0`, `—`), unavailable: bool (store `None`) }` — every human-readable label is built with `i18n::t(language, key, args)`, never an English literal; `pub fn usage_view(records: &[UsageRecord], period: Period, now_unix: f64, utc_offset_seconds: i32, language: Language) -> UsageView` (pure); commands `usage_get(period)` (offset from `chrono::Local::now().offset().local_minus_utc()`: add `chrono = "0.4"` with default features to `app/src-tauri/Cargo.toml`; `now_unix` from the shell clock) and `usage_reset` (→ `reset_usage`, the hook emits `usage://changed`).
  - `UsageTab.svelte` (`UsageSettingsTab.swift`): `Period` segmented control `1 day` / `7 days` / `30 days` (default 30), `Reset stats…` with the confirm `Delete all usage history?` (`Delete` / `Cancel`); three cards `Est. Cost` (red value), `Tokens`, `Calls` with their labels; `TokenChart.svelte` (SVG, 60 px tall, 3 px gaps, bars stacked in tracked order with the four colours, max = largest bucket total or 1, labels under the bars, `role="img"` + an `aria-label` and a visually hidden table of the bucket totals); the provider table `Provider` / `Input` / `Output` / `Calls` / `Est. Cost` with the coloured dot; refetch on `usage://changed` and on period change.

- [ ] **Step 1: Write the failing tests**

```rust
// usage_view_labels_match_mac               (two Claude records 1000/2000 claude-haiku-5-5 -> costLabel "$0.0022", tokensLabel "6.0K", inOutLabel "in 2.0K · out 4.0K", callsLabel "avg 3.0K/call")
// usage_view_no_activity                    (no records -> costLabel "~$?", callsLabel "no activity"; rows all "—","—",0,"—")
// usage_view_rows_always_four_in_order
// usage_view_labels_are_localized           (the same records with Language::Pl: in_out_label, calls_label, period_label and chart_title all differ from the En view and equal i18n::t(Pl, key, args))
```
```ts
// usage.test.ts:
// period_default_30_and_switch_refetches     (invoke('usage_get', { period: 'days30' }) then 'day' after clicking '1 day')
// chart_stacks_in_tracked_order              (bucket { Claude: [1,1], Grok: [2,2] } -> two rects, Claude below Grok, heights proportional)
// chart_has_text_alternative                 (aria-label present; hidden table has one row per bucket)
// reset_requires_confirmation                (click 'Reset stats…' -> dialog; 'Cancel' -> no invoke; 'Delete' -> invoke('usage_reset'))
// usage_changed_event_refetches
```

- [ ] **Step 2: Run to verify failure**: `cargo test -p lingopop-app usage && npm test --prefix app -- usage` → FAIL.

- [ ] **Step 3: Implement** as above.

- [ ] **Step 4: Run to verify pass**: `cargo test -p lingopop-app && npm test --prefix app && npm run check --prefix app` → PASS.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md` bullets. 
```bash
git add -A && git commit -m "feat(app): add the Usage tab with cards, stacked token chart and provider table"
```

Verification: Mac.

---

### Task 10: Magic tab, About tab and the update checker

**Files:**
- Create: `app/src-tauri/src/updates.rs`, `app/src/settings/{MagicTab.svelte, AboutTab.svelte}`, `app/src/lib/scalars.ts`, `app/src/settings/magicAbout.test.ts`
- Modify: `app/src-tauri/src/{lib.rs, settings_cmds.rs}`, `app/src/lib/{settingsTypes.ts, settingsIpc.ts}`, `i18n/windows.json`, `AGENTS.md`

**Interfaces:**
- Consumes: Task 1 (`updates::{fetch_latest_version, is_newer, should_check, RELEASES_PAGE_URL}`, `UiPrefs.latest_version / last_update_check`), Task 4 (`UiState::update`, `settings://changed`, `open_url`), core `DEFAULT_MAGIC_PROMPT`, `sanitize_magic_prompt`, `VERSION`, `ReqwestTransport` (the shell's existing instance, shared `Arc<dyn Transport>`).
- Produces:
  - `updates.rs`: `pub struct UpdateState { transport: Arc<dyn Transport>, ui: Arc<UiState>, clock: Arc<dyn Clock>, deadline: Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync> }` (prod: `tokio::time::sleep(UPDATE_REQUEST_TIMEOUT)`; tests: `pending()` or `ready(())`) with `pub async fn check_now(&self) -> Option<String>` (`fetch_latest_version_with(transport, deadline())`; on `Some(v)` store `latest_version = Some(v)` and `last_update_check = now` in `ui.json`; returns `Some(v)` only if `is_newer(&v, VERSION)`; `None` otherwise, silently) and `pub async fn check_at_startup(&self) -> Option<String>` (`should_check(prefs.last_update_check, now)` ? `check_now` : the cached `latest_version` filtered by `is_newer`). `setup` spawns `check_at_startup` once, then a `tokio::time::interval(CHECK_INTERVAL)` loop (first tick skipped) calling `check_now` DIRECTLY (the cache rule is for startup only; applying it in the loop skips every other tick); each `Some` emits `settings://changed`. `settings_view.latest_version` = `prefs.latest_version.filter(|v| is_newer(v, VERSION))` (an older or equal cached tag is `null`). Command `updates_check` = `check_now`. Testable: `UpdateState` takes `FakeTransport` + `ManualClock`.
  - `scalars.ts`: `export function countScalars(s: string): number` (`Array.from(s).length`), `export function truncateScalars(s: string, max: number): string`; parity with Rust `chars().count()` pinned by a table of cases (`"a😀b"` = 3, a combining sequence `"é"` as 2 code points = 2, a lone surrogate counts 1).
  - `MagicTab.svelte` (`MagicSettingsTab.swift`): description key; `System prompt` monospace textarea (`min-height: 120px`), truncated live to 2000 scalars; counter `%lld / 2000` red at ≥ 1900; `Reset to default` → confirm `Reset prompt?` / `This will replace your prompt with the default. Continue?` (`Reset` / `Cancel`) → draft = `DEFAULT_MAGIC_PROMPT` (delivered in `SettingsView` as `defaultMagicPrompt`, add the field); `Save` disabled while draft equals the saved prompt → `settings_update magicPrompt`; after the response the draft becomes the sanitized saved value.
  - `AboutTab.svelte` (`AboutTab.swift`): icon 120x120 (`/icon.png` copied into `app/public/`), `LingoPop`, `Version %@`, spinner while checking, `Check for updates` → `updates_check`; banner `Version %@ available` + `Download` → `open_url(RELEASES_PAGE_URL)` when `latestVersion` (from the view or the command) is set; the two Windows paragraphs (windows keys), `Privacy Policy` → `open_url("https://github.com/slucheninov/lingopop-app/blob/main/PRIVACY.md")`.

- [ ] **Step 1: Write the failing tests**

```rust
// app updates.rs:
// check_now_stores_cache_and_reports_newer   (FakeTransport tag v9.9.9 -> Some("9.9.9"); ui.json latestVersion "9.9.9", lastUpdateCheck == clock now)
// check_now_silent_on_404_and_network        (None; cache untouched)
// check_at_startup_uses_cache_within_3h      (cache latest "9.9.9" at now-100 -> Some without a request: FakeTransport.requests().is_empty())
// check_at_startup_refetches_at_exactly_3h   (ManualClock advanced by exactly CHECK_INTERVAL_SECONDS -> one request)
// check_now_ignores_cache                    (fresh cache, check_now -> one request)
// check_now_times_out_silently               (deadline = ready(()), transport never resolves -> None; cache untouched; no tokio timer)
// settings_view_hides_older_cached_version   (ui.json latestVersion "0.0.1" -> settings_view.latest_version == None; "9.9.9" -> Some)
// equal_or_older_latest_is_none              (tag equal to VERSION -> None; current prerelease vs stable same triple -> Some)
```
```ts
// magicAbout.test.ts:
// scalar_parity_table                        (the cases above)
// counter_red_at_1900_and_truncates_at_2000
// reset_requires_confirmation_then_sets_default
// save_disabled_until_changed_then_sends_patch ({ field: 'magicPrompt', value: draft })
// about_shows_version_and_banner_on_update   (view.latestVersion '9.9.9' -> 'Version 9.9.9 available'; 'Download' -> invoke('open_url', { url: RELEASES_PAGE_URL }))
// check_for_updates_invokes_and_stays_silent_on_null
```

- [ ] **Step 2: Run to verify failure**: `cargo test -p lingopop-app updates && npm test --prefix app -- magicAbout` → FAIL.

- [ ] **Step 3: Implement** as above.

- [ ] **Step 4: Run to verify pass**: `cargo test -p lingopop-app && npm test --prefix app && npm run check --prefix app && python3 scripts/gen-localizations.py check` → PASS.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md` bullets (`updates.rs`: FakeTransport-only testing, no URL / version override).
```bash
git add -A && git commit -m "feat(app): add the Magic and About tabs with the GitHub release check"
```

Verification: Mac. (End of PR B.)

---

### Task 11: History window, tray menu, icon states, `StatusSink`

**Files:**
- Create: `app/src-tauri/src/history_cmds.rs`, `app/src/history/{History.svelte, HistoryRow.svelte}`, `app/src/lib/historySearch.ts`, `app/src/history/history.test.ts`, `crates/lingopop-orchestrator/src/status.rs`, `scripts/make-tray-icons.swift`, `app/src-tauri/icons/tray/{idle.png, working-1.png, working-2.png, error.png}` (32x32 PNG, generated, committed)
- Modify: `app/src-tauri/src/{tray.rs, lib.rs, windows.rs, state.rs}`, `crates/lingopop-orchestrator/src/{lib.rs, orchestrator.rs, actions.rs, screen.rs, fake.rs}`, orchestrator tests (`Deps` gains `status`), `app/src/main.ts`, `i18n/windows.json`, `AGENTS.md`

**Interfaces:**
- Consumes: Task 4 (`ShellStores` readers + hooks, `open_settings`, `open_history`, `settings://changed`), Task 3 `i18n::t`, core `HistoryEntry`, `languages::quick_list`, `AIOperation::english_name`, `Hotkey::display_string`, platform `Clipboard::write_text` (the shell runs `platform.clipboard.write_text` on `spawn_blocking`, outside the orchestrator gate; a plain write, no exclusion formats).
- Produces:
  - Orchestrator `status.rs`: `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum TrayStatus { Idle, Working, Error }`; `pub trait StatusSink: Send + Sync { fn status(&self, status: TrayStatus); }`; `Deps.status: Option<Arc<dyn StatusSink>>`; `FakeStatus` (`statuses() -> Vec<TrayStatus>`) in `fake.rs`. Calls: `Working` when a Loading model is presented (text flow and the Screen Translate Loading after the pick, `ScreenTranslateFlow.swift:71`); `Error` in `present_error` and on an error outcome in `finish`; `Idle` on a success outcome in `finish`, in `close` / `escape` / `dismissed`, after Replace, after a cancelled pick (`Ok(None)`) and after any early return of a flow that presented nothing. INVARIANT: a flow that set `Working` always ends in `Idle` or `Error`: `start` aborting the previous generation sets `Idle` if that generation had presented Loading (tracked by a `working: bool` in `SessionState`, set with `Working`, cleared with `Idle` / `Error`), and the join-result inspection in `start` (panic / cancel) does the same.
  - Orchestrator `pub async fn write_clipboard(&self, text: String) -> Result<(), GateError>` (in `actions.rs`: ONE gate job `platform.clipboard.write_text(&text)`, no session, no generation, no popup) — the single clipboard path for the History window and the tray rows, so a hung clipboard owner yields `Busy` / `TimedOut` instead of blocking a worker; the shell maps `Err` to `Err("Couldn't copy.")`.
  - `history_cmds.rs`: `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct HistoryRow { id: String (uppercase uuid), original, translated, source_language: Option<String>, target_language, provider_name: String (display_name or the raw value when unknown), operation: Option<AIOperation>, operation_name: String, timestamp_unix: f64 (`entry.timestamp.unix_seconds()`, never the raw Apple value) }`; commands `history_list` (newest first), `history_remove(id)` (bad uuid → `Err("Invalid id.")`), `history_clear`, `clipboard_write(text)` (= `orchestrator.write_clipboard`). The single `ShellStores` hook (installed once in Task 4) now also sends `()` on a `tokio::sync::watch` channel the `TrayController` listens to; the tray task debounces 100 ms and rebuilds on its own task, never inside the hook.
  - `historySearch.ts`: `export function filterHistory(rows: HistoryRow[], query: string): HistoryRow[]` (trim; empty = all; case-insensitive `includes` on original or translated, `HistoryView.swift:10-20`); `export function formatHistoryDate(unix: number, language: string): string` (`Intl.DateTimeFormat(language, { hour: '2-digit', minute: '2-digit', day: 'numeric', month: 'short', year: 'numeric' })`).
  - `History.svelte` (`HistoryView.swift`): search field `Search history`; trash button `Clear` only when rows exist → `history_clear` (no confirmation, parity); states `No translations yet.` + `Press your hotkey on selected text to translate.`, `No matches for “%@”.`; `HistoryRow.svelte`: operation icon + name, for Translate a badge `%@ → %@` (source → target) or the target alone, provider badge, date, `…` menu `Copy original` / `Copy translation` / `Delete` (→ `clipboard_write`, `history_remove`), original (3-line clamp, grey) and translation (6-line clamp), selectable text. Refetch on `history://changed`. `main.ts` routes `history`.
  - `tray.rs`: `pub struct TrayController { tray: TrayIcon, app: AppHandle, status: Mutex<StatusState { current: TrayStatus, generation: u64, task: Option<tokio::task::JoinHandle<()>> }> }` with `pub fn build(app, language) -> tauri::Result<Arc<Self>>`, `pub fn rebuild(&self)` (builds the menu from `config.settings()` + `stores.history_entries()` and `tray.set_menu`, called only from the debounced tray task or `setup`), `pub fn set_status(&self, s: TrayStatus)`: under the lock bump `generation`, `abort()` the previous `task` (animation or error timer), then `Idle` → `idle.png` and no task; `Working` → spawn a task alternating `working-1/2.png` every 450 ms while its generation is current; `Error` → `error.png` plus a task that after 3 s sets `Idle` if its generation is still current (`StatusItemController.swift:26-45, 66-78`); every icon change goes through `pub(crate) trait IconSink { fn set_icon(&self, frame: Frame); }` (prod: `tray.set_icon(Image::from_bytes(include_bytes!(..)))` inside `run_on_main_thread`; tests: a recording sink), so the transitions are unit-tested with `tokio::time::pause`. Construction order: `Orchestrator::new(Deps { status: Some(tray_sink), .. })` needs the tray and the tray's menu handlers need the orchestrator → `pub struct TraySink(OnceLock<Arc<TrayController>>)` implements `StatusSink` (forwards when set, drops the status before) and is filled right after `tray::build` in `setup`. `impl StatusSink for TrayController`. Menu (ids and order, `StatusMenuController.swift:62-144`): `header` disabled (`Select the clip you want to add to your clipboard`); up to `menu_history_count(&settings)` rows `hist:<uuid>` (text from `menu_row_text`) or `No recent translations` disabled; separator; `op:<raw>` per `AIOperation::ALL` with `english_name` and the accelerator text = `display_string` (`MenuItem::with_id(.., accelerator: None)`, the chord shown in the label as `"<name>\t<chord>"`; **decision: no `accelerator`, since tao would try to register it**); separator; submenu `Target: %@` with `quick_list()` + current first if absent, `CheckMenuItem` on the current (`lang:<name>` → `settings_update TargetLanguage`); `History…`, `Settings…`, `About LingoPop` (→ `open_settings(Some("about"))`); separator; `Quit LingoPop`. EVERY menu handler only `tauri::async_runtime::spawn`s (trigger, `write_clipboard`, `update_settings` + emit, `open_*`) and returns; no `block_on`, no store lock, no window creation in the handler. Pure helpers with tests: `pub fn menu_history_count(settings: &Settings) -> usize` (`min(menu_history_count, max_history_count)`), `pub fn menu_label(text: &str) -> String` (`&` → `&&`, applied to every row text and language name), `pub fn menu_row_text(index: usize, original: &str, translated: &str) -> String`: each side trimmed, `\n` → space, truncated to 40 chars with `…`, joined with ` → `, `&` doubled, prefixed with the mnemonic `&1 `..`&9 `, `&0 ` for index 0..9 and NO prefix for index ≥ 10 (muda renders `&N` as the Windows mnemonic; **decision: mnemonics kept, verified by the checklist; if they render literally on Windows the prefix is dropped**). Row click → `orchestrator.write_clipboard(translated)`; operation click → `orchestrator.trigger_from(op, TriggerSource::Tray)` (Task 12; until then `trigger(op)`).
  - `settings://changed` also triggers `tray.rebuild()` (target language, hotkeys, counts).

- [ ] **Step 1: Write the failing tests**

```rust
// orchestrator (new tests/orchestrator_status.rs): status_working_then_idle_on_success; status_error_on_failure; status_idle_on_close; screen_translate_working_starts_after_pick (FakePicker Ok(Some) -> statuses start with [Working]; Ok(None) -> statuses == [Idle] at most, never Working); superseded_working_flow_ends_idle (a hung transport flow has presented Loading (statuses == [Working]); a second trigger aborts it: statuses contain Idle before the second Working); write_clipboard_goes_through_gate (FakeClipboard text written; a hung clipboard -> Err(TimedOut) within the gate timeout, no popup call)
// app tray.rs (tokio::time::pause, recording IconSink): working_alternates_frames (450 ms steps -> working-1, working-2, working-1); working_then_idle_stops_animation (set Idle, advance 5 s: no set_icon after idle.png); error_reverts_to_idle_after_3s; error_then_working_cancels_revert (Working at 1 s, advance 5 s: still animating, no idle frame); menu_history_count_is_min; menu_row_text_truncates_and_mnemonics (index 0 -> starts with "&1 "; index 9 -> "&0 "; index 10 -> no "&" prefix; 50-char original -> 40 chars + "…"; "\n" replaced); menu_label_escapes_ampersand ("Rock & Roll" -> "Rock && Roll"; a row "A & B → C" contains "A && B → C"); target_items_put_current_first_when_not_quick
// app history_cmds.rs: history_row_uses_display_names (provider "DeepL" -> "DeepL"; unknown raw "X" -> "X"); history_row_timestamp_is_unix (entry at Timestamp::from_unix_seconds(1_700_000_000.0) -> timestamp_unix == 1_700_000_000.0, not 721_692_800.0); history_remove_rejects_bad_uuid
```
```ts
// history.test.ts: filterHistory_rules ("  Hello " matches original/translated case-insensitively; "" -> all); formatHistoryDate_uses_language ('de' output contains the German month abbreviation); empty_state_texts; no_match_text_has_curly_quotes; row_menu_copy_translation_invokes_clipboard_write; delete_invokes_history_remove; history_changed_refetches; translate_row_shows_source_arrow_target_badge
```

- [ ] **Step 2: Run to verify failure**: `cargo test -p lingopop-orchestrator --test orchestrator_status && cargo test -p lingopop-app && npm test --prefix app -- history` → FAIL.

- [ ] **Step 3: Implement**; generate the four tray PNGs with `swift scripts/make-tray-icons.swift app/src-tauri/icons/tray` (idle = the 32x32 app icon; working frames = the icon with a rotating arc in two phases; error = the icon with a red disc in the corner; 32x32, committed).

- [ ] **Step 4: Run to verify pass**: all three suites PASS; push; `gh pr checks`.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md` bullets (`status.rs`, `history_cmds.rs`, `tray.rs` menu ids and the mnemonic decision, the icon script). Checklist lines: tray rows copy the translation; mnemonic digits select rows (or render literally → drop); `Target:` submenu; About opens the About tab; icon animates while working and turns red for 3 s on an error; History window search / copy / delete / clear.
```bash
git add -A && git commit -m "feat(app): add the History window, the full tray menu with icon states and the orchestrator status sink"
```

Verification: Mac for all tests and the macOS dev tray; icon rendering and mnemonics manual on Windows.

---

### Task 12: Last-foreground tracker and tray-triggered text operations

**Files:**
- Create: `crates/lingopop-platform/src/{last_foreground.rs, fake/last_foreground.rs, windows/last_foreground.rs, macos/last_foreground.rs}`
- Create: `crates/lingopop-platform/tests/last_foreground.rs`, `crates/lingopop-platform/tests/foreground_helper.rs` (`harness = false`, Windows only via `#![cfg(windows)]` + an empty `main` elsewhere)
- Modify: `crates/lingopop-platform/{Cargo.toml (Win32_UI_Accessibility, the helper `[[test]]` entry), src/lib.rs, src/native.rs, src/fake/mod.rs}`, `crates/lingopop-platform/tests/windows_smoke.rs`, `crates/lingopop-orchestrator/src/{orchestrator.rs, lib.rs}`, `crates/lingopop-orchestrator/tests/orchestrator_text.rs`, `app/src-tauri/src/tray.rs`, the `Platform { .. }` literal list of Task 4, `AGENTS.md`

**Interfaces:**
- Consumes: platform `ForegroundWindow::{current, restore}`, `FakeForeground::{set_current? (add `set_current(Option<WindowHandle>)`), fail_restore, foreground_after_polls, restored}`, orchestrator `text_flow` gate job, `KEY_NOTHING_SELECTED`.
- Produces:
  - `last_foreground.rs`: `pub trait LastForeground: Send + Sync { fn last_external(&self) -> Option<WindowHandle>; }` and the pure filter `#[doc(hidden)] pub fn is_trackable(class_name: &str, pid: u32, own_pid: u32) -> bool` (false for our own pid and for the shell classes `Shell_TrayWnd`, `Shell_SecondaryTrayWnd`, `NotifyIconOverflowWindow`, `TopLevelWindowForOverflowXamlIsland`, `XamlExplorerHostIslandWindow`, `Progman`, `WorkerW`, `#32768` (menus): clicking the tray makes these the foreground and they must not replace the remembered window). Windows `WindowsLastForeground::new() -> Result<Self, PlatformError>`: a dedicated thread `lingopop-foreground-tracker` per instance with a message loop (`PeekMessageW` to create the queue, `GetMessageW` loop) that installs `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, NULL, callback, 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)`; NO process-wide static: the callback finds its instance through a `thread_local!` slot of the hook thread (out-of-context hooks are called on the thread that installed them) holding `Arc<AtomicU64>`; it stores `hwnd` when `idObject == OBJID_WINDOW` and `is_trackable(GetClassNameW(hwnd), GetWindowThreadProcessId pid, GetCurrentProcessId())`; `last_external()` returns the stored handle only when `IsWindow` is true (the spec's only staleness rule), else `None`; the initial value is `GetForegroundWindow()` at start when trackable. `Drop` posts a `WM_APP` quit to the thread, which calls `UnhookWinEvent` ON THE HOOK THREAD before leaving its loop, then `join()`s it, so `native_bundle_constructs` and a second instance never share state. macOS dev: `MacLastForeground` = `MacForeground::current()` (the frontmost app pid). `FakeLastForeground::new(Option<WindowHandle>)`, `set(Option<WindowHandle>)`, `calls() -> usize`. `Platform.last_foreground: Arc<dyn LastForeground>`.
  - Smoke-test helper: because `WINEVENT_SKIPOWNPROCESS` hides every window of the test process, the external windows of the desktop tests are created by a helper PROCESS: a separate test target `crates/lingopop-platform/tests/foreground_helper.rs` with `harness = false` (`[[test]] name = "foreground_helper" harness = false` in `Cargo.toml`): its `main` reads `--window <title>`, creates a topmost `EDIT` window with that title, prints `READY <hwnd>` on stdout, pumps messages, and exits on stdin EOF (invoked with no arguments, as `cargo test` does, it prints nothing and exits 0). The smoke test locates the helper binary next to its own executable (`current_exe().parent()` + `foreground_helper-*`), spawns it twice, reads the two `READY` lines, `SetForegroundWindow`s each in turn (allowed: a process may set the foreground of a process it started), and closes stdin to end them.
  - Orchestrator: `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum TriggerSource { Hotkey, Tray }`; `pub async fn trigger_from(&self, operation: AIOperation, source: TriggerSource)`; `trigger(op)` = `trigger_from(op, Hotkey)`. In `text_flow` with `Tray`, the single gate job begins with: `let target = last_foreground.last_external()`; `None` → the job returns `Captured { selection: Ok(None), .. }` so the existing `nothing_selected` path shows `KEY_NOTHING_SELECTED` (**decision: both "no remembered window" and "restore failed" use `KEY_NOTHING_SELECTED`; `KEY_FOREGROUND_LOST` claims the text is on the clipboard, which is false here**); `Some(w)` → `foreground.restore(w)`, poll `foreground.current() == Some(w)` every 20 ms up to 300 ms (the `ReplaceOptions` numbers) — failure or timeout → same `Ok(None)` path; success → proceed with `target = Some(w)` into `capture_selection` as today. `Hotkey` is unchanged. Screen Translate ignores the source.
  - Shell: the tray `op:<raw>` click calls `trigger_from(op, TriggerSource::Tray)`.

- [ ] **Step 1: Write the failing tests**

```rust
// orchestrator_text.rs additions:
// tray_trigger_restores_remembered_window_before_capture (RF 4) (FakeLastForeground(Some(W9)), FakeForeground current = W7 (our Settings window stand-in): fg.restored() == [W9] and the restore event precedes the first clipboard event in EventLog; Result shown)
// tray_trigger_without_remembered_window_shows_nothing_selected (RF 4) (None -> error key KEY_NOTHING_SELECTED, no clipboard event)
// tray_trigger_restore_failure_shows_nothing_selected (fail_restore)
// hotkey_trigger_never_consults_tracker               (FakeLastForeground.calls() == 0 after a Hotkey trigger)
// platform tests/last_foreground.rs (every OS): is_trackable_table (own pid false; each of the eight shell classes false; "Notepad" with a foreign pid true; "Edit" true)
// windows_smoke.rs (#[ignore = "needs interactive desktop"]):
// tracker_remembers_foreground_switch_between_external_windows (two helper PROCESSES, SetForegroundWindow each in turn; last_external() follows within 500 ms)
// tracker_ignores_own_process_windows (RF 4)         (one helper process window is foreground; then an in-process EDIT window is activated: last_external() still returns the helper's hwnd)
// tracker_returns_none_after_window_destroyed        (close the helper's stdin -> process exits -> last_external() == None)
// two_trackers_do_not_share_state                    (construct two WindowsLastForeground, drop one, the other still updates; no panic on the second Drop)
```

- [ ] **Step 2: Run to verify failure**: `cargo test -p lingopop-orchestrator --test orchestrator_text` → FAIL to compile.

- [ ] **Step 3: Implement** as above (helper threads in the smoke tests pump their own message loops, like the existing `capture_selection` smoke test).

- [ ] **Step 4: Run to verify pass**: `cargo test -p lingopop-orchestrator && cargo test -p lingopop-platform`; push; `gh pr checks` incl. the desktop step.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md` bullets (`last_foreground.rs`, `TriggerSource`, the key decision). Checklist: select text in Notepad, open the tray menu, click Translate → the popup shows the Notepad selection; with no prior external window (fresh sign-in, nothing opened) → the nothing-selected message.
```bash
git add -A && git commit -m "feat(platform,orchestrator): track the last external foreground window and restore it for tray-triggered operations"
```

Verification: orchestrator tests on the Mac; the hook in Windows CI desktop smoke; end-to-end manual.

---

### Task 13: Region overlay (native Win32) and the `RegionPicker` wiring

**Files:**
- Create: `crates/lingopop-platform/src/{overlay.rs, fake/overlay.rs, windows/overlay.rs, macos/overlay.rs}`, `crates/lingopop-platform/tests/{overlay_geometry.rs, windows_desktop_input.rs}`
- Modify: `crates/lingopop-platform/{Cargo.toml (Win32_Graphics_Dwm), src/lib.rs, src/native.rs, src/fake/mod.rs}`, `crates/lingopop-platform/tests/windows_smoke.rs`, `.github/workflows/ci.yml` (optional input-driven step), `app/src-tauri/Cargo.toml` (`crossbeam-channel`), the `Platform { .. }` literal list of Task 4, `crates/lingopop-orchestrator/src/screen.rs`, `crates/lingopop-orchestrator/src/fake.rs` (`FakePicker` cancel counter), `crates/lingopop-orchestrator/tests/orchestrator_screen.rs`, `app/src-tauri/src/{picker.rs, lib.rs}`, `AGENTS.md`

**Interfaces:**
- Consumes: platform `Rect`, `MonitorInfo`, `monitor_at`, `virtual_screen`, `Monitors`, `DpiGuard`, `MessageWindow` patterns, `crossbeam_channel`; orchestrator `RegionPicker`, `screen_flow`.
- Produces:
  - `overlay.rs`: `pub trait RegionOverlay: Send + Sync { fn pick(&self, cancel: crossbeam_channel::Receiver<()>) -> Result<Option<Rect>, PlatformError>; }` (blocking; returns when the user releases the mouse, presses Esc, or `cancel` receives a message or is disconnected; `Ok(None)` = cancelled or below the threshold). Pure helpers (`#[doc(hidden)] pub`, tested everywhere): `pub const MIN_SELECTION_PX: i32 = 10; pub const DIM_ALPHA: u8 = 77 (0.3 * 255); pub const BORDER_PT: f64 = 1.5; pub const LABEL_PT: f64 = 11.0;` `pub fn selection_rect(start: Point, current: Point) -> Rect` (min / abs), `pub fn selection_accepted(r: Rect) -> bool` (`width >= 10 && height >= 10`, physical px), `pub fn border_px(scale: f64) -> i32` (`ceil(1.5 * scale)`), `pub fn label_origin(sel: Rect, label: Size, bounds: Rect) -> Point` (right-aligned inside the selection's bottom-right with a 4 px margin, kept inside `bounds`: `CaptureOverlayWindow.swift:196-211`, y axis flipped), `pub fn size_label(r: Rect) -> String` (`"<w> × <h>"`, U+00D7).
  - Windows `WindowsOverlay::new()`: `pick` runs on a thread `lingopop-overlay` under `DpiGuard::per_monitor_v2()` and creates ONE layered window (**decision: one window, not one per monitor**) at the ORIGIN of `virtual_screen(monitors)` (which may be negative) with its size, styles `WS_POPUP | WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, shown with `SW_SHOWNOACTIVATE`. Content is a top-down 32-bit premultiplied DIB pushed with `UpdateLayeredWindow`: dim `DIM_ALPHA` black everywhere, the selection interior at ALPHA 1 (not 0: fully transparent layered pixels are click-through, so the mouse-up over the hole would reach the window below), a white border of `border_px(scale of monitor_at(selection corner))`, the size label in `LABEL_PT * scale` Segoe UI semibold white with a 1 px black shadow. Redraws are incremental: `WM_MOUSEMOVE` repaints only the union of the old and new selection rects (plus border and label) into the DIB and calls `UpdateLayeredWindow` with `pptDst = NULL` and `ULW_ALPHA` — the whole DIB is referenced but only the dirty pixels were touched; the initial dim fill happens once. Input: the window never activates, so `WM_LBUTTON*` delivery is not guaranteed; the overlay therefore owns a `WH_MOUSE_LL` and a `WH_KEYBOARD_LL` hook for the pick's lifetime, both installed AND pumped on the overlay thread (low-level hooks are global and are called on the installing thread's message loop): left button down → start (`selection_rect(start, start)`), move → redraw, left button up → finish with `selection_accepted`, VK_ESCAPE down → cancel and swallowed; every other key / button passes through; mouse events over the overlay are also swallowed (return 1) so the app below sees no click; no `SetCapture`. Crosshair via `WM_SETCURSOR` → `IDC_CROSS` plus `SetCursor` on every hooked move while picking. A `SetTimer` 50 ms tick checks `cancel.try_recv()` (`Ok` or `Disconnected` → cancel). All paths destroy the window, unhook both hooks, kill the timer (RAII guards), then `DwmFlush()` followed by a fixed `sleep(50 ms)` before returning, so the following `capture_region` never contains the overlay. Picks are SERIALIZED: a `Mutex<()>` inside `WindowsOverlay` is held for the whole `pick` (including cleanup), so a replacement pick starts only after the previous overlay is gone. macOS dev: `MacOverlay` returns `Err(Unsupported("region overlay on macOS"))`. `FakeOverlay::new(Result<Option<Rect>, PlatformError>)`, `picks() -> usize`, `block_until_cancel(bool)`, `active() -> usize` (currently inside `pick`), `max_active() -> usize`. `Platform.overlay: Arc<dyn RegionOverlay>`.
  - Orchestrator `screen.rs`: `Err(PlatformError::Unsupported(_))` from `pick` is logged (`region picker unsupported`) and treated as a cancel (nothing shown). `FakePicker` gains `cancelled() -> usize` (incremented by a drop guard inside its `pick` future when dropped before completion) and `block_forever()`.
  - Shell `picker.rs`: `pub struct OverlayPicker { overlay: Arc<dyn RegionOverlay>, done: Arc<tokio::sync::Mutex<()>> }` (add `crossbeam-channel = "0.5.17"` to `app/src-tauri/Cargo.toml`); `impl RegionPicker`: `pick` creates `(tx, rx) = crossbeam_channel::bounded(1)`, acquires `done` (held across the await, so a replacement waits for the previous native pick's cleanup ack = the `spawn_blocking` returning), moves `rx` into `tauri::async_runtime::spawn_blocking(move || overlay.pick(rx))` and holds `tx` in a guard that lives across the `.await`; dropping the future (a second Screen Translate hotkey, close) disconnects the channel, the overlay cancels, and the blocking task's completion releases `done` for the next pick. `NoPicker` is deleted; `Deps.picker = Arc::new(OverlayPicker::new(platform.overlay.clone()))`.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/overlay_geometry.rs (every OS): selection_rect_normalizes_drag_direction (start (100,100) current (20,40) -> Rect { x: 20, y: 40, width: 80, height: 60 }); selection_accepted_threshold (9x10 false, 10x10 true); border_px_scales (1.0 -> 2, 1.5 -> 3, 2.0 -> 3); label_origin_stays_inside_bounds (also with a negative virtual-screen origin); size_label_uses_multiplication_sign ("120 × 40")
// orchestrator_screen.rs additions: unsupported_pick_is_a_silent_cancel (no popup calls, log line present); second_screen_translate_cancels_first_pick (RF 5) (FakePicker.block_forever(); tokio::spawn(orch.trigger(ScreenTranslate)) — the first trigger must be spawned, a sequential await would hang — yield, trigger again: cancelled() == 1)
// app picker.rs (FakeOverlay): overlay_pick_returns_none_when_cancel_sender_drops (RF 5) (block_until_cancel(true); drop the OverlayPicker future -> the blocking call returns within 1 s); rapid_restarts_keep_one_native_pick_active (M3) (three picks started 10 ms apart, each dropped before the next; FakeOverlay.max_active() == 1 and picks() == 3 after all settle)
// windows_smoke.rs (#[ignore = "needs interactive desktop"], required CI step): overlay_creates_and_destroys_window (count top-level windows of the overlay thread before / after a pick cancelled through the channel); overlay_destroys_window_on_cancel (RF 5) (send on tx -> pick returns Ok(None) within 500 ms, window gone, no WH_MOUSE_LL / WH_KEYBOARD_LL left: a SendInput Escape afterwards is seen by a test window); overlay_window_origin_is_virtual_screen_origin (GetWindowRect == virtual_screen)
// windows_desktop_input.rs (NEW file, #[ignore = "needs interactive desktop, input-driven"], NOT in the required CI step; run by hand and in an optional `continue-on-error: true` step): overlay_esc_cancels (SendInput VK_ESCAPE -> Ok(None)); overlay_drag_below_threshold_is_none (5x5 absolute drag -> Ok(None)); overlay_drag_returns_rect (100x60 drag -> width 100 ± 1); overlay_backward_drag_normalizes (drag from bottom-right to top-left -> the same rect); overlay_release_over_interior_finishes (mouse-up inside the hole still finishes, alpha-1 interior); overlay_survives_capture_loss (a second window calls SetForegroundWindow mid-drag -> the pick still completes on mouse-up or returns Ok(None), never hangs)
```

- [ ] **Step 2: Run to verify failure**: `cargo test -p lingopop-platform --test overlay_geometry && cargo test -p lingopop-orchestrator --test orchestrator_screen` → FAIL to compile.

- [ ] **Step 3: Implement** as above.

- [ ] **Step 4: Run to verify pass**: local suites PASS; push; `gh pr checks` incl. the required desktop step (which runs only `desktop_probe` and `windows_smoke`); the input-driven file runs in a new optional step `Windows input-driven desktop tests` with `continue-on-error: true`: `cargo test -p lingopop-platform --locked --test windows_desktop_input -- --ignored --nocapture --test-threads=1`.

- [ ] **Step 5: Documentation, gate and commit**

`AGENTS.md` bullets (`overlay.rs` and the Windows design: one window, NOACTIVATE + LL Esc hook + capture, `DwmFlush`, cancellation by channel). Checklist: overlay dims all monitors incl. mixed DPI; crosshair; size label; Esc; < 10x10 cancels silently; the capture never shows the overlay; a second hotkey press during selection restarts the overlay; the taskbar is covered.
```bash
git add -A && git commit -m "feat(platform,app): add the native Win32 region overlay and wire it as the Screen Translate picker"
```

Verification: geometry + cancellation on the Mac; window behaviour in Windows CI desktop smoke; visuals manual.

---

### Task 14: Manual checklist, CHANGELOG, AGENTS.md pass, PR ready

**Files:**
- Modify: `docs/testing/manual-checklist.md`, `CHANGELOG.md`, `AGENTS.md`, `docs/superpowers/specs/2026-11-01-phase4b-windows-ui-design.md` (status line: implemented)

- [ ] **Step 1: Checklist**

Add the sections `Settings`, `Shortcuts`, `Screen Translate`, `Tray and History`, `Startup and language` with the lines collected in Tasks 5-13 (format `- [ ] action. Expected: result.`), plus: second launch of the exe focuses / opens Settings; language change → Relaunch → the UI is in the new language and only one process runs; API key never in `lingopop.log`; `Check for updates` with no stable release does nothing visible; closing Settings while recording a shortcut restores the hotkeys.

- [ ] **Step 2: Docs**

`CHANGELOG.md` Unreleased: one bullet per area (Settings window, Providers key entry, Shortcuts, Usage, Magic, About + update check, History window, tray menu and icon states, Screen Translate overlay and hotkey, localization en/pl/uk/de, launch at login, second instance opens Settings). `AGENTS.md`: re-read the Layout for the IPC contract table, the new platform traits in `native()`, the `Build / test` line with `python3 scripts/gen-localizations.py check`, and the "Platform checks" paragraph (registry tests run by default; overlay / tracker smoke in the desktop step). The design doc gets `Статус: реализовано в плане ...` on line 3.

- [ ] **Step 3: Final verification and commit**

Run: `npm ci --prefix app && npm run build --prefix app && npm test --prefix app && npm run check --prefix app && python3 scripts/gen-localizations.py check`, then the task gate; push; `gh pr checks` green on both legs and CodeRabbit.
```bash
git add -A && git commit -m "docs: add the Phase 4b checklist, changelog and layout notes"
```
Mark the PR(s) ready for review. Never merge.

---

## Self-review notes

- Spec coverage: §3.1 goals → Tasks 4-13; §5 data model → Tasks 1, 4, 7; §6.1-6.8 → Tasks 6-11; §7 tray → 11-12; §8 overlay → 13; §9 localization → 3 (+ 6 for the Relaunch row, 5 for the mechanism); §10 usage → 2, 9; §11 autostart → 5; §12 updates → 1, 10; §13 `ui.json` → 1, 4; §14 testing → each task; §15 executor choices → decided inline (one overlay window; `StatusSink` trait; tracker on its own thread; `TriggerSource` enum; mnemonics kept; relaunch by `--wait-for-pid`; `i18n/` layout and `gen-localizations.py`; explicit exclusions file). Nothing from §15 is left open except the visual details it explicitly delegates.
- Type consistency: `SettingsView` / `SettingsPatch` / `UiPatch` (Task 4) are the only shapes Tasks 6-10 consume; `HotkeyStatus` is defined once in Task 4 and filled in Task 8; `TriggerSource` (12) is used by the tray (11 notes the interim `trigger`); `RegionOverlay::pick(cancel)` (13) matches `OverlayPicker`.
- Review round 1 (three independent reviews) folded in: no sync window creation (B1), idempotent hotkey suspend with a `MainThread` seam (B2), owned i18n maps + `Cow` lookup (B3), `UiPatch` camelCase (B4), placeholder lazy modules (B5), tracker helper process + `is_trackable` + per-instance state (B6), tray animation generations and the Working→Idle invariant (B7), pending tab via `ui_bootstrap` (M1), overlay LL hooks / alpha-1 hole / dirty-rect redraw / serialized picks / separate input-driven test file (M2-M4), key-free `provider_validate` (M5), read-modify-write `set_connection` (M6), localized usage labels and `key|args` errors (M7), update loop and 10 s deadline (M8), one store hook invoked outside the lock (M9), gated `write_clipboard` (M10), Unix-second filtering (M11), 30-day labels (M12), ASCII region (M13), `panelWidth(model, measure, fontSize)` (M14), Cargo features / setup-python (M15), minor items m1-m7.
- Proportion: ~125 KB for a spec of ~45 KB (the review round added decisions, not code); code appears only as signatures, test names and assertions.
