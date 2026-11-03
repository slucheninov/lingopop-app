# LingoPop: production macOS and Windows on Rust/Tauri

Date: 2026-11-03
Status: APPROVED by the owner on 2026-11-03. Implementation planning follows; execution method and task sequencing are agreed separately.

## Intent and owner decisions

LingoPop moves to this Rust/Tauri repository as the actively developed product for both macOS and Windows. The existing Swift macOS application remains available and is frozen as a behavioral reference and fallback. Do not archive its repository, disable workflows, remove releases, uninstall it, or change its data as part of this design.

Confirmed by the owner:

- Develop one Rust/Tauri codebase for macOS and Windows.
- Keep and freeze the existing Swift app, then switch to Rust/Tauri.
- No built-in iCloud synchronization.
- Allow choosing a storage folder for history and usage statistics only.
- Settings, UI preferences, hotkeys, credentials and provider connection settings stay local to each device.

This document supersedes the original design's Windows-only product scope and its rule that macOS is a never-shipped development target. Existing safety, format compatibility, and Windows behavior constraints remain in force. Windows alpha.7 was released independently at `187507c2f413f8377922ab5deb90f9e45f400b8e`; it is the implementation baseline.

## Approaches considered

1. Keep Swift on macOS and Rust/Tauri on Windows: less initial porting, but duplicates feature work, UI and release maintenance. Owner rejected this as the long-term direction.
2. One Rust/Tauri product with platform adapters: shared core, orchestration, UI, localization and release version; native OS behavior behind platform seams. Owner selected this approach.
3. Replace native OS behavior with generic webview behavior: does not satisfy selection capture, focus-preserving popup and screen translation requirements. Rejected by the author.

## Current implementation and gaps

The workspace already separates core, orchestration, platform and Tauri shell. CI runs frontend and Rust checks on macOS and Windows, but the release workflow produces only a Windows portable exe.

The macOS platform module explicitly declares itself development-only. Current gaps include:

- `native.rs` selects `DevPlaintextProtector` on macOS. This must never be used in a shipped Mac build.
- Clipboard snapshots preserve only text; restoring them discards rich text, images and file representations.
- Foreground handles track application PIDs rather than a verified source window/element.
- Popup show/hide is Unsupported; dismiss monitoring and launch-at-login are no-ops.
- Capture, OCR and region overlay are Unsupported.
- Monitor coordinates are logical points and work areas include the menu bar/Dock. They do not satisfy the existing physical-coordinate assumptions throughout capture and popup placement.
- Hotkey labels and UI wording assume Windows even though the native layer maps Win to Command.
- Localizations are generated from the Swift checkout, which cannot remain an active authoring dependency after that app is frozen.
- Shell paths currently use `LingoPop-dev` on macOS, and one `DataPaths` currently places every file in one directory.

These are product work, not merely release-workflow changes.

## Shared and platform-specific architecture

Keep the Rust core OS-independent and the orchestrator free of Tauri/OS dependencies. Shared operations, provider clients, settings logic, history/usage schemas, prompts, pricing and Svelte UI remain single implementations.

OS adapters own permissions, clipboard, synthesized input, foreground targeting, native popup integration, outside-click/Escape observation, monitor conversion, selection overlay, screenshot capture, OCR, secret protection and login items. Necessary platform-specific UI appears only on that platform.

Use Rust bindings to the native frameworks as the default implementation approach. A narrowly scoped native bridge is permitted only where inspection demonstrates a binding gap; do not create a second Swift application or duplicate business logic.

Preserve callback constraints: enqueue async work from UI/native callbacks; do not block event loops, hold standard mutexes across awaits, or hold a commit lock while dispatching to the main thread. Keep the orchestrator's generation/commit guarantees for supersession, cancellation and persistence.

## macOS behavior required before general availability

