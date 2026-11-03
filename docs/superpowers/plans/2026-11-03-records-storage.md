# Records Storage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Select a folder for history and usage only, with recoverable relaunch-based switching and safe external-file handling.

**Architecture:** Inject separate local/record paths. Core owns the record types and a `RecordsWriter` seam; a platform filesystem seam supplies directory identity and local locks; a shell coordinator journals two-file switches and reconciles validated envelopes. Record persistence runs on blocking threads outside the orchestrator commit mutex; no UI callback does filesystem work.

**Tech Stack:** Existing Rust/serde/file helpers, target-specific filesystem handles, Tauri IPC, Svelte settings and current envelope codecs.

**Spec:** `docs/superpowers/specs/2026-11-03-cross-platform-design.md`; all Global Constraints and the shared gate in `2026-11-03-cross-platform.md` apply verbatim.

## Global Constraints

- The selected folder contains `history.json` and `usage.json` only, plus their atomic-write temporary files. Config, journals, backups, lock files and credentials remain local.
- Settings, UI preferences, hotkeys, credentials and provider connection settings stay local to each device.
- Changing the folder takes effect on relaunch. The old folder remains authoritative until both output files and the local pointer commit are verified.
- Keep source files untouched. No built-in iCloud synchronization or guarantee of lossless simultaneous disconnected-device writes.
- Preserve existing UUID/envelope/date formats, clear watermarks, 90-day tombstone retention, history limit and usage retention.
- Core never uses `cfg!(windows)` / `cfg!(unix)` for path validation and never creates a record directory; record bytes reach disk only through an injected `RecordsWriter`.

## Review Focus

- Unsupported, malformed or transient external files never become an empty writable store (Tasks 1, 4): `unsupported_envelope_refuses_writes`, `folder_removed_mid_session_marks_unavailable_without_write`, `read_only_folder_mutation_fails_without_quarantine`, `corrupt_external_history_keeps_usage_available`.
- A replaced directory or alias never changes the transaction target silently, and an unmounted folder is never recreated (Tasks 1–3): `path_writer_missing_directory_fails_without_creating_it`, `directory_replacement_never_writes`, `alias_replacement_is_changed_directory`.
- Interrupted pair writes recover with source and preexisting destination data retained, and external edits during recovery are never overwritten (Task 3): `recovery_after_each_fault_step_yields_one_consistent_state`, `crash_during_pointer_commit_recovers`, `crash_between_writes_with_missing_destination_removes_created`, `external_edit_during_recovery_is_recovery_required`, `startup_timeout_leaves_records_unavailable_then_ready`.
- A completed clear/delete is not resurrected from an old folder or stale in-memory copy, and persistence never blocks popup callbacks (Tasks 3–4): `merged_history_limit_zero_keeps_all_entries`, `finish_persists_outside_commit_mutex`, `persist_failure_marks_store_stale_not_popup_error`.
- UI cancellation, failed preference saves, relaunch/Quit during an operation and recovery actions preserve the active stores (Task 5): `scheduling_a_folder_does_not_switch_running_stores`, `shutdown_refuses_late_commit`.

---

### Task 1: Separate local paths, local storage preferences, strict record decoding and the records writer seam

**Files:** Modify `crates/lingopop-core/src/paths.rs`, `lib.rs`, `store_common.rs`, `history_store.rs`, `usage_store.rs`; create `storage_error.rs`, `storage_prefs.rs`, `records_snapshot.rs`, `records_writer.rs`, `tests/records_storage.rs`; modify `app/src-tauri/src/state.rs`, `lib.rs` and affected task-test callsites; extend `scripts/check-core-deps.sh` expectations only if the guard needs a new allowed module name (it must stay green with no platform dependency).

