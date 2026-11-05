# Changelog

All notable changes to LingoPop are documented here. Each release also has auto-generated notes on the [Releases page](https://github.com/slucheninov/lingopop-app/releases).

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/) and the project uses [Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- macOS preview builds: each release also carries `LingoPop-<version>-macos-arm64` (Apple silicon) and `-macos-x64` (Intel) as `.dmg` and `.zip` with the menu-bar app `LingoPop Preview` (macOS 13 or newer, ad-hoc signed unless Developer ID secrets are configured). The release notes now end with installation instructions for both platforms ([docs/INSTALL.md](docs/INSTALL.md)).
- The release is published only after every platform leg succeeded and one combined `SHA256SUMS.txt` was verified against the exact asset set, the executable architectures and the macOS bundle metadata.

### Changed

- The repository now covers both the Windows and the macOS version; README and AGENTS.md describe the shared codebase and the cross-platform plan.
- The manual checklist file is removed; release-time manual checks are recorded in the PR instead.
- CI and every release leg check that `Cargo.toml`, `tauri.conf.json`, `app/package.json` and `app/package-lock.json` carry the same version and that the macOS Tauri override has no `version` (`scripts/check-version.py`).

## [0.2.0-alpha.7] - 2026-11-03

### Added

- History window (from the tray menu): search, copy the original or the translation, delete one entry or clear all.
- Tray menu with the recent history, the six operations with their hotkeys, the target language and Settings / History / About; the tray icon animates while an operation runs and turns red for 3 seconds after an error. An operation started from the tray menu works on the text you had selected in your last window.
- Screen Translate: a native selection overlay (dimmed screen, crosshair, size label; Esc or a tiny selection cancels) opened by the Screen Translate hotkey or the tray menu, then OCR and translation in the popup.

## [0.2.0-alpha.6] - 2026-11-03

### Added

- Settings window (a second launch opens it) with the Main, Providers, Shortcuts, Usage, Magic and About tabs, ported from the macOS app.
- Main: target language with auto-detect and language pairs, popup font size, app language (English, Polish, Ukrainian, German; takes effect after a restart), input limits and launch at login.
- Providers: API keys (stored with DPAPI, never shown again), base URL, model, Referer and region per provider, the active provider and its fallback.
- Shortcuts: record, clear and validate the six global hotkeys, with conflict and invalid-chord messages.
- Usage: token and cost statistics for 1, 7 and 30 days with a chart and a per-provider table, and a reset.
- Magic: edit the Magic prompt (2000 characters) or reset it to the default.
- About: version, update check against GitHub releases (also every three hours in the background) and the privacy policy link.
- The popup and the Settings window are localized in Polish, Ukrainian and German.

## [0.2.0-alpha.5] - 2026-11-01

### Fixed

- The administrator-app error now closes by itself after 6 s; Windows does not deliver clicks or Esc in an elevated window to LingoPop's hooks.
- Replace now keeps the leading and trailing whitespace (line breaks) of the selection it overwrites; selecting a full line and replacing it no longer joins it with the next line.
- No Alt menu access-key tips (Notepad F E V ...) after a hotkey that contains Alt or Win: the hotkey now taps an unassigned key so the physical Alt release is not a lone Alt press.

### Changed

- The manual checklist is trimmed to basic functionality.

## [0.2.0-alpha.4] - 2026-11-01

### Changed

- The default active provider is Google (Free) instead of Claude (the mac default), so a fresh install translates without an API key. An `activeProvider` saved in `settings.json` still wins.

### Fixed

- Google (Free) always failed with "Rate limit exceeded" (HTTP 429): Google rejects the HTTP/2 connection of the HTTP client; Google (Free) requests now use HTTP/1.1.
- Clicking outside the popup did not close it in a virtual machine (guest tools deliver the mouse as injected input, which the dismiss hook ignored); injected clicks now count, injected Escape is still ignored.

## [0.2.0-alpha.3] - 2026-10-31

### Fixed

- Popup scrollbars and a clipped close button on Windows: the window shadow shrank the client area below the size the frontend measured; the popup window no longer has a shadow
- Dismiss-hook installation failures (and success) are now written to `lingopop.log` instead of the invisible stderr

## [0.2.0-alpha.2] - 2026-10-31

### Fixed

- The portable exe loaded the dev server URL (the popup showed "can't reach this page"); release builds now enable the `custom-protocol` feature and embed the frontend

## [0.2.0-alpha.1] - 2026-10-31

- Repository bootstrap and core foundations (not user-visible)
- HTTP transport, eight providers and the operation runner with fallback in `lingopop-core`; live smoke tests (not user-visible)
- Platform layer for Windows in `lingopop-platform` (selection capture and replace, clipboard safety, screen capture, OCR, global hotkeys, DPAPI secrets) and a macOS dev implementation (not user-visible)
- Platform review fixes: atomic conditional clipboard restore (`restore_if`), modifier wait before the snapshot, protection markers kept across snapshot / restore, MSRV 1.89 (not user-visible)
- Phase 4a, tray app with the text flow: new `lingopop-orchestrator` crate (serialized platform gate with timeouts, popup placement, view model, text and Screen Translate flows over the platform traits, popup actions Copy / Replace / Retry / Rewrite / Explain / Retranslate), a no-activate topmost popup window with Esc and click-outside dismiss hooks, a Tauri v2 tray shell (single instance, global hotkeys on the event-loop thread, file log without user text, `%APPDATA%\LingoPop` data folder) and a Svelte popup (loading, result, OCR and error views, adaptive action row). Screen Translate has no hotkey yet (region picker in Phase 4b); settings UI and API key entry are not included
- Phase 4a fixes: a failed Copy or Replace keeps the translation on screen with a notice instead of replacing it with an error; OCR images over the engine limit are downscaled; monitors fall back to 96 DPI per failed DPI query; the clipboard snapshot logs unsaved format ids; the manual test checklist is in `docs/testing/manual-checklist.md`; a Replace that times out or panics (the paste may still happen) no longer offers Replace again in that session; the popup is placed and clamped with its measured size, and the click-outside area follows it
- Phase 4a review fixes: a failed app build is logged before exit; popup labels go through the translation function and the action row is measured with the same titles; a panic in an operation flow shows an error instead of a stuck loading popup; Screen Translate timeouts have their own message
- The frontend is type-checked with TypeScript 7 (`svelte-check --tsgo`, `@typescript/native` alias; TypeScript 6 stays as the svelte-check host) (not user-visible)