- Menu-bar application without an ordinary Dock presence; Settings and History remain reachable.
- Capture selected text through the existing orchestration seam, with correct modifier release and permission errors.
- Snapshot all readable pasteboard items and types, preserve item boundaries and protection markers, and restore only when the observed change sequence permits it. Do not promise restoration of unreadable/promised representations: retain an explicit supported/failure policy and avoid destructive partial restoration.
- Replace returns to the captured source target; detect target loss and avoid pasting into another app or the application's own popup.
- Native popup can appear without stealing focus; Copy, Replace, Retry, Escape, outside-click dismissal and selectable text work with native lifecycle ordering.
- Establish and test one coordinate contract at the platform boundary for logical points, physical pixels, image rectangles, monitor origins and backing scales. Mixed Retina/non-Retina displays, menu bar/Dock, negative origins and display changes are acceptance cases.
- Screen Translate includes region selection, cancellation/cleanup, screenshot capture, native OCR and vision-provider retry. It does not silently turn into a successful empty result when permissions or OCR are unavailable.
- Explain Accessibility and Screen Recording requirements with localized actions and retry after access changes. Do not request unrelated permissions preemptively.
- Platform-specific shortcut display, recording, defaults, conflicts and modifier mapping. Preserve existing Windows serialization; Command must not appear as Win in Mac UI. Support the existing Swift Translate shortcut Option+T by default on a fresh Mac install; verify registration and text-input behavior.
- Functional login item behavior and a stable installed app identity/path. Test repeated launches and upgrades with permission grants.

## Local secrets and application identity

Windows keeps DPAPI and its existing data directory and bundle identity.

Author proposal for macOS: implement a `SecretProtector` using authenticated encryption with a random encryption key held in the user's macOS Keychain. The encrypted payload stays in the app's local data directory. Never fall back to plaintext if Keychain access, encryption or persistence fails. Keep fake protectors test-only; prevent production selection of `DevPlaintextProtector` with a build-time guard and a native-bundle regression check.

Use a separate stable Mac identifier, proposed `app.lingopop.tauri.macos`, and local data directory `~/Library/Application Support/LingoPop-Tauri`. Preserve the Swift identity `com.lingopop.app` and its existing files. The Mac preview is named LingoPop Preview so both installations can coexist; the final display name may become LingoPop without changing identifier or data path. Do not silently import plaintext dev credentials from `LingoPop-dev`.

No secrets, encryption keys, provider settings, UserDefaults exports or credential-bearing logs are written into the selected history/usage folder. Secret and URL debug redaction remains a cross-cutting requirement.

## Choosing the history and statistics folder

### Scope and local configuration

The selected folder contains the managed files `history.json` and `usage.json` only, plus temporary atomic-write files required for those files. App settings, UI preferences, folder selection, hotkeys, credentials, logs, migration journals and safety backups remain local. Existing unrelated files in a chosen folder are untouched.

Add a local storage preference separate from the Swift-compatible `Settings` payload, proposed `storage.json`. Its optional absolute history/usage directory defaults to the existing local app directory. Distinguish `LocalPaths` from the selected `RecordsPaths`; the core receives paths and never selects a system directory.

UI in Settings shows the current folder, a native folder chooser, Open Folder, and Use Default. Store platform-native paths losslessly; macOS and Windows selections are independent even when the user points both at the same external service. Resolve equivalent folder paths to avoid migrating into the same physical directory through an alias/symlink. Do not silently follow a replaced folder identity during a migration.

### Switching and migration protocol

Author proposal: changing the folder takes effect on relaunch, avoiding live replacement of stores during an operation. The UI explicitly says so. Saving a pending choice must not change the running stores.