**Interfaces:**
- 1a. `LocalPaths` owns settings/ui/secrets/storage/journal/backups; `RecordsPaths::new(PathBuf)` exposes only `history_json()`/`usage_json()`. Keep `DataPaths` as a compatibility constructor mapping both locations to the same root until callers migrate. Serde `NativePath::{UnixBytes(Vec<u8>), WindowsWide(Vec<u16>)}` with `HostKind::{Unix, Windows}` and `NativePath::validate(&self, host: HostKind) -> Result<(), StorageError>` (wrong-encoding, relative and malformed values rejected; the host is injected by the shell, never `cfg!` in core; local preferences are not rewritten on rejection). `DirectoryId(Vec<u8>)` is an opaque serde id produced by the platform crate. `DirectoryRef { path: NativePath, id: DirectoryId }` (a folder replaced by a different directory at the same path between launches is detected as `ChangedDirectory`). `FolderChoice::{Default, Custom(DirectoryRef)}`, `SwitchMode::{CopyCurrent, UseDestination}`, `PendingSwitch { destination: FolderChoice, mode: SwitchMode }`, versioned `StoragePrefs { active: FolderChoice, pending: Option<PendingSwitch> }`. Core never converts `NativePath` via lossy strings. `RecordFile::{History, Usage}` and `FileRevision::{Missing, Sha256([u8; 32])}` live here (core cannot depend on platform; `scripts/check-core-deps.sh`), and Task 2 reuses them. `StorageError` derives `Debug, Clone, PartialEq, Eq` with fixed text and no raw record/path payload: `Unavailable`, `UnsupportedVersion`, `InvalidData`, `Io(std::io::ErrorKind)` (payload-free), `ChangedDirectory`, `Conflict`, `SameDirectory`, `RecoveryRequired`, `Busy` (lease/lock not acquired within its bound) and `ShuttingDown`.
- 1b. `ReadPolicy::{InitializeMissing, Established}`. Rule: the DEFAULT folder is app-owned and always reads with `InitializeMissing` (missing file = empty envelope, legacy bare array accepted and rewritten as v2 on first write, corrupt file quarantined as today); a CUSTOM folder reads with `InitializeMissing` only inside a switch transaction (`UseDestination` initializes a missing file with an empty v2 envelope and accepts a legacy bare array) and with `Established` afterwards (missing / unsupported / corrupt = `Unavailable` / `UnsupportedVersion` / `InvalidData`, never quarantined, never written). Strict per-file `decode_history(bytes: Option<&[u8]>, policy)` and `decode_usage(bytes: Option<&[u8]>, policy)` return `Result<MergeableEnvelope<HistoryEntry>, StorageError>` / `Result<MergeableEnvelope<UsageRecord>, StorageError>`; `RecordsSnapshot::decode(history: Option<&[u8]>, usage: Option<&[u8]>, policy) -> Result<RecordsSnapshot, StorageError>` combines them for pair transactions (ordinary refresh keeps the unaffected store available) and retains raw bytes/revisions. Pure `RecordsSnapshot::merged(&self, other: &Self, now: Timestamp, history_limit: u32) -> Result<RecordsSnapshot, StorageError>` using existing envelope/store normalization; `history_limit == 0` means no truncation (never "history disabled" as in `HistoryStore::add`). `RecordsWriter` trait: `replace(&self, file: RecordFile, expected: FileRevision, bytes: &[u8]) -> Result<FileRevision, StorageError>` and `read(&self, file: RecordFile) -> Result<Option<Vec<u8>>, StorageError>`. `EnvelopeFile<T>` / `HistoryStore` / `UsageStore` persist records ONLY through an injected `Arc<dyn RecordsWriter>`, never through `fsio::atomic_write` (its `create_dir_all` would silently recreate an unmounted folder). Core ships `PathRecordsWriter::new(RecordsPaths)` (temp file in the same directory + `sync_all` + rename + parent-directory fsync on Unix via `File::open(dir).sync_all()`, NO `create_dir_all`; a missing directory is `StorageError::Unavailable`) for core tests ONLY. It is path-based and cannot honour the held-directory contract, so the platform `RecordsFs` never uses it as an inner writer (see Task 2).

**Commits:**
- 1a `feat(storage): separate local configuration from record paths` owns `crates/lingopop-core/src/{paths.rs,lib.rs,storage_error.rs,storage_prefs.rs}`, the 1a half of `tests/records_storage.rs`, and `app/src-tauri/src/{state.rs,lib.rs}` path wiring.
- 1b `feat(storage): strict record decoding and records writer seam` owns `crates/lingopop-core/src/{records_snapshot.rs,records_writer.rs,store_common.rs,history_store.rs,usage_store.rs}`, the 1b half of `tests/records_storage.rs`, and the `state.rs` store construction.

- [ ] (1a) Write `records_storage.rs` regressions: `LocalPaths` secret/config methods never resolve under a custom `RecordsPaths`; default paths stay byte-for-byte compatible; `NativePath` roundtrips non-UTF8 Unix bytes and Windows unpaired units without lossy conversion; `validate(HostKind::Unix)` rejects `WindowsWide` and vice versa; absent prefs default locally, malformed/future prefs block rewriting; `DirectoryRef` serde roundtrip keeps the id bytes.
- [ ] (1b) Write regressions: `decode(None, None, Established) == Err(Unavailable)`, `decode(valid_history, future_usage, Established) == Err(UnsupportedVersion)`, zero file mutations for decoding errors; default folder reads missing files as empty and quarantines corruption as today; a custom folder under `InitializeMissing` accepts a legacy bare array and a missing file, under `Established` never quarantines; legacy/BOM, UUID deduplication and newer clear/tombstone dominance with the mac fixtures; the `ReadPolicy` matrix above.

**Regression anchor — `established_missing_records_are_unavailable`:** fixture: no history bytes, no usage bytes, `Established`; asserts on the named result.

```rust
assert!(matches!(RecordsSnapshot::decode(None, None, ReadPolicy::Established), Err(StorageError::Unavailable)));
```

**Regression anchor — `merged_history_limit_zero_keeps_all_entries`:** fixture: two snapshots holding 3 and 4 distinct history entries (no shared UUIDs), `history_limit = 0`.

```rust
let merged = a.merged(&b, now, 0).unwrap();
assert_eq!(merged.history().items().len(), 7);
```

**Regression anchor — `path_writer_missing_directory_fails_without_creating_it`:** fixture: `PathRecordsWriter` over a tempdir path that was removed after construction.

```rust
assert_eq!(writer.replace(RecordFile::History, FileRevision::Missing, b"{}"), Err(StorageError::Unavailable));
assert!(!missing_dir.exists());
```

