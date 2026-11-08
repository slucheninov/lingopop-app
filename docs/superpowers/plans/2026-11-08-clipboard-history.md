# Clipboard History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement task-by-task. Checkboxes track progress.

**Goal:** Persist a separate local text clipboard history, available in the tray and a dedicated window.

**Architecture:** OS adapters detect changes and read a consistent, sensitivity-checked snapshot. A shared bounded store owns history; the shell coordinates notifications, settings, tray and window. Existing clipboard preservation must remain intact.

**Tech Stack:** Rust, Tauri 2, Svelte 5, Windows APIs, AppKit.

**Spec:** `docs/superpowers/specs/2026-11-08-clipboard-history-design.md`

## Global Constraints

- Retention defaults to 100, tray display to 10; history is separate from translations and persists locally.
- No keyboard interception or new shortcuts. Never store marked sensitive data or log clipboard contents.
- Observe Windows notifications and the macOS pasteboard change counter; do not persist marked temporary clipboard writes; foreground selection copies may be recorded.
- Tray entries need no right-click deletion; delete entries in the history window.
- Preserve frozen Swift, core dependency boundaries, and existing clipboard behavior. No push or commit without user authorization.

## Review Focus

- Change occurs between checking sensitivity and reading text: verify the snapshot is internally consistent.
- Selection copies may be recorded; marked temporary writes must be skipped without suppressing foreign copies.
- Sensitive or transient formats coexist with plain text: prove the complete entry is skipped.
- Concurrent UI deletion and clipboard observation: deleted entries do not reappear from stale events.
- Failed writes, locked files, malformed content, and large inputs: no secret in logs, no silent memory/disk divergence, bounded operation.

## Task 1: Verify tray interaction and platform contracts

**Files:** inspect `app/src-tauri/src/tray.rs`, `crates/lingopop-platform/src/{windows,macos}/clipboard.rs`, and dependency APIs.

- [x] Inspect native tray menu events. User removed the per-row right-click requirement after finding Tauri does not expose it.
- [ ] Identify supported sensitive/transient markers and how to obtain text and markers from one clipboard generation on each platform. Record limitations; never claim unmarked secrets can be identified.
- [x] Map selection/copy/restore paths. User accepted recording selection copies; skip marked transient writes without suppression windows that can swallow a foreign copy.

## Task 2: Persistent store and independent settings

**Files:** create `crates/lingopop-core/src/clipboard_history.rs`; modify `crates/lingopop-core/src/{lib,paths,settings}.rs` and relevant settings tests.

- [ ] Write focused failing tests for newest-first persistence, exact-text deduplication and promotion, delete, clear, limits, reload, corrupt/locked files, atomic write failure, and no plaintext in diagnostic output.
- [ ] Run the focused core tests and confirm the new cases fail for the intended reason.
- [ ] Implement a separate bounded store following `HistoryStore`/`fsio` persistence conventions. Give entries stable IDs so tray and window actions do not rely on mutable indices. Publish mutations only after successful persistence.
- [ ] Add independently persisted settings with defaults 100 and 10; test defaults, backwards-compatible loading, bounds and trimming when retention decreases. Keep existing translation settings unchanged.
- [ ] Run `cargo test -p lingopop-core --locked` and `bash scripts/check-core-deps.sh`.

## Task 3: Clipboard observation

**Files:** `crates/lingopop-platform/src/clipboard.rs`, new focused platform modules under `src/windows/` and `src/macos/`, `app/src-tauri/src/state.rs`, `app/src-tauri/src/lib.rs`.

- [ ] Write failing deterministic tests using a fake event source for changes, duplicated notifications, sensitive/transient markers, generation changes during reads, app-origin temporary operations, concurrent foreign copies, startup and shutdown.
- [ ] Implement the interfaces established in Task 1, Windows notification lifetime and macOS change-count monitoring. Check markers and text in one stable generation; fail closed on unreadable metadata. Keep blocking work off callbacks and do not hold shared locks across OS dispatch.
- [ ] Connect successful observations to the store, settings limit and UI change events. Test that a copy selected from history promotes the existing entry once.
- [ ] Run focused platform and shell tests. Add native smoke checks for both OSs, including exclusion markers and clipboard preservation; distinguish unrun native checks from passing tests.

## Task 4: Settings, tray and full-history window

**Files:** `app/src-tauri/src/{tray,windows,settings_cmds,lib}.rs`, `app/src/settings/MainTab.svelte`, new `app/src/clipboard-history/` components, application routing and localization catalogs.

- [ ] Add failing tests for independent settings defaults and updates, tray ordering/limit, click-to-copy, deletion and clearing, and synchronization between window and tray, including stale responses.
- [ ] Implement shell commands for listing, copying, deleting and clearing entries with stable IDs, plus events for committed changes. Use the verified native interaction from Task 1; avoid exposing clipboard contents to logs.
- [ ] Add settings controls and localizations to the authoritative catalogs and regenerate using the existing add-only workflow.
- [ ] Add the dedicated window and tray menu entries. Test scrolling long histories, long/multiline text, keyboard access, errors, delete, and clear.
- [ ] Run frontend tests, `npm run check`, and localization checks; run focused shell tests.

## Task 5: Documentation and final validation

**Files:** `README.md`, `docs/settings.md`, `PRIVACY.md`, `AGENTS.md`, and any relevant platform test documentation.

- [ ] Document automatic local recording, sensitive-marker limitations, independent limits, deletion and clearing, and clipboard contents created by app operations.
- [ ] Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `bash scripts/check-core-deps.sh`, `python3 scripts/gen-localizations.py check`, and frontend `npm test` / `npm run check`.
- [ ] Run the macOS production gate and its fixture tests; execute available native desktop checks. Record unavailable Windows/native checks explicitly and perform a self-review of the complete diff.
