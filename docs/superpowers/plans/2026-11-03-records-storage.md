# Records Storage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Select a folder for history and usage only, with recoverable relaunch-based switching and safe external-file handling.

**Architecture:** Inject separate local/record paths. A platform filesystem seam supplies lossless path identities and local locks; a shell coordinator journals two-file switches and reconciles validated envelopes. Current synchronous `Persist` operations stay synchronous atomic commits on worker paths; no UI callback does filesystem work.

**Tech Stack:** Existing Rust/serde/file helpers, target-specific filesystem handles, Tauri IPC, Svelte settings and current envelope codecs.

**Spec:** `docs/superpowers/specs/2026-11-03-cross-platform-design.md`; all Global Constraints and the shared gate in `2026-11-03-cross-platform.md` apply verbatim.

## Global Constraints

- The selected folder contains `history.json` and `usage.json` only, plus their atomic-write temporary files. Config, journals, backups, lock files and credentials remain local.
- Settings, UI preferences, hotkeys, credentials and provider connection settings stay local to each device.
- Changing the folder takes effect on relaunch. The old folder remains authoritative until both output files and the local pointer commit are verified.
- Keep source files untouched. No built-in iCloud synchronization or guarantee of lossless simultaneous disconnected-device writes.
- Preserve existing UUID/envelope/date formats, clear watermarks, 90-day tombstone retention, history limit and usage retention.

## Review Focus

- Unsupported, malformed or transient external files never become an empty writable store (Tasks 1, 4).
- A replaced directory or alias never changes the transaction target silently (Task 2).
- Interrupted pair writes recover with source and preexisting destination data retained (Task 3).
- A completed clear/delete is not resurrected from an old folder or stale in-memory copy (Tasks 3–4).
- UI cancellation, failed preference saves and relaunch during an operation preserve the active stores (Task 5).

---

### Task 1: Separate local paths, local storage preferences and strict record decoding

**Files:** Modify `crates/lingopop-core/src/paths.rs`, `lib.rs`, `store_common.rs`; create `storage_prefs.rs`, `records_snapshot.rs`, `tests/records_storage.rs`; modify `app/src-tauri/src/state.rs`, `lib.rs` and affected task-test callsites.

**Interfaces:** `LocalPaths` owns settings/ui/secrets/storage/journal/backups; `RecordsPaths::new(PathBuf)` exposes only `history_json()`/`usage_json()`. Keep `DataPaths` as a compatibility constructor mapping both locations to the same root until callers migrate. Define serde `NativePath::{UnixBytes(Vec<u8>), WindowsWide(Vec<u16>)}`, `FolderChoice::{Default, Custom(NativePath)}`, `SwitchMode::{CopyCurrent, UseDestination}`, `PendingSwitch { destination: FolderChoice, mode: SwitchMode }`, and versioned `StoragePrefs { active: FolderChoice, pending: Option<PendingSwitch> }`. Core never converts NativePath via lossy strings. Reject a wrong-OS encoding, relative paths and malformed native path values without rewriting local preferences. Define `ReadPolicy::{InitializeMissing, Established}` and strict `RecordsSnapshot::decode(history: Option<&[u8]>, usage: Option<&[u8]>, policy: ReadPolicy) -> Result<RecordsSnapshot, StorageError>` retaining raw bytes/revisions and both typed envelopes; missing established files and future versions are errors. `StorageError` defines Unavailable, UnsupportedVersion, InvalidData, Io, ChangedDirectory, Conflict, SameDirectory and RecoveryRequired with fixed text, key-free Debug and no raw record/path payload. Add strict per-file `decode_history(bytes: Option<&[u8]>, policy: ReadPolicy)` and `decode_usage(bytes: Option<&[u8]>, policy: ReadPolicy)` returning `Result<MergeableEnvelope<HistoryEntry>, StorageError>` / `Result<MergeableEnvelope<UsageRecord>, StorageError>` respectively; RecordsSnapshot::decode combines them for pair transactions, while ordinary refresh can keep the unaffected store available. Expose pure `RecordsSnapshot::merged(&self, other: &Self, now: Timestamp, history_limit: u32) -> Result<RecordsSnapshot, StorageError>` using existing envelope/store normalization.

- [ ] Write `records_storage.rs` regressions asserting `LocalPaths` secret/config methods never resolve under a custom RecordsPaths; default paths stay byte-for-byte compatible; `NativePath` roundtrips non-UTF8 Unix bytes and Windows unpaired units without lossy conversion; absent prefs default locally, but malformed/future prefs block rewriting. Pin `decode(None, None, Established) == Err(StorageError::Unavailable)`, `decode(valid_history, future_usage, Established) == Err(StorageError::UnsupportedVersion)`, and zero file mutations for decoding errors. Test legacy/BOM, UUID deduplication and newer clear/tombstone dominance with the mac fixtures.

