# LingoPop

LingoPop is a tray / menu-bar app that translates, fixes, rewrites and explains selected text, and translates text on screen, with the AI provider of your choice.

This repository holds **both the Windows and the macOS version**: one Rust/Tauri v2 codebase with an OS-agnostic core, a shared Svelte UI and native adapters for each OS.

| Platform | Status |
|---|---|
| Windows 10/11 x64 | Pre-release: unsigned portable exe on the [Releases page](https://github.com/slucheninov/lingopop-app/releases) (needs the WebView2 runtime; SmartScreen warns) |
| macOS 13+ (Apple silicon and Intel) | Preview, starting with the next pre-release: `LingoPop Preview` as an ad-hoc signed `.dmg` / `.zip` per architecture on the [Releases page](https://github.com/slucheninov/lingopop-app/releases) (Gatekeeper asks for Open Anyway; a menu-bar app that needs Accessibility and Screen Recording). The remaining work, including the import from the Swift app, is in [docs/superpowers/plans/2026-11-03-cross-platform.md](docs/superpowers/plans/2026-11-03-cross-platform.md) |

The original Swift macOS app [`slucheninov/lingopop`](https://github.com/slucheninov/lingopop) is frozen. This repository uses the same history and usage formats, and the macOS version will offer an explicit import of the ordinary settings, history and usage from it. API keys and provider connection overrides are not imported and must be re-entered.

## Installing

Download the file for your system from the [Releases page](https://github.com/slucheninov/lingopop-app/releases): `LingoPop-<version>-windows-x64-portable.exe` for Windows, `LingoPop-<version>-macos-arm64.dmg` (Apple silicon) or `LingoPop-<version>-macos-x64.dmg` (Intel) for macOS 13 or newer, and check it against `SHA256SUMS.txt`. The full instructions (SmartScreen, WebView2, Gatekeeper Open Anyway, the macOS permissions, data folders) are in [docs/INSTALL.md](docs/INSTALL.md) and in the notes of every release.

## Documentation

- Installing: [docs/INSTALL.md](docs/INSTALL.md); releasing: [docs/RELEASING.md](docs/RELEASING.md)
- Cross-platform design: [docs/superpowers/specs/2026-11-03-cross-platform-design.md](docs/superpowers/specs/2026-11-03-cross-platform-design.md)
- Original Windows design: [docs/superpowers/specs/2026-10-30-lingopop-app-design.md](docs/superpowers/specs/2026-10-30-lingopop-app-design.md)
- Contributor and agent instructions: [AGENTS.md](AGENTS.md)
- Getting an API key for each provider: [docs/api-keys.md](docs/api-keys.md)
- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md)
- Security policy: [SECURITY.md](SECURITY.md)
- Privacy policy: [PRIVACY.md](PRIVACY.md)
- Changelog: [CHANGELOG.md](CHANGELOG.md)
