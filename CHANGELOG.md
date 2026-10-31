# Changelog

All notable changes to LingoPop for Windows are documented here. Each release also has auto-generated notes on the [Releases page](https://github.com/slucheninov/lingopop-app/releases).

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/) and the project uses [Semantic Versioning](https://semver.org/).

## Unreleased

- Repository bootstrap and core foundations (not user-visible)
- HTTP transport, eight providers and the operation runner with fallback in `lingopop-core`; live smoke tests (not user-visible)
- Platform layer for Windows in `lingopop-platform` (selection capture and replace, clipboard safety, screen capture, OCR, global hotkeys, DPAPI secrets) and a macOS dev implementation (not user-visible)
- Platform review fixes: atomic conditional clipboard restore (`restore_if`), modifier wait before the snapshot, protection markers kept across snapshot / restore, MSRV 1.89 (not user-visible)