**Regression anchor — `established_missing_records_are_unavailable`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert!(matches!(RecordsSnapshot::decode(None, None, ReadPolicy::Established), Err(StorageError::Unavailable)));
```

- [ ] Run `cargo test -p lingopop-core --test records_storage --locked`; capture RED for the new routing/strict decoder contracts.
- [ ] Implement codecs/paths and non-writing strict validation. Existing ordinary startup corruption behavior stays available only for the original legacy-local path; selected/established folders use strict reads. Thread both path groups into `state::load` without changing secrets/config defaults.
- [ ] Run the focused command, `cargo test -p lingopop-core --test mac_fixtures --locked`, and affected shell tests. Expected: zero failures and unchanged compatibility fixtures; no generated golden edits.
- [ ] Review and locally commit scoped changes: `feat(storage): separate local configuration from record paths`.

### Task 2: Directory identity, lossless paths and cooperating local writer exclusion

**Files:** Create `crates/lingopop-platform/src/records_fs.rs`, `fake/records_fs.rs`, `windows/records_fs.rs`, `macos/records_fs.rs`, `tests/records_fs.rs`; modify platform `lib.rs`, native/fake module exports and `Cargo.toml` only for necessary target bindings.

**Interfaces:** Define opaque serde `DirectoryId` (native file/volume identity), `DirectoryRef { path: NativePath, id: DirectoryId }`, `RecordFile::{History, Usage}`, `FileRevision` (Missing or SHA-256 of bytes), and `RecordsLease: Send`. `RecordsFs: Send + Sync` supplies `identify(path: &NativePath) -> Result<DirectoryRef, StorageError>`, `lock(dirs: &[DirectoryRef]) -> Result<Box<dyn RecordsLease>, StorageError>`, `read(dir: &DirectoryRef, file: RecordFile) -> Result<Option<Vec<u8>>, StorageError>`, and `replace(dir: &DirectoryRef, file: RecordFile, expected: FileRevision, bytes: &[u8]) -> Result<FileRevision, StorageError>`. Add `remove_created(dir, file, expected) -> Result<(), StorageError>` for rolling back a previously missing destination only if the expected revision still matches. Each read/write checks directory identity using held directory handles. Native-path conversion lives here, not in core/UI. Local locks are OS-released handles in the app's local lock namespace, keyed by canonical directory identity and acquired in sorted order; never place persistent lock files in the selected folder. No trait method accepts arbitrary file names.

- [ ] Write `records_fs.rs`: alias and real path yield the same identity/lock; two local processes cannot hold the same directory lease concurrently; process exit releases it; a directory swapped after identify returns `ChangedDirectory` and does not write; compare-replace on a foreign revision returns `Conflict`; removing a newly created file refuses a foreign revision. Fake tests cover both native path encodings; native tests cover the host filesystem.

**Regression anchor — `directory_replacement_never_writes`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(result, Err(StorageError::ChangedDirectory));
assert_eq!(destination_bytes_after, destination_bytes_before);
```

- [ ] Run `cargo test -p lingopop-platform --test records_fs --locked`; capture RED, then verify chosen OS bindings against installed sources/official APIs and the macOS 13 floor.
- [ ] Implement the filesystem seam and fake fault hooks. Do not promise atomic exclusion against cloud clients/noncooperating applications; revision checks detect observed changes, not a distributed CAS.
- [ ] Run focused tests on the host and require Windows CI for handle/alias semantics. Expected: no partial write on lease/identity/conflict errors; no local-lock file under RecordsPaths.
- [ ] Review and locally commit: `feat(storage): guard record directories and revisions`.

### Task 3: Recoverable pair migration at startup

**Files:** Create `app/src-tauri/src/storage/{mod.rs,journal.rs,migration.rs,recovery.rs,tests.rs}`; modify `lib.rs`, `state.rs`, `Cargo.toml` (direct dependency only if needed), and core `storage_prefs.rs`.