- [ ] Run `cargo test -p lingopop-core --test records_storage --locked`; capture RED for the new routing/strict decoder/writer contracts.
- [ ] Implement codecs/paths, the writer seam and non-writing strict validation; migrate `EnvelopeFile<T>` persistence from `fsio::atomic_write` to the injected writer. Ordinary startup corruption behavior stays only for the default folder; custom folders use strict reads. Thread both path groups and the writer into `state::load` without changing secrets/config defaults.
- [ ] Run the focused command, `cargo test -p lingopop-core --test mac_fixtures --locked`, `bash scripts/check-core-deps.sh` and affected shell tests. Expected: zero failures and unchanged compatibility fixtures; no generated golden edits.
- [ ] Review and locally commit 1a and 1b separately with the messages above.

### Task 2: Directory identity, lossless paths and cooperating local writer exclusion

**Files:** Create `crates/lingopop-platform/src/records_fs.rs`, `fake/records_fs.rs`, `windows/records_fs.rs`, `macos/records_fs.rs`, `tests/records_fs.rs`; modify platform `lib.rs`, native/fake module exports and `Cargo.toml` (Windows `windows` crate features for `Win32_Storage_FileSystem` handles and `SetFileInformationByHandle` / `FlushFileBuffers`, target-scoped; `libc` is already a Unix dependency of the platform crate and is used for `openat`/`renameat`/`fstat`).

**Interfaces:** Reuse core `DirectoryId`, `DirectoryRef`, `RecordFile`, `FileRevision`, `StorageError` and `RecordsWriter` from Task 1; define only `RecordsLease: Send`. `RecordsFs: Send + Sync` supplies `identify(path: &NativePath) -> Result<DirectoryRef, StorageError>`, `lock(dirs: &[DirectoryRef], timeout: Duration) -> Result<Box<dyn RecordsLease>, StorageError>`, `open(dir: &DirectoryRef) -> Result<Arc<dyn RecordsWriter>, StorageError>` (a `RecordsWriter` bound to the directory, holding its handle for the session) and `remove_created(dir, file, expected) -> Result<(), StorageError>` for rolling back a previously missing destination only if the expected revision still matches. Identity is held through directory handles and the writer returned by `open` is platform-specific and handle-relative: `openat`/`renameat` on Unix; on Windows temp-file creation relative to the directory handle and the rename through `SetFileInformationByHandle(FileRenameInfoEx)` with the directory handle as `RootDirectory` and `FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS`, then `FlushFileBuffers` on the file and on the directory handle for durability (no path-based `MoveFileExW`, no `MOVEFILE_WRITE_THROUGH`); no by-path `create_dir_all`. If the Windows handle-relative rename cannot be verified on the CI runner, the task stops with `NEEDS_CONTEXT` instead of falling back to a path-based rename. Every `read`/`replace` re-identifies the path and compares it to the held handle (`ChangedDirectory` on mismatch); handles stay open for the session. `replace` fsyncs the file and the parent directory (Unix) before returning the new revision, so a returned revision is durable; Windows flushes the file and the directory handle with `FlushFileBuffers` after the handle-relative rename. Add core `fsio::sync_dir(path)` (std only: open the directory and `sync_all` on Unix, no-op on Windows) for the local pointer commit in Task 3. Locks are OS-released handles in the app-local lock namespace keyed by `DirectoryId` and acquired in sorted order, so stale lock files need no cleanup; never place lock files in the selected folder. Lease acquisition is bounded (2 s, injected `Sleeper`/clock) and returns `Busy`. Non-cooperating writers (the Swift app, cloud clients) are not excluded; the user-facing note is "Do not point the Swift LingoPop app at the same folder." (key listed in Task 5). macOS: the app is unsandboxed, so security-scoped bookmarks are out of scope; a TCC "Files and Folders" denial, an unmounted `/Volumes/*` path and iCloud Drive `.icloud` placeholders map to `Unavailable`; identity uses the file-system id (`st_dev` + `st_ino` via `fstat`) so case-insensitive paths and firmlinks compare equal. No trait method accepts arbitrary file names.

**Commits:**
- `feat(storage): guard record directories and revisions` owns `crates/lingopop-platform/src/{records_fs.rs,fake/records_fs.rs,windows/records_fs.rs,macos/records_fs.rs,lib.rs,native.rs,fake/mod.rs}`, `crates/lingopop-platform/tests/records_fs.rs` and `crates/lingopop-platform/Cargo.toml`.

- [ ] Write `records_fs.rs`: alias and real path yield the same identity/lock; two local processes cannot hold the same directory lease concurrently; process exit releases it; a lease that cannot be taken within 2 s on the fake clock returns `Busy`; a directory swapped after identify returns `ChangedDirectory` and does not write; a directory swapped between identification and the end of `replace` (barrier-controlled test `directory_swapped_during_replace_is_not_trusted`) is detected by re-identifying after the operation: the trusted revision is not advanced and the result is `ChangedDirectory` (or `RecoveryRequired` when the outcome is ambiguous), never success; compare-replace on a foreign revision returns `Conflict`; removing a newly created file refuses a foreign revision; a removed directory returns `Unavailable` and is not recreated; fault hooks fail the file fsync and the directory fsync separately and surface `Io`. Fake tests cover both native path encodings; native tests cover the host filesystem.

