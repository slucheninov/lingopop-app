# LingoPop

LingoPop is a tray / menu-bar app that translates, fixes, rewrites and explains selected text, and translates text on screen, with the AI provider of your choice.

This repository holds **both the Windows and the macOS version**: one Rust/Tauri v2 codebase with an OS-agnostic core, a shared Svelte UI and native adapters for each OS.

| Platform | Status |
|---|---|
| Windows 10/11 x64 | Pre-release: unsigned portable exe on the [Releases page](https://github.com/slucheninov/lingopop-app/releases) (needs the WebView2 runtime; SmartScreen warns) |
| macOS 13+ (Apple Silicon and Intel) | In progress: production adapters, packaging and import from the Swift app are planned in [docs/superpowers/plans/2026-11-03-cross-platform.md](docs/superpowers/plans/2026-11-03-cross-platform.md); the current macOS build is for development only |

The original Swift macOS app [`slucheninov/lingopop`](https://github.com/slucheninov/lingopop) is frozen. This repository uses the same history and usage formats, and the macOS version will offer an explicit import of the ordinary settings, history and usage from it. API keys and provider connection overrides are not imported and must be re-entered.

- Cross-platform design: [docs/superpowers/specs/2026-11-03-cross-platform-design.md](docs/superpowers/specs/2026-11-03-cross-platform-design.md)
- Original Windows design: [docs/superpowers/specs/2026-10-30-lingopop-app-design.md](docs/superpowers/specs/2026-10-30-lingopop-app-design.md)
- Contributor and agent instructions: [AGENTS.md](AGENTS.md)
- Getting an API key for each provider: [docs/api-keys.md](docs/api-keys.md)
- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md)
- Security policy: [SECURITY.md](SECURITY.md)
- Privacy policy: [PRIVACY.md](PRIVACY.md)
- Changelog: [CHANGELOG.md](CHANGELOG.md)
