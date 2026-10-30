# Contributing to LingoPop

Thanks for your interest in LingoPop! Project architecture and conventions live in [AGENTS.md](AGENTS.md); read it before making non-trivial changes.

## Development setup

Requirements: Rust stable (see `rust-toolchain.toml`). No Windows machine is needed for the core crate: it is OS-agnostic and builds and tests on macOS and Windows. Windows-specific behaviour is verified by CI. Phase 4 (the Tauri app) adds Node.

```bash
cargo build --workspace
```

## Before opening a pull request

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
bash scripts/check-core-deps.sh
```

- Open pull requests against `main` only. Direct pushes to `main` are blocked.
- CI (`build-and-test`) must be green before a PR can be merged.
- Pull requests are reviewed by CodeRabbit (config in `.coderabbit.yaml`). Merging requires its approval (resolve or answer its comments), a green `CodeRabbit` status check and green `build-and-test`. Reviews run automatically on pushes to open, non-draft pull requests (mark a draft "Ready for review" to get reviewed); comment `@coderabbitai review` only if a review was skipped, paused or rate-limited. Authors cannot self-approve via CodeRabbit commands.
- Update `AGENTS.md` if you change the architecture or conventions.
- Include screenshots for UI changes.

## Core rules

- `lingopop-core` stays OS-agnostic: no `windows`, `winapi`, `tauri*`, `objc*`, `cocoa` or `core-foundation*` dependencies and no `unsafe`.
- Never rename persisted raw values (`ProviderType`, `AIOperation`) or change the date and UUID encoding; they are shared with the macOS app.
- Never localize persisted identifiers, language names in the language table, AI prompt texts, log messages, or HTTP/JSON fields.

See `AGENTS.md` for details.

## Security and secrets

Never paste API keys, tokens, or other secrets into issues, pull requests, or logs. Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
