# AGENTS.md

This file provides guidance to AI coding agents (Claude Code, Codex, Cursor, etc.) when working with code in this repository. It is the single source of truth for project instructions; `CLAUDE.md` only points here.

## What this is

LingoPop for Windows: a tray AI translator (translate, fix grammar, rewrite, explain, screen translate, magic) built with Tauri v2 and an OS-agnostic Rust core. It is a sibling of the macOS app `slucheninov/lingopop` and shares its persisted formats. Design spec: [docs/superpowers/specs/2026-10-30-lingopop-app-design.md](docs/superpowers/specs/2026-10-30-lingopop-app-design.md).

## Layout

Each task appends its own modules here in the same commit.

- `Cargo.toml` - workspace (`resolver = "2"`, members `crates/*`, shared package fields, version `0.1.0`)
- `rust-toolchain.toml`, `rustfmt.toml` - toolchain (honoured only by CI locally, no `rustup` on the dev machine) and formatting
- `.gitignore`, `.gitattributes` (`eol=lf`), `LICENSE`, `README.md`, `AGENTS.md`, `CLAUDE.md`
- `scripts/check-core-deps.sh` - guard: `lingopop-core` must not depend on OS or UI crates
- `.github/workflows/ci.yml` - CI (`core` matrix on Windows and macOS, aggregator job `build-and-test`)
- `crates/lingopop-core/Cargo.toml`, `crates/lingopop-core/src/lib.rs` - the OS-agnostic core crate (`VERSION`)
- `crates/lingopop-core/src/operation.rs` - `AIOperation` (frozen raw values, English names)
- `crates/lingopop-core/src/provider.rs` - `ProviderType`, `ProviderField`, static `ProviderCapabilities`, `ProviderConnectionSettings`, `default_providers()`
- `crates/lingopop-core/src/languages.rs` - `LanguageInfo`, the 28-row `ALL` table, `quick_list()`, `info()`, `code()` / `deepl_code()` / `microsoft_code()`, `name_for_detect_code()` (ISO 639-3 detector codes)
- `crates/lingopop-core/src/time.rs` - `Timestamp(f64)`, Apple-reference seconds kept exactly as read (lossless round trip, whole seconds written as JSON integers like Swift, always finite)
- `crates/lingopop-core/src/serde_uuid.rs` - `#[serde(with = ...)]` UUID codec (uppercase on write, any case on read)
- `crates/lingopop-core/src/records.rs` - `HistoryEntry`, `UsageRecord` (provider/operation kept as raw strings so unknown values survive a rewrite; typed accessors)
- `crates/lingopop-core/src/envelope.rs` - `MergeableEnvelope<T>` v2 file format (`version`/`items`/`deleted`/`clearedBefore`, legacy bare array and UTF-8 BOM read), `Tombstone`, `Mergeable`, pure `merged` / `pruned` / `mark_deleted` / `mark_cleared` (90-day tombstone retention; uuid case-insensitive, linear time)
- `crates/lingopop-core/src/fsio.rs` - file helpers: `strip_bom` (the crate's single BOM rule), `is_blank`, `read_optional`, `atomic_write` (temp file + `sync_all` + rename, 0600 on Unix when private; `std::fs::rename` replaces an existing file on Windows too), `quarantine` (`<name>.bak-<unix seconds>[-n]`), `LoadOutcome`, `StoreError`
- `crates/lingopop-core/src/paths.rs` - `DataPaths` (file names in the injected data dir: `settings.json`, `history.json`, `usage.json`, `secrets.dat`; the core never computes the dir)
- `crates/lingopop-core/src/store_common.rs` - crate-private `EnvelopeFile<T>`: the one implementation of the store load rule (missing / blank / BOM / legacy / quarantine of corrupt files) and persist rule (prune tombstones, atomic write, then swap memory)
- `crates/lingopop-core/src/history_store.rs` - `HistoryStore` (`history.json`, newest first, cap on `add` / `trim`, `remove` tombstones, `clear` watermark, cap overflow never tombstoned)
- `crates/lingopop-core/src/usage_store.rs` - `UsageStore` (`usage.json`, oldest first, 31-day retention from UTC midnight, `reset` watermark)
- `crates/lingopop-core/src/time.rs` also holds `Clock`, `SystemClock` and the test-driven `ManualClock`

## Build / test

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
bash scripts/check-core-deps.sh
```

Windows is verified only by CI; the dev machine is a Mac.

## Rules

- The core stays OS-agnostic: no `windows`, `winapi`, `tauri*`, `objc*`, `cocoa`, `core-foundation*` dependencies, and `#![forbid(unsafe_code)]`. `scripts/check-core-deps.sh` enforces it in CI.
- Persisted raw values and the date/UUID encoding are frozen and must never be renamed or changed:
  - `ProviderType`: `"Claude"`, `"OpenAI"`, `"Gemini"`, `"Grok"`, `"Google (Free)"`, `"Google Translate"`, `"DeepL"`, `"Microsoft Translator"`.
  - `AIOperation`: `"translate"`, `"grammarFix"`, `"rewrite"`, `"explain"`, `"screenTranslate"`, `"magic"`.
  - Dates are JSON numbers: `f64` seconds since 2001-01-01T00:00:00Z (add 978307200 for Unix epoch).
  - UUIDs are written uppercase and read case-insensitively.
  - `history.json` / `usage.json` use the envelope v2 `{ "version": 2, "items": [...], "deleted": [{"id","at"}], "clearedBefore": <date> }`.
  - UI and error text use `display_name()`, never the raw value.
- Prompt texts are copied verbatim from the macOS `Core/PromptBuilder.swift` (token `{{targetLanguage}}`) and are never localized.
- No network in unit tests.
- No secrets or API keys in logs, fixtures or issues.

## Mac compatibility

Compatibility fixtures are generated by `scripts/mac-fixtures/run.sh` and are never hand-edited.

## Branching

Until Phase 1 Task 12 is done, commits go directly to `main`. Afterwards `main` is PR-only with the required check `build-and-test` and CodeRabbit approval.