**Interfaces:** `StorageCoordinator::new(local: LocalPaths, fs: Arc<dyn RecordsFs>, clock: Arc<dyn Clock>)`; `prepare_switch(choice: FolderChoice, mode: SwitchMode) -> Result<SwitchPreview, StorageError>` reads/counts but never changes active paths; `save_pending(...) -> Result<StoragePrefs, StorageError>` writes only local prefs; `recover_and_apply(history_limit: u32) -> Result<StorageStartup, StorageError>` executes before opening writable record stores. `SwitchPreview` includes source/destination identity, both revisions, merged history/usage counts and applied retention counts. `StorageStartup { paths: RecordsPaths, status: StorageStatus }`; `StorageStatus::{Ready, MigrationFailed, RecoveryRequired}` never disguises failure as success. A local versioned journal records source/destination identities, old preference bytes, preexisting destination bytes in separate private local backup files, intended output hashes and stage. Local `MigrationFs` fake injects failure after every durable operation; local files use existing private atomic-write helpers.

- [ ] Write recovery matrix tests: interrupt after journal creation, each backup, history replace, usage replace, validation, preference commit and journal completion. Assert restart yields either the old selection with original destination restored or the complete new pair, never a mixed active pair. Assert repeat recovery is idempotent, source bytes never change, corrupt/future destination refuses writes, SameDirectory is a no-op, UseDestination leaves source untouched, merged UUIDs/deletions/clear markers survive. If installed destination hashes no longer match the journal, assert recovery does not overwrite external edits and exposes RecoveryRequired.