1. Select destination and choose either Copy current data or Use destination data. Copy current data is the default. Returning to the default directory follows the same flow.
2. At the next startup, before any operation can write records, obtain local exclusivity for this app and prevent another local Rust/Tauri instance from writing the source/destination records during the switch.
3. Revalidate source/destination identity and both files. An existing unreadable, corrupt or unsupported destination is an error, never permission to start empty, quarantine automatically, or overwrite it. Do not reuse the ordinary startup corruption-to-empty path for migration validation.
4. For Copy current data, merge source and destination envelopes using UUID identity, tombstones and clear watermarks; never concatenate arrays or discard destination entries. Apply existing history limit, usage retention and tombstone retention policies explicitly and show the resulting counts. For Use destination data, leave source content untouched and apply the same validation.
5. Persist recoverable local transaction metadata and local backups of preexisting destination files. Prepare both outputs, atomically replace each file, verify both, then atomically commit the local active-folder preference. Atomic file replacement alone is not a two-file transaction: use the journal to recover an interruption between files or before the pointer commit.
6. Until both files are verified and the pointer is committed, the old folder remains authoritative. Startup recovery completes or restores the pending transaction before loading writable stores; never publish a mixed history/usage selection. Retry must be idempotent and preserve UUIDs, deletions and clear markers.
7. Keep source files untouched. After success, the app reads/writes only the selected folder. Do not automatically delete old copies or continuously merge them back, which could resurrect deletions. Local recovery backups have an explicit user-visible cleanup path.

If folder validation or migration fails, preserve the old active selection and all source data, show an actionable error, and allow retry/cancel/choose another folder. Do not claim the pending selection was applied.

### External changes and unavailable storage

The app supplies file storage, not an iCloud API or cloud-account integration. A user may choose a folder managed by another application. This does not make cloud replication a distributed transaction.

Author proposal for supported behavior: reload external valid changes on activation and before mutations, reconcile envelopes before writing, and emit existing History/Usage events. Serialize cooperating local processes through an OS-backed file lock; revalidate the on-disk revision before commit. Implement a minimal watcher or bounded recheck for external replacement when needed, without rewriting files merely because a watcher fired.

Malformed, unreadable or unsupported externally changed files keep the last trusted in-memory snapshot visible as stale/unavailable and block destructive writes to that store. Do not quarantine or overwrite an intermediate cloud download, or silently redirect writes to the default folder. Missing files in a previously established selected folder are treated as unavailable until explicitly resolved; a newly chosen empty folder is initialized only through the switch protocol.

The supported first release does not guarantee lossless simultaneous writes from disconnected devices, nor automatic resolution of provider-created conflict-copy files. State this limitation where the folder is selected, succinctly. Sequential use after the external client finishes syncing is the supported cloud-folder scenario. Tombstones/clear watermarks remain necessary for deterministic merges; account for their existing 90-day horizon when testing stale replicas. This project does not add its own network sync service.

## Transition from Swift

Keep Swift runnable and its data unchanged. The Rust Mac app offers an explicit one-time import rather than silently taking over the existing bundle or directory. Require the Swift app to be quit during import; do not install a second set of active global hotkeys automatically while it is running.

First-release import scope proposed by the author:

- History and usage from a user-selected existing local or previously cloud-managed folder, using the established legacy/envelope codecs and a local journal. No iCloud-container discovery or API dependency.
- Local ordinary settings from the Swift UserDefaults domain `com.lingopop.app`, read through native APIs without exporting the full domain. Whitelist fields, translate supported hotkeys, report unsupported ones, and preserve Rust defaults for absent fields.
- Keys and provider connection overrides require deliberate re-entry in the first Rust Mac release. Do not decode or copy the Swift AES-GCM `secrets.dat`, salt or machine-key files, and do not copy encrypted bytes into the new protector. UI clearly explains this before import; Swift keeps the originals. A future secure credential importer is separate work.
- Never import old iCloud flags or the obsolete cloud settings file. No edits to Swift UserDefaults or its files, no deleting its login item, and no uninstalling it. Explain how to disable the old app's login item/hotkeys when switching.
- Retrying or cancelling import does not duplicate records or overwrite newer Rust settings. Store import completion locally only after verified persistence. Recovery scenarios mirror the folder transaction guarantees.

User-requested freeze is a development policy, not a request to archive the Swift GitHub repository. Update this repository's README/AGENTS and the design scope after approval; leave Swift untouched unless separately requested.

## Localization ownership

