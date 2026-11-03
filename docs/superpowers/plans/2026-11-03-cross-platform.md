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
- Persistence never runs under the orchestrator commit mutex: present under the lock, persist on a blocking thread after releasing it (Storage Task 4). Records are written only through the checked `RecordsWriter` seam, never through `fsio::atomic_write` (which creates directories).
- Presentation effects are fenced by a lock-free latest-id fence advanced in `Orchestrator::start`; no session, commit or fence lock is ever held while waiting on the main thread (Mac Task 13).
- Target macOS 13.0, arm64 and x86_64. Verify dependency/API compatibility; report `NEEDS_CONTEXT` with evidence if that floor cannot be met.
- Mac native bundle uses authenticated encryption with a Keychain-held random key. No plaintext fallback or automatic dev-secret import.
- Swift bundle `com.lingopop.app` and its files remain unchanged. Rust Mac identity is `app.lingopop.tauri.macos`, local directory `~/Library/Application Support/LingoPop-Tauri`, preview name LingoPop Preview.
- Mac previews may be ad-hoc signed with accurate limitations. General availability requires Developer ID/notarization or an explicit owner-approved ad-hoc distribution exception.
- Every new visible string gets pl/uk/de entries and catalog signature validation. Never log selected text, record payloads, screenshots, keys or credential-bearing URLs.

## Review Focus

1. Folder disappears, is replaced through an alias, or contains an unsupported envelope: retain trusted state and refuse destructive writes. Tests: `alias_replacement_is_changed_directory`, `unsupported_envelope_refuses_writes`, `folder_removed_mid_session_marks_unavailable_without_write`, `path_writer_missing_directory_fails_without_creating_it` (Storage Tasks 1–4).
2. Crash between the two destination writes or during import: recover idempotently without mixing active folders or overwriting unrelated external edits. Tests: `recovery_after_each_fault_step_yields_one_consistent_state`, `crash_during_pointer_commit_recovers`, `swift_import_recovers_after_each_write_step`, `swift_import_does_not_resurrect_tombstoned_or_cleared_entries` (Storage Task 3, Transition Task 18).
3. Clipboard has multiple items, promised/unreadable types or a concurrent foreign write: abort safely or preserve every supported representation, never silently clear it. Tests: `snapshot_aborts_on_concurrent_foreign_write`, `promised_type_is_skipped_not_read` (Mac Task 10).
4. Old native popup work arrives after a new operation, dismissal or Replace: reject stale effects, avoid lock inversion, and wait for hide acknowledgment before paste. Tests: `old_show_queued_during_new_capture_has_zero_effects`, `stale_show_after_dismiss_has_no_effect`, `hide_ack_precedes_paste` (Mac Tasks 13–14).
5. Mixed backing scales, display removal, revoked permissions or lost source target: correct native conversion and actionable failure, without pasting/capturing elsewhere. Tests: `display_removal_invalidates_capture_plan`, `revoked_accessibility_never_pastes`, `target_lost_shows_manual_paste` (Mac Tasks 8, 11–12, 15–16).

---

## Subsystem plans and order

1. [Records storage — Tasks 1–5](2026-11-03-records-storage.md): independently usable on existing Windows and Mac development builds; no native Mac release required.
2. [Production macOS — Tasks 6–17](2026-11-03-macos-production.md): builds on shared storage/catalog boundaries and establishes the native security/UI contracts before capture and packaging.
3. [Transition and releases — Tasks 18–20](2026-11-03-transition-release.md): explicit Swift import, combined platform packaging, real-machine acceptance and support docs.

Use separate PRs A (Tasks 1–5), B (6–9), C (10–12), D (13–14), E (15–17), F (18), G (19–20). Do not publish Mac artifacts through the Windows-only workflow before Tasks 7–17 pass their gates. Each task lists its scoped local commits in a `**Commits:**` line (Tasks 1, 3, 5 and 13 have two or three); do not batch past a requested task or commit boundary. Default order is sequential even when some tasks could run in parallel.

## File ownership map