**Regression anchor — `directory_replacement_never_writes`:** fixture: identify dir A, rename A away and create a different directory at the same path, then `replace(History, Missing, b"x")` through the held handle.

```rust
assert_eq!(result, Err(StorageError::ChangedDirectory));
assert_eq!(destination_bytes_after, destination_bytes_before);
```

**Regression anchor — `alias_replacement_is_changed_directory`:** fixture: a symlink `link -> dirA` is identified; the link is repointed to `dirB` (macOS and Unix; a junction on Windows CI), then `read(History)`.

```rust
assert_eq!(fs.open(&dir_ref_via_link).unwrap().read(RecordFile::History), Err(StorageError::ChangedDirectory));
assert!(dir_b_files.is_empty());
```

- [ ] Run `cargo test -p lingopop-platform --test records_fs --locked`; capture RED, then verify chosen OS bindings against installed sources/official APIs and the macOS 13 floor.
- [ ] Implement the filesystem seam and fake fault hooks (the `FaultFs` of Task 3 wraps them). Do not promise atomic exclusion against cloud clients/noncooperating applications; revision checks detect observed changes, not a distributed CAS.
- [ ] Run focused tests on the host and require Windows CI for handle/alias semantics. Expected: no partial write on lease/conflict errors and on an identity mismatch detected BEFORE the write; for a directory swapped DURING `replace` the replacement directory stays unchanged and the trusted revision is not advanced (the detached original may have been written; no rollback is defined for it, recovery treats it as an unreferenced directory); no local-lock file under `RecordsPaths`.
- [ ] Review and locally commit with the message above.

### Task 3: Recoverable pair migration at startup

**Files:** Create `app/src-tauri/src/storage/{mod.rs,local_fs.rs,fault_fs.rs,journal.rs,migration.rs,recovery.rs,tests.rs}`; modify `lib.rs`, `state.rs`, `Cargo.toml` (direct dependency only if needed), and core `storage_prefs.rs`.

