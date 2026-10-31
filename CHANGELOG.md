# Changelog

All notable changes to LingoPop for Windows are documented here. Each release also has auto-generated notes on the [Releases page](https://github.com/slucheninov/lingopop-app/releases).

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/) and the project uses [Semantic Versioning](https://semver.org/).

## Unreleased

- Repository bootstrap and core foundations (not user-visible)
- HTTP transport, eight providers and the operation runner with fallback in `lingopop-core`; live smoke tests (not user-visible)
- Platform layer for Windows in `lingopop-platform` (selection capture and replace, clipboard safety, screen capture, OCR, global hotkeys, DPAPI secrets) and a macOS dev implementation (not user-visible)
- Platform review fixes: atomic conditional clipboard restore (`restore_if`), modifier wait before the snapshot, protection markers kept across snapshot / restore, MSRV 1.89 (not user-visible)
- Phase 4a, tray app with the text flow: new `lingopop-orchestrator` crate (serialized platform gate with timeouts, popup placement, view model, text and Screen Translate flows over the platform traits, popup actions Copy / Replace / Retry / Rewrite / Explain / Retranslate), a no-activate topmost popup window with Esc and click-outside dismiss hooks, a Tauri v2 tray shell (single instance, global hotkeys on the event-loop thread, file log without user text, `%APPDATA%\LingoPop` data folder) and a Svelte popup (loading, result, OCR and error views, adaptive action row). Screen Translate has no hotkey yet (region picker in Phase 4b); settings UI and API key entry are not included
- Phase 4a fixes: a failed Copy or Replace keeps the translation on screen with a notice instead of replacing it with an error; OCR images over the engine limit are downscaled; monitors fall back to 96 DPI per failed DPI query; the clipboard snapshot logs unsaved format ids; the manual test checklist is in `docs/testing/manual-checklist.md`; a Replace that times out or panics (the paste may still happen) no longer offers Replace again in that session; the popup is placed and clamped with its measured size, and the click-outside area follows it