- `lingopop-core`: path/config codecs, strict record validation/merge, catalog composition, portable shortcut labels and import mapping. No native directory selection or UserDefaults access.
- `lingopop-platform`: directory identity/locking, native path codecs, permissions, Keychain encryption, pasteboard, display conversion, source targeting, AppKit presentation, capture/OCR/overlay, login and Swift read-only access.
- `app/src-tauri/src/storage/`: folder commands, local migration journal/recovery, record-store coordination; `state.rs` only wires the services.
- `app/src-tauri/src/popup_shell.rs` and orchestrator `popup.rs`: queued presentation protocol with fenced main-thread effects, not synchronous AppKit dispatch under session locks.
- `app/src/settings/StorageSection.svelte`, `PermissionsSection.svelte`, `ImportSection.svelte`: focused settings components. Reuse current tabs and IPC/event patterns.
- `i18n/`, `scripts/`, platform Tauri config and workflow files: owned by the tasks that first need them; no unrelated repository restructuring.

## Shared task gate and evidence

For every behavior change: write the named regression first, capture meaningful RED output (compile-error RED allowed only for a new API), implement, capture GREEN, inspect the complete task diff and commit only scoped paths. Tests pin behavior and failure recovery; do not mirror internal implementation.

At each PR boundary run: `npm run build --prefix app`, `npm run check --prefix app`, `npm test --prefix app`, `python3 scripts/gen-localizations.py check`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `bash scripts/check-core-deps.sh`, and `git diff --check`. Windows-only behaviour is verified by the CI Windows leg and the owner's Windows 11 machine in the Task 20 matrix; the Mac dev machine cannot run it and must say so in the task record. `scripts/check-version.py` (Transition Task 19) joins the gate once it exists. Use the installed toolchain and existing lockfiles; update dependencies only when the task needs them. CI must preserve Windows/macOS matrix and required Windows desktop smoke.

Record each task's exact focused command, runner-summary lines, review result and native/manual coverage in an ignored `.superpowers/cross-platform/task-N.md`. Distinguish local tests, CI evidence, built artifacts and real installed-app coverage. Do not present one as proof of another.

## Plan self-review

- [x] Every approved spec section maps to a numbered task or a global constraint (localization ownership → Task 6; folder limitation text → Task 5 (Task 20 confirms it and mirrors it in `docs/migration-from-swift.md`); README/AGENTS/design scope update → done in the planning PR, Task 20 only refreshes it for the final shipped state; old-app login item/hotkey hint → Task 18 UI).
- [x] Cross-plan interfaces are explicit: `RecordFile`/`FileRevision`/`HostKind`/`RecordsWriter` live in core (Task 1); `JournalKind::{FolderSwitch, SwiftImport}` and the startup order are declared in Task 3 and filled by Task 18; `SwiftSource::is_running` is shared by Tasks 9 and 18; `prepare_shutdown` (Task 5) gains the Hide-ack wait in Task 13.
- [x] The five review-focus classes have concrete named RED tests with fixture descriptions, not placeholders.
- [x] Native API choices are fixed before RED (NSPanel via `object_setClass`, cached `NSScreen` topology, `CGDisplayCreateImageForRect` + preflight for macOS 13, `objc2-vision`, `SMAppService`, Keychain attributes, AES-256-GCM format); anything the pinned crates do not expose is reported as `NEEDS_CONTEXT`, never guessed.
- [x] Known limitations are stated, not hidden: Escape is not swallowed on macOS; Replace's target check narrows the race and is not atomic; ad-hoc signing loses TCC grants and may prompt for the Keychain key on every upgrade; pruned 90-day tombstones cannot block an older Swift entry on import.
- [x] No product changes, releases, repository archival, Swift migration or user-data access have been executed by writing this plan.

## Revision log

- 2026-11-03 r2: reworked after independent review of the three subplans: persistence moved outside the commit mutex and onto a checked writer seam; lock-free presentation fence advanced on generation bump; typed `ProtectError` with a user-initiated secrets reset; Swift records imported from UserDefaults; prerelease-aware update channel; version-check script and combined checksums in the release workflow; concrete RED anchors and commit splits for Tasks 1, 3, 5, 13.

## Execution handoff

The design is approved; these plans were reviewed and reworked on 2026-11-03 (r2). Choose subagent-driven task implementation/review or native implementation with a whole-branch independent review. Do not infer that the earlier PR C request to use all models establishes a model policy for this new project.