**Interfaces:**
- 3a. `LocalFs` trait for journal/backup/prefs writes: `write_atomic(path, bytes, private: bool)`, `read(path)`, `remove(path)`, `sync_dir(path)`; `StdLocalFs` wraps the existing private atomic-write helpers plus `sync_dir` (no-op on Windows). `FaultFs` is the test fake wrapping both `LocalFs` and `RecordsFs` with `fail_after(FaultStep)`, where `FaultStep` enumerates every durable operation: `JournalCreate`, `Backup(n)`, `DestinationReplace(RecordFile)`, `PointerCommit`, `JournalComplete`, `JournalRemove`, plus the typed substeps `FileFlush(step)` and `DirFlush(step)` for each durable write above, and the import steps `ImportSettings`, `ImportHistory`, `ImportUsage`, `ImportMarker` (`FaultStep::import_steps()`, used by Transition Task 18); the operation-order assertions list these steps explicitly. The journal is versioned and typed: `JournalKind::{FolderSwitch, SwiftImport}`; `SwiftImport` is filled by Transition Task 18, only the extension point is declared here. Startup order: a pending folder switch is recovered/applied first, then a pending import runs against the resulting active folder. The journal records source/destination `DirectoryRef`s, old preference bytes, preexisting destination bytes in separate private local backup files, intended output hashes and stage.
- 3b. `StorageCoordinator::new(local: LocalPaths, local_fs: Arc<dyn LocalFs>, fs: Arc<dyn RecordsFs>, clock: Arc<dyn Clock>)`; `prepare_switch(choice: FolderChoice, mode: SwitchMode) -> Result<SwitchPreview, StorageError>` reads/counts but never changes active paths; `save_pending(...) -> Result<StoragePrefs, StorageError>` writes only local prefs; `recover_and_apply(history_limit: u32) -> Result<StorageStartup, StorageError>` executes before writable record stores open. `SwitchPreview` includes source/destination identity, both revisions, merged history/usage counts and applied retention counts. `StorageStartup { paths: RecordsPaths, status: StorageStatus }`; `StorageStatus::{Ready, Pending, MigrationFailed, RecoveryRequired}` never disguises failure as success. Before Transition Task 18 lands, a pending `SwiftImport` journal is preserved untouched: startup reports `RecoveryRequired`, keeps the writable record stores closed (records `Unavailable`) and never reports recovery complete; regression test `pending_swift_import_journal_keeps_stores_closed_before_task_18`. Durable order: backups, destination writes (each fsynced), then the `storage.json` pointer commit (atomic write + `sync_dir`; the pointer commit uses the same handle-relative primitive as `RecordsFs`, in a shared platform helper: Windows opens the local directory handle, creates the temp file relative to it, renames through `SetFileInformationByHandle(FileRenameInfoEx)` with `RootDirectory` and `FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS`, then `FlushFileBuffers` on the file and the directory handle; Unix uses `renameat` and an fsync of the directory fd; so `sync_dir` being a no-op on Windows does not weaken durability), then journal completion, which starts only after the pointer commit returns. A failure of either `FlushFileBuffers` (file or directory handle) on Windows, or of the directory fsync on Unix, is an error: it prevents journal completion and removal, the journal stays and the next startup recovers (regression `flush_failure_keeps_journal_and_recovers_on_startup`, with `FaultFs` failing each flush step in turn). Microsoft documentation does not explicitly guarantee rename durability through flushing a directory handle, so the task must verify the sequence on Windows (a CI test that the directory-handle flush succeeds); if the OS refuses `FlushFileBuffers` on a directory handle there is no silent fallback: the task stops with `NEEDS_CONTEXT` and records it in the task report for the owner. A Windows crash test covers the pointer commit, journal completion and journal removal steps for PROCESS crash only; power-loss durability is a stated limitation and is not claimed. `MigrationFailed` never auto-retries at launch and waits for a user action (Task 5); `RecoveryRequired` blocks writable records until a user action. Startup: recovery runs in `setup` after the single-instance plugin, never in the `--wait-for-pid` prelude (a waiting replacement process must not touch records), on a dedicated std thread with a bounded 10 s wait; on timeout the app continues with records `Unavailable` and status `Pending`, and the thread publishes `Ready` / `MigrationFailed` / `RecoveryRequired` when it finishes, at which point the stores open. While status is `Pending`, `storage_schedule`, `storage_use_default`, `storage_cancel_pending` and `storage_resolve` are rejected with a fixed busy key (they cannot race the recovery thread's journal or pointer commit); regression `storage_mutation_rejected_while_recovery_pending`.

**Commits:**
- 3a `feat(storage): local filesystem seam, journal and fault injection` owns `app/src-tauri/src/storage/{mod.rs,local_fs.rs,fault_fs.rs,journal.rs}` and the journal/`FaultFs` tests in `storage/tests.rs`.
- 3b `feat(storage): journal record folder switches across relaunch` owns `app/src-tauri/src/storage/{migration.rs,recovery.rs}`, the recovery tests, `app/src-tauri/src/{lib.rs,state.rs}` and `crates/lingopop-core/src/storage_prefs.rs`.

- [ ] (3a) Write journal tests: versioned roundtrip, unknown version refused, `SwiftImport` kind decodes but is not executed here; `FaultFs` fails exactly at the requested `FaultStep` and records the order of durable operations.
- [ ] (3b) Write the recovery matrix: `recovery_after_each_fault_step_yields_one_consistent_state` iterates every `FaultStep`, restarts, and asserts either the old selection with the original destination restored or the complete new pair, never a mixed active pair; repeat recovery is idempotent, source bytes never change, corrupt/future destination refuses writes, `SameDirectory` is a no-op, `UseDestination` leaves the source untouched, merged UUIDs/deletions/clear markers survive; a startup that exceeds the 10 s bound.

**Regression anchor — `crash_after_history_replace_keeps_a_complete_selection`:** fixture: source folder S (3 history, 2 usage), destination D with preexisting history H0 and usage U0; `fail_after(DestinationReplace(History))`, then restart.

```rust
assert_eq!(recovered.active_folder, old_folder);
assert_eq!(recovered.destination_pair, (h0_bytes, u0_bytes));
assert_eq!(source_after, source_before);
```

**Regression anchor — `crash_during_pointer_commit_recovers`:** fixture: both destination files installed, `fail_after(PointerCommit)` leaves `storage.json` either old or new (run both outcomes), then restart.

```rust
assert!(recovered.status == StorageStatus::Ready);
assert!(pair_is_complete(&recovered) || pair_is_original(&recovered));
assert_eq!(recovered.active_folder == new_folder, pointer_names_new);
```

**Regression anchor — `crash_between_writes_with_missing_destination_removes_created`:** fixture: D is empty, `fail_after(DestinationReplace(History))` with `CopyCurrent`, pointer still old.

```rust
assert!(!d.join("history.json").exists());
assert!(!d.join("usage.json").exists());
assert_eq!(recovered.active_folder, old_folder);
```

**Regression anchor — `external_edit_during_recovery_is_recovery_required`:** fixture: crash after history replace, then a foreign process rewrites D's `history.json` before restart so its hash differs from the journal-installed hash.

```rust
assert_eq!(recovered.status, StorageStatus::RecoveryRequired);
assert_eq!(d_history_after, foreign_bytes);
```

**Regression anchor — `startup_timeout_leaves_records_unavailable_then_ready`:** fixture: `FaultFs` stalls the first destination read past the 10 s bound on the paused clock, then releases it.

```rust
assert_eq!(state_at_timeout.status, StorageStatus::Pending);
assert!(state_at_timeout.history_store.is_none());
assert_eq!(state_after_release.status, StorageStatus::Ready);
assert!(state_after_release.history_store.is_some());
```

- [ ] Run `cargo test -p lingopop-app storage:: --locked`; capture RED.
- [ ] Implement journal-first durability, sorted dual-directory leases, strict reads, local backup/prepare, checked pair replacement and verification, then the pointer commit and journal completion. Clear pending only with a verified active commit. Recovery policy is deterministic: if the durable pointer still names the old folder, roll back only destination files whose hash matches journal-installed hashes (and remove files the journal created); if it names the new verified transaction, complete bookkeeping instead of rolling back a legitimately active folder; any hash mismatch is `RecoveryRequired`. Recovery ambiguity blocks writable record stores but keeps local settings and recovery UI available. Register startup recovery in `setup` after single-instance and before persistence hooks/hotkeys can produce records. Never use quarantine-to-empty on selected data.
- [ ] Run the failure matrix and filesystem tests; expected zero failures and zero source mutations. Inspect temp files/backups: only managed record temp files may be in the selected folder; all backups/journal metadata remain local.
- [ ] Review and locally commit 3a and 3b separately with the messages above.

### Task 4: Safe external reconciliation, store availability and persistence outside the commit mutex

**Files:** Modify core `store_common.rs`, `history_store.rs`, `usage_store.rs`; create core `tests/records_reconcile.rs`; modify orchestrator `orchestrator.rs`, `fake.rs` and `tests/orchestrator_text.rs`; modify shell `state.rs`, `windows.rs`, `history_cmds.rs`, `usage_cmds.rs`, `tray.rs`, `storage/mod.rs`; create shell `storage/reconcile_tests.rs`.

**Interfaces:**
- Core: `HistoryStore::reconcile(snapshot: RecordsSnapshot) -> Result<bool, StorageError>` and `UsageStore::reconcile(snapshot: RecordsSnapshot) -> Result<bool, StorageError>` use only their corresponding envelope; `bool` means the visible snapshot changed, not that a write happened. Shell `HistoryWriteError` maps from `StorageError`.
- Two-lock rule in `ShellStores`: `records_view` (std mutex, in-memory snapshot, never any I/O under it, read by tray/History) and `records_io` (held only on blocking threads, never under the orchestrator commit mutex; serializes lease + refresh + reconcile + checked replace). `ShellStores::refresh_records() -> Result<(), StorageError>` (bounded by the lease timeout) and `records_status() -> RecordsStatus` expose per-store Ready/Unavailable/Stale with fixed user-message keys; strict per-file decoding from Task 1 isolates a bad history file from valid usage and vice versa.
- `Persist::add_history`, `append_usage` and `trim_history` become `async` (`async_trait`). `Orchestrator::finish` builds the history entry / usage record and presents under the commit mutex (generation check), releases it, then awaits persistence on `spawn_blocking`; a persist failure is logged with fixed text and surfaced through `records_status()` (`Stale` / `Unavailable`), never as a popup error. Notifications fire after the view swap and after all locks are released. `FakePersist` records call order so `present` is observed before `persist`.
- Refresh wiring: `windows.rs` `Focused(true)` of the `settings` and `history` windows and `history_list` call `refresh_records()`; the tray rebuild reads `records_view` and `records_status()` only and never refreshes. The spec's bounded recheck is "on activation and before every mutation", with no timer. Migration preview (`storage_preview`, Task 5) is informative only and revalidated at relaunch.

**Commits:**
- `feat(storage): reconcile external records before committing mutations` owns core `store_common.rs`, `history_store.rs`, `usage_store.rs`, `tests/records_reconcile.rs`, shell `state.rs`, `storage/{mod.rs,reconcile_tests.rs}`, `history_cmds.rs`, `usage_cmds.rs`, `windows.rs`, `tray.rs`.
- `refactor(orchestrator): persist records outside the commit mutex` owns orchestrator `orchestrator.rs`, `fake.rs`, `tests/orchestrator_text.rs`, `tests/orchestrator_actions.rs`, `tests/orchestrator_screen.rs` and the shell `Persist` impl in `state.rs`.

- [ ] Write core and shell tests: external append becomes visible without watcher writes; add/delete/clear/reset first merges the current disk revision; deleting locally then seeing an old envelope does not resurrect the item within retention; changed clear watermark wins; missing, corrupt, future or denied external read retains the last trusted snapshot but rejects mutation; failed saves do not fire success hooks; reconnect plus explicit refresh restores availability; an external replacement between read and checked replace returns `Conflict`, not overwrite. Pin 31-day usage policy and the 90-day tombstone horizon rather than claiming unbounded offline reconciliation.

**Regression anchor — `corrupt_external_history_keeps_usage_available`:** fixture: trusted history T and valid usage loaded; the external `history.json` is overwritten with `{not json`; call `refresh_records()`, then `add_history(e)`.

```rust
assert_eq!(history_after, last_trusted_history);
assert_eq!(history_mutation, Err(StorageError::InvalidData));
assert!(usage_is_available);
assert_eq!(success_hook_count, 0);
```

**Regression anchor — `unsupported_envelope_refuses_writes`:** fixture: established custom folder whose `history.json` is `{"version": 99, "items": []}`.

```rust
assert_eq!(store_open_result, Err(StorageError::UnsupportedVersion));
assert_eq!(history_bytes_after, history_bytes_before);
assert!(no_bak_files_in_folder);
```

**Regression anchor — `folder_removed_mid_session_marks_unavailable_without_write`:** fixture: running stores on a custom folder, then the folder is deleted (fake: `FakeRecordsFs::remove_dir`), then `add_history(e)`.

```rust
assert_eq!(add_result, Err(StorageError::Unavailable));
assert_eq!(records_status().history, StoreState::Unavailable);
assert!(!folder.exists());
assert_eq!(last_trusted_history_view, before);
```

**Regression anchor — `read_only_folder_mutation_fails_without_quarantine`:** fixture: valid `history.json` in a folder whose writes return `Io(PermissionDenied)` (fake) and whose read of the file is intact.

```rust
assert_eq!(add_result, Err(StorageError::Io(std::io::ErrorKind::PermissionDenied)));
assert!(folder_listing_has_no_bak_files);
assert_eq!(history_bytes_after, history_bytes_before);
```

**Regression anchor — `finish_persists_outside_commit_mutex`:** fixture: `FakePersist` whose `add_history` awaits a never-released gate; run a Translate flow to `Result`, then call `popup_resized(..)` and `close()`.

```rust
assert!(popup_calls.contains(&PopupCall::ShowAt(..)));   // presented before persist
assert!(timeout(100ms, orchestrator.popup_resized(size)).await.is_ok());
assert!(timeout(100ms, orchestrator.close()).await.is_ok());
assert_eq!(persist_order, ["present", "persist_started"]);
```

**Regression anchor — `persist_failure_marks_store_stale_not_popup_error`:** fixture: `FakePersist` returns `Err(StorageError::Io(..))`; run a Translate flow.

```rust
assert!(matches!(last_popup_model.phase, PopupPhase::Result { .. }));
assert_eq!(records_status().history, StoreState::Stale);
assert_eq!(logged_lines_contain_user_text, false);
```

- [ ] Run `cargo test -p lingopop-core --test records_reconcile --locked`, `cargo test -p lingopop-orchestrator --locked` and `cargo test -p lingopop-app reconcile --locked`; capture RED.
- [ ] Implement reconcile-before-mutate with bounded conflict retry (three attempts, then actionable `Conflict`), no watch-triggered rewrites, and the activation/pre-mutation refresh. No polling loop. Never automatically redirect unavailable selected stores locally. Keep ordinary explicit Copy to clipboard usable while storage is unavailable.
- [ ] Run focused tests and existing `history_cmds`/usage/orchestrator commit tests. Expected zero failures; callbacks can read stores without deadlock; no stale success events or generation-regressed writes.
- [ ] Review and locally commit both commits with the messages above.

### Task 5: Folder settings, native selection, recovery actions and shutdown

**Files:** Create shell `storage/commands.rs`, `storage/dialog.rs`, `storage/command_tests.rs`; modify shell `lib.rs`, `settings_cmds.rs`, `commands.rs`, `tray.rs`, `capabilities/default.json`, `Cargo.toml`, `tauri.conf.json` only if the plugin needs registration (CSP unchanged); modify orchestrator `orchestrator.rs`/`lib.rs` and focused shutdown tests; create frontend `lib/storageTypes.ts`, `lib/storageIpc.ts`, `settings/StorageSection.svelte`, `settings/storage.test.ts`; modify `settings/MainTab.svelte`, `settings/Relaunch.svelte`, `i18n/windows.json`, `AGENTS.md`, `PRIVACY.md`, `CHANGELOG.md`.

**Interfaces:**
- 5a. `Orchestrator::prepare_shutdown() -> impl Future<Output = ()>`: sets a `shutting_down` flag in `SessionState` checked by `finish` under the commit mutex (new operations are refused with `ShuttingDown`), cancels the current generation, awaits native pick/presentation cleanup and in-flight persistence (bounded 3 s) outside session/commit locks, then releases leases. A timeout never releases leases or the commit fence: they stay owned by the persistence job until it exits (a `spawn_blocking` job cannot be cancelled). After a timeout `app_relaunch` refuses to spawn the replacement and returns an error key, and tray Quit proceeds to `app.exit(0)` (process exit ends the job and frees the OS-released locks). It is wired into BOTH `app_relaunch` (before spawning the `--wait-for-pid` replacement) and tray Quit (`app.exit(0)` only after it returns). Storage Task 5 ships before Mac Task 13; the final-Hide-ack wait is added by Task 13.
- 5b. Async IPC `storage_get() -> StorageView`, `storage_choose() -> Option<FolderSelection>`, `storage_preview(selection_id: Uuid, mode: SwitchMode) -> SwitchPreview`, `storage_schedule(selection_id: Uuid, mode: SwitchMode) -> StorageView`, `storage_use_default(mode: SwitchMode) -> StorageView`, `storage_cancel_pending() -> StorageView`, `storage_open() -> ()`, `storage_cleanup_backups() -> ()` and `storage_resolve(action: RecoveryAction) -> StorageView` with `RecoveryAction::{KeepDestination, RestoreBackups, AbandonPending, Retry}` for `MigrationFailed` / `RecoveryRequired`; each action is a journal-driven transaction covered by the same `FaultFs` matrix as Task 3, and the UI shows the four actions with explanations. All return typed fixed-key errors. `FolderSelection { id, displayPath }` is an opaque backend-held native-path capability, never a lossy path roundtrip or arbitrary frontend path. `StorageView { activePath: String, pendingPath: Option<String>, status: StorageStatus, records: RecordsStatus, relaunchNeeded: bool, backupCount: u32 }` (`records` is Task 4's per-store `RecordsStatus` with its `StoreState` for history and usage, so the UI can tell an unavailable or stale store from a migration state; `status` stays the separate migration state) carries display labels only. `StorageDialog` is an injected async chooser backed by `tauri-plugin-dialog` pinned to an exact verified version, with capability `dialog:allow-open` granted only to the `settings` window; CSP unchanged. Use the existing settings-write serialization for local preference changes. `storage_open` opens only the configured backend path.
- New English keys (all added with pl/uk/de to `i18n/windows.json`; the file keeps its name, macOS-only keys later go to `i18n/macos.json` per Mac Task 6; `catalogCoverage.test.ts` must pass): `History and statistics folder`, `Change folder…`, `Open folder`, `Use default folder`, `Copy current data`, `Use destination data`, `Changes take effect after restarting LingoPop.`, `This folder stores history and statistics only.`, `Use one device at a time and wait for your sync app to finish.`, status `Ready`, `Pending`, `Unavailable`, `Stale`, `Migration failed`, `Recovery required`, recovery actions `Keep destination data`, `Restore backups`, `Abandon pending change`, `Retry` (with one-sentence explanation keys for each), `Cancel`, `Clean up backups`, `Restart`, preview strings `%lld history entries, %lld usage records after merge` and `Entries older than 90 days and usage older than 31 days are not carried over.`, and the Swift note `Do not point the Swift LingoPop app at the same folder.`.

**Commits:**
- 5a `feat(storage): coordinated shutdown before relaunch and quit` owns orchestrator `orchestrator.rs`/`lib.rs`, `tests/orchestrator_actions.rs` (shutdown tests), shell `commands.rs` (`app_relaunch`) and `tray.rs` (Quit).
- 5b `feat(settings): choose history and statistics storage folder` owns shell `storage/{commands.rs,dialog.rs,command_tests.rs}`, `lib.rs`, `settings_cmds.rs`, `Cargo.toml`, `capabilities/default.json`, frontend `lib/storage*.ts`, `settings/{StorageSection.svelte,storage.test.ts,MainTab.svelte,Relaunch.svelte}`, `i18n/windows.json`, `AGENTS.md` (a Layout bullet for every new module of Tasks 1–5), `PRIVACY.md` and `CHANGELOG.md` `Unreleased` notes (records may now live outside `%APPDATA%` / Application Support).

- [ ] (5a) Write tests: `shutdown_refuses_late_commit` (a pending operation that finishes after `prepare_shutdown` begins cannot commit or persist and returns `ShuttingDown`), persistence in flight is awaited up to 3 s, leases are released after it finishes, `persistence_over_3s_keeps_leases_and_blocks_relaunch` (a persistence job that exceeds 3 s keeps the leases, cannot be followed by a replacement spawn and commits nothing after Quit starts), tray Quit and `app_relaunch` both call it before exit.
- [ ] (5b) Write command and frontend tests: cancelled chooser performs zero writes; `CopyCurrent` is selected initially; preview counts/retention appear before confirmation; `UseDestination` and `UseDefault` are explicit; schedule changes pending only, never active stores; failed preference save restores the UI; each `RecoveryAction` runs its journal transaction and survives the fault matrix; `Retry`/`Cancel` after migration failure retains the old selection; stale dialog results cannot override a newer choice; cleanup refuses an active journal's backups; no keys/config paths appear in storage IPC or created destination files; exact new English keys and pl/uk/de coverage.

**Regression anchor — `scheduling_a_folder_does_not_switch_running_stores`:** fixture: running app on the default folder, `storage_choose` returns a selection for folder F, `storage_schedule(id, CopyCurrent)`.

```typescript
expect(view.relaunchNeeded).toBe(true);
expect(view.activePath).toBe(oldPath);
expect(view.pendingPath).toBe(chosenPath);
expect(recordWrites).toHaveLength(0);
```

**Regression anchor — `shutdown_refuses_late_commit`:** fixture: a Translate flow blocked on its provider; `prepare_shutdown()` starts, then the provider answers.

```rust
assert_eq!(finish_result, Err(StorageError::ShuttingDown));
assert_eq!(persist_calls, 0);
assert!(leases_released);
```

- [ ] Run `cargo test -p lingopop-app storage:: --locked`, `cargo test -p lingopop-orchestrator --locked` and `npm test --prefix app -- src/settings/storage.test.ts`; capture RED, implement 5a then 5b, then rerun. Native dialog work runs outside callback locks; file reads/writes run off the UI thread. Startup migration does all record switching.
- [ ] Run `python3 scripts/gen-localizations.py check` and `npm test --prefix app -- src/lib/catalogCoverage.test.ts`; expected pass with every new key in `i18n/windows.json`.
- [ ] Verify real chooser, Open Folder, keyboard navigation and displayed geometry in an isolated application/browser environment when execution authorizes browser work; otherwise record native/UI validation pending. Do not infer real-render coverage from jsdom.
- [ ] Run the shared PR gate and both CI legs; expected all required checks green, Windows default data untouched.
- [ ] Review and locally commit 5a and 5b separately with the messages above.