Promote the currently committed translation values to authoritative catalogs in this repository after approval; record provenance. Normal generation and CI no longer require a Swift checkout. Existing strings remain intact, and newly shared/macOS/Windows strings have clear platform applicability. Retain placeholder validation. Allow Command symbols in Mac-only values while continuing to reject them in Windows values. Compatibility fixtures may stay as pinned historical fixtures; their regeneration is optional reference maintenance, not a build dependency.

## Packaging and release

One Rust/Tauri workspace version for Windows and macOS. Add native macOS arm64 and x86_64 release artifacts, initially separate `.dmg`/`.zip` outputs rather than requiring a universal binary. Preserve Windows portable exe behavior and asset names. Every artifact is checked against a checksum manifest with unique architecture/platform names.

Proposed minimum macOS target: 13.0, matching the frozen Swift app. Before committing to it in code, verify all pinned dependencies and chosen native APIs support it, and use compatibility paths where necessary. If infeasible, return a concrete OS-support decision to the owner; do not silently raise the minimum.

Mac preview packaging must use a consistent signing strategy that permits repeatable permission testing. Public alpha may use the existing project's unsigned/ad-hoc distribution model with accurate installation instructions. Developer ID signing/notarization is a separate distribution gate when the owner supplies an Apple Developer identity; do not assume credentials or membership exist, and do not block internal preview evaluation on obtaining them. Do not claim an ad-hoc preview has a stable trusted upgrade experience; verify and document permission re-grant behavior.

General availability requires either Developer ID signed/notarized artifacts or an explicitly owner-approved ad-hoc distribution exception. This is a release-readiness choice, not a requirement to buy anything during implementation.

Release jobs build and validate all selected platform assets before publishing the combined release. Updates choose the current OS/architecture asset and remain within this repository. Initial previews do not auto-replace Swift installations. Repository description, support documentation, screenshots/PR-recorded release checks and privacy/security documents must describe the actual shipped platforms and local/cloud-folder behavior accurately.

## Verification and rollout boundaries

- Shared deterministic tests: record routing, versioned configuration, safe decoding, import/merge, tombstones and clears, failed writes, and events.
- Storage fault injection: crash after every journal/write/preference boundary, read-only/disconnected folders, malformed or future files, cloud-style replacements, aliases, local competing writers, clear/delete after a switch, and pending operations at relaunch.
- Security checks: native Mac bundle selects a real protector, no plaintext fallback or secret exports, sentinel redaction tests, no secret/config files in the selected folder.
- Mac unit/fake tests plus real installed-app tests for permissions, clipboard formats, focus, fullscreen/Spaces, multiple monitors/scales, hotkeys, popup lifecycle, Screen Translate, login and repeat upgrades. Static code review and a successful CI build do not prove these behaviors.
- Existing Windows CI and desktop smoke stay required. Validate new folder UI and all old capture/Replace/overlay paths on Windows, not just macOS.
- Build and inspect arm64/x86_64 artifacts and verify checksums; report runtime coverage separately for each architecture.
- Introduce a Mac preview before replacing Swift as the recommended app. General availability follows real-machine acceptance and import recovery checks.

## Scope excluded

Built-in iCloud/cloud authentication or synchronization service; shared settings/hotkeys/credentials; Linux support; modifying frozen Swift behavior; automatic Swift encrypted-secret migration; a promise of cross-device transactional writes; and renaming the GitHub repository.

## Decisions explicitly left to the builder

Concrete Rust bindings/native APIs after primary-source and local dependency verification; type/module names; watcher library; local journal encoding and lock implementation; native permission window details; exact task/PR decomposition and test fixtures. Preserve all behavioral contracts above. The owner has approved this design; implementation plans are reviewed separately.

## Approval checkpoints

Owner approved this design on 2026-11-03, including the proposed relaunch-based folder switch, explicit Swift import with credential re-entry, external-folder concurrency limits, Mac minimum OS/architectures, and preview distribution policy. Next: review the implementation plans and choose execution method before product changes.
