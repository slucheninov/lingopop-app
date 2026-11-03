# Cross-platform LingoPop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the existing Rust/Tauri application for macOS and Windows, with a selectable folder for history/statistics and a safe transition from frozen Swift.

**Architecture:** Keep core, orchestration and Svelte shared. Add production macOS adapters, split local configuration from record locations, and finish with import/release integration. Three bounded subsystem plans avoid mixing storage recovery, native UI correctness and distribution in one implementation pass.

**Tech Stack:** Rust workspace, Tauri v2, Svelte, native macOS/Windows services, Python standard-library catalog tooling, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-03-cross-platform-design.md` (owner approved 2026-11-03).

## Global Constraints

- Baseline: `187507c2f413f8377922ab5deb90f9e45f400b8e`, released `v0.2.0-alpha.7`. Read current `AGENTS.md` and the approved spec before each task.
- Develop one Rust/Tauri codebase for macOS and Windows. Keep and freeze the existing Swift app, then switch to Rust/Tauri.
- No built-in iCloud synchronization. The selected folder contains the managed files `history.json` and `usage.json` only, plus temporary atomic-write files required for those files.
- Settings, UI preferences, hotkeys, credentials and provider connection settings stay local to each device.
- No product implementation before plan review and selection of execution method. Do not push, create a PR, merge or tag merely because a task says Commit; those steps mean local scoped commits. Publishing is separately authorized.
- Preserve Windows DPAPI, directory, bundle identity, existing portable-exe asset names, callback ordering and behavior. No new Linux support.
- Core remains OS-independent, without a direct normal tokio dependency or platform TLS stack; orchestrator remains Tauri/OS-crate-free. Every platform-bound dependency is target-scoped.
- Do not block UI callbacks, hold standard mutexes across awaits, or hold a commit lock while dispatching to the main thread. Preserve generation fencing and hide-before-Replace.
- Target macOS 13.0, arm64 and x86_64. Verify dependency/API compatibility; report `NEEDS_CONTEXT` with evidence if that floor cannot be met.
- Mac native bundle uses authenticated encryption with a Keychain-held random key. No plaintext fallback or automatic dev-secret import.
- Swift bundle `com.lingopop.app` and its files remain unchanged. Rust Mac identity is `app.lingopop.tauri.macos`, local directory `~/Library/Application Support/LingoPop-Tauri`, preview name LingoPop Preview.
- Mac previews may be ad-hoc signed with accurate limitations. General availability requires Developer ID/notarization or an explicit owner-approved ad-hoc distribution exception.
- Every new visible string gets pl/uk/de entries and catalog signature validation. Never log selected text, record payloads, screenshots, keys or credential-bearing URLs.

## Review Focus

1. Folder disappears, is replaced through an alias, or contains an unsupported envelope: retain trusted state and refuse destructive writes (Storage Tasks 1–4).
2. Crash between the two destination writes or during import: recover idempotently without mixing active folders or overwriting unrelated external edits (Storage Task 3, Transition Task 18).
3. Clipboard has multiple items, promised/unreadable types or a concurrent foreign write: abort safely or preserve every supported representation, never silently clear it (Mac Task 10).
4. Old native popup work arrives after a new operation, dismissal or Replace: reject stale effects, avoid lock inversion, and wait for hide acknowledgment before paste (Mac Tasks 13–14).
5. Mixed backing scales, display removal, revoked permissions or lost source target: correct native conversion and actionable failure, without pasting/capturing elsewhere (Mac Tasks 8, 11–12, 15–16).

---

## Subsystem plans and order

1. [Records storage — Tasks 1–5](2026-11-03-records-storage.md): independently usable on existing Windows and Mac development builds; no native Mac release required.
2. [Production macOS — Tasks 6–17](2026-11-03-macos-production.md): builds on shared storage/catalog boundaries and establishes the native security/UI contracts before capture and packaging.
3. [Transition and releases — Tasks 18–20](2026-11-03-transition-release.md): explicit Swift import, combined platform packaging, real-machine acceptance and support docs.

Use separate PRs A (Tasks 1–5), B (6–9), C (10–12), D (13–14), E (15–17), F (18), G (19–20). Do not publish Mac artifacts through the Windows-only workflow before Tasks 7–17 pass their gates. Each task is a reviewable local commit; do not batch past a requested task boundary. Default order is sequential even when some tasks could run in parallel.

## File ownership map

- `lingopop-core`: path/config codecs, strict record validation/merge, catalog composition, portable shortcut labels and import mapping. No native directory selection or UserDefaults access.
- `lingopop-platform`: directory identity/locking, native path codecs, permissions, Keychain encryption, pasteboard, display conversion, source targeting, AppKit presentation, capture/OCR/overlay, login and Swift read-only access.
- `app/src-tauri/src/storage/`: folder commands, local migration journal/recovery, record-store coordination; `state.rs` only wires the services.
- `app/src-tauri/src/popup_shell.rs` and orchestrator `popup.rs`: queued presentation protocol with fenced main-thread effects, not synchronous AppKit dispatch under session locks.
- `app/src/settings/StorageSection.svelte`, `PermissionsSection.svelte`, `ImportSection.svelte`: focused settings components. Reuse current tabs and IPC/event patterns.
- `i18n/`, `scripts/`, platform Tauri config, workflow and manual-checklist files: owned by the tasks that first need them; no unrelated repository restructuring.

## Shared task gate and evidence

For every behavior change: write the named regression first, capture meaningful RED output (compile-error RED allowed only for a new API), implement, capture GREEN, inspect the complete task diff and commit only scoped paths. Tests pin behavior and failure recovery; do not mirror internal implementation.

At each PR boundary run: `npm run build --prefix app`, `npm run check --prefix app`, `npm test --prefix app`, `python3 scripts/gen-localizations.py check`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `bash scripts/check-core-deps.sh`, and `git diff --check`. Use the installed toolchain and existing lockfiles; update dependencies only when the task needs them. CI must preserve Windows/macOS matrix and required Windows desktop smoke.

Record each task's exact focused command, runner-summary lines, review result and native/manual coverage in an ignored `.superpowers/cross-platform/task-N.md`. Distinguish local tests, CI evidence, built artifacts and real installed-app coverage. Do not present one as proof of another.

## Plan self-review

- [x] Every approved spec section maps to a numbered task below or a global constraint.
- [x] Cross-plan interfaces and dependencies are explicit; Tasks 13–14 resolve the current synchronous-popup/main-thread conflict.
- [x] Five review-focus classes have named tests in their owning tasks.
- [x] Native API/dependency verification is part of the owning task, without guessing new SDK symbols or versions during planning.
- [x] No product changes, releases, repository archival, Swift migration or user-data access have been executed by writing this plan.

## Execution handoff

The design is approved; these plans still need owner review. Choose subagent-driven task implementation/review or native implementation with a whole-branch independent review. Do not infer that the earlier PR C request to use all models establishes a model policy for this new project.