**Regression anchor — `crash_after_history_replace_keeps_a_complete_selection`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(recovered.active_folder, old_folder);
assert_eq!(recovered.destination_pair, original_destination_pair);
assert_eq!(source_after, source_before);
```

- [ ] Run `cargo test -p lingopop-app storage:: --locked`; capture RED.
- [ ] Implement journal-first durability, sorted dual-directory leases, strict reads, local backup/prepare, checked pair replacement and verification, then preference commit. Clear pending only with a verified active commit. Recovery policy is deterministic: if the durable pointer still names the old folder, roll back only destination files matching journal-installed hashes; if it names the new verified transaction, complete bookkeeping instead of rolling back a legitimately active folder. Recovery ambiguity blocks writable record stores but keeps local settings and recovery UI available. Register startup recovery before persistence hooks/hotkeys can produce records. Never use quarantine-to-empty on selected data.
- [ ] Run the failure matrix and filesystem tests; expected zero failures and zero source mutations. Inspect temp files/backups: only managed record temp files may be in the selected folder; all backups/journal metadata remain local.
- [ ] Review and locally commit: `feat(storage): journal record folder switches across relaunch`.

### Task 4: Safe external reconciliation and store availability

**Files:** Modify core `store_common.rs`, `history_store.rs`, `usage_store.rs`; create core `tests/records_reconcile.rs`; modify shell `state.rs`, `history_cmds.rs`, `usage_cmds.rs`, `storage/mod.rs`; create shell `storage/reconcile_tests.rs`.

**Interfaces:** Add `HistoryStore::reconcile(snapshot: RecordsSnapshot) -> Result<bool, StoreError>` and `UsageStore::reconcile(snapshot: RecordsSnapshot) -> Result<bool, StoreError>` using only their corresponding envelope; `bool` indicates a changed visible snapshot, not a write. The shell coordinator owns pair revisions and leases, refreshes on Settings/History activation and before every mutation, and supplies the reconciled current state to an operation under the same local lease through checked persistence. `ShellStores::refresh_records() -> Result<(), StorageError>` and `records_status() -> RecordsStatus` expose per-store Ready/Unavailable/Stale with fixed user-message keys; strict per-file decoding from Task 1 isolates a bad history file from valid usage and vice versa. `Persist` stays synchronous, preserving orchestrator generation commit semantics; notifications occur only after successful state change and after all store/lease locks are released. Do not offload persistence and report success before a commit.

- [ ] Write tests: external append becomes visible without watcher writes; add/delete/clear/reset first merges the current disk revision; deleting locally then seeing an old envelope does not resurrect the item within retention; changed clear watermark wins; missing, corrupt, future or denied external read retains the last trusted snapshot but rejects mutation; failed saves do not fire success hooks; reconnect plus explicit refresh restores availability. Simulate an external replacement between read and checked replace and assert Conflict, not overwrite. Pin 31-day usage policy and the 90-day tombstone horizon rather than claiming unbounded offline reconciliation.

**Regression anchor — `corrupt_external_history_keeps_usage_available`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```rust
assert_eq!(history_after, last_trusted_history);
assert_eq!(history_mutation, Err(StorageError::InvalidData));
assert!(usage_is_available);
assert_eq!(success_hook_count, 0);
```

- [ ] Run `cargo test -p lingopop-core --test records_reconcile --locked` and `cargo test -p lingopop-app reconcile --locked`; capture RED.
- [ ] Implement reconcile-before-mutate with bounded conflict retry (three attempts, then actionable Conflict), no watch-triggered rewrites, and refresh on activation. A polling loop is unnecessary: explicit activation/pre-mutation refresh satisfies the first-release external-change contract. Never automatically redirect unavailable selected stores locally. Keep ordinary explicit Copy to clipboard usable while storage is unavailable.
- [ ] Run focused tests and existing `history_cmds`/usage/orchestrator commit tests. Expected zero failures; callbacks can read stores without deadlock; no stale success events or generation-regressed writes.
- [ ] Review and locally commit: `fix(storage): reconcile external records before committing mutations`.

### Task 5: Folder settings, native selection and recovery UI

**Files:** Create shell `storage/commands.rs`, `storage/dialog.rs`, `storage/command_tests.rs`; modify shell `lib.rs`, `settings_cmds.rs`, `commands.rs`, orchestrator `orchestrator.rs`/`lib.rs` and focused shutdown tests; create frontend `lib/storageTypes.ts`, `lib/storageIpc.ts`, `settings/StorageSection.svelte`, `settings/storage.test.ts`; modify `settings/MainTab.svelte`, `settings/Relaunch.svelte`, shell `commands.rs` for coordinated relaunch and catalogs in this repository.

**Interfaces:** Async IPC `storage_get() -> StorageView`, `storage_choose() -> Option<FolderSelection>`, `storage_preview(selection_id: Uuid, mode: SwitchMode) -> SwitchPreview`, `storage_schedule(selection_id: Uuid, mode: SwitchMode) -> StorageView`, `storage_use_default(mode: SwitchMode) -> StorageView`, `storage_cancel_pending() -> StorageView`, `storage_open() -> ()`, `storage_cleanup_backups() -> ()`. All return typed fixed-key errors. Exact English UI keys: `History and statistics folder`, `Change folder…`, `Open folder`, `Use default folder`, `Copy current data`, `Use destination data`, `Changes take effect after restarting LingoPop.`, `This folder stores history and statistics only.`, and `Use one device at a time and wait for your sync app to finish.` `FolderSelection { id, displayPath }` is an opaque backend-held native-path capability, never a lossy path roundtrip or arbitrary frontend path. `StorageView { activePath: String, pendingPath: Option<String>, status: StorageStatus, relaunchNeeded: bool, backupCount: u32 }` carries display labels only; no record contents or native credentials. `StorageDialog` is an injected async native folder chooser; verify the pinned Tauri/native implementation before adding dependencies. Use existing settings-write serialization for local preference changes. `storage_open` opens only the configured backend path, not arbitrary UI input.

- [ ] Write command and frontend tests: cancelled chooser performs zero writes; CopyCurrent is selected initially; preview counts/retention appear before confirmation; UseDestination and UseDefault are explicit; schedule changes pending only, never active stores; failed preference save restores the UI; Retry/Cancel after migration failure retains old selection; stale dialog results cannot override a newer choice; cleanup refuses an active journal's backups. Assert no keys/config paths appear in storage IPC or created destination files. Pin exact new English keys and pl/uk/de coverage.

**Regression anchor — `scheduling_a_folder_does_not_switch_running_stores`:** Arrange the scenario in the preceding test step; define these observed values in that task's local test fixture.

```typescript
expect(view.relaunchNeeded).toBe(true);
expect(view.activePath).toBe(oldPath);
expect(view.pendingPath).toBe(chosenPath);
expect(recordWrites).toHaveLength(0);
```

- [ ] Run `cargo test -p lingopop-app storage:: --locked` and `npm test --prefix app -- src/settings/storage.test.ts`; capture RED, implement the commands/component, then rerun. Native dialog work runs outside callback locks; file reads/writes run off the UI thread. Add `Orchestrator::prepare_shutdown() -> impl Future<Output = ()>` to cancel the current generation, stop accepting new triggers and await native pick/presentation cleanup outside session/commit locks. Shell app_relaunch awaits it before spawning the existing --wait-for-pid replacement and exiting. Test a pending operation that finishes late cannot commit after shutdown begins. Startup migration does all record switching.
- [ ] Verify real chooser, Open Folder, keyboard navigation and displayed geometry in an isolated application/browser environment when execution authorizes browser work; otherwise record native/UI validation pending. Do not infer real-render coverage from jsdom.
- [ ] Run the shared PR gate and both CI legs; expected all required checks green, Windows default data untouched.
- [ ] Review and locally commit: `feat(settings): choose history and statistics storage folder`.
