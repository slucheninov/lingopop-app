# In-app updates: design

Date: 2026-11-06. Status: draft for owner review.

## Intent

Today an update means: notice the banner in Settings > About, open the release page, download, replace the app by hand. On macOS every ad-hoc build is a new app for the privacy database (TCC), so Accessibility and Screen Recording stop working after each update and must be granted again.

The owner wants: **press Check for updates, see the new version, press Update, then Restart, and the app comes back as the new version, with Accessibility and Screen Recording still working.**

Success = a user on version N updates to N+1 from Settings > About in a few clicks, on Windows and on macOS, and on macOS the text operations, Replace, Esc and Screen Translate work right after the restart without opening System Settings.

## Decided by the owner

- The flow lives in **Settings > About**: Check for updates -> new version shown -> **Update** -> **Restart** -> the app restarts as the new version.
- **macOS permissions must survive updates.** Releases are signed with **one long-lived self-signed code-signing certificate** (owner's choice over a paid Apple Developer ID).
  - Verified on 2026-11-06 with a throwaway bundle: two different builds signed with the same self-signed certificate share the designated requirement `identifier "<bundle id>" and certificate leaf = H"<sha1>"`, and Accessibility and Screen Recording granted to one stayed granted for the other.
  - An ad-hoc build has the requirement `cdhash H"..."` and lost both. Running the ad-hoc build did not break the grant for the certificate-signed builds.
- Only stable `X.Y.Z` releases are offered. Pre-releases, drafts and suffixed tags are ignored (PR #28, core `stable_version`).
- The macOS app is named `LingoPop`. **PR #28 (`chore/macos-app-name`) is a hard prerequisite** and is merged first: on `main` the bundle, `package-macos.sh` and `verify-release-assets.py` still say `LingoPop Preview`, and `stable_version` exists only on that branch.
- Order of work: spec, then key generation, then implementation in its own PR, then a release and a two-version end-to-end check.
- **The first updater-capable version is 0.2.1** (owner's choice, no 0.3.0).
- **The two-version check runs on real public releases after the merge** (owner's choice): no test repository, no update-URL override in any build.

## Goals

1. **About-tab update flow** on both platforms: check, Update with progress, Restart.
2. **macOS permission continuity**: every release from the first updater-capable version on is signed with the same certificate.
3. **Authenticated updates**: the app installs only files signed by the project's update key. HTTPS or a checksum from the same release alone is not enough.
4. **Safe install**: a failed download, verification or install leaves the current app working and untouched.
5. **Release pipeline**: the Release workflow produces the signed update files and refuses to publish a macOS build that is not signed with the pinned certificate.

## Non-goals

- Silent or automatic download and install. The user always presses Update. The background check only shows the banner, as today.
- Delta or partial updates.
- Downgrades or choosing a version.
- Pre-release or beta channels.
- Notarization, Developer ID and a Windows Authenticode certificate. A later switch to Developer ID is possible, but it changes the designated requirement, so every user grants the permissions once more.
- An updater for the Swift app or for LingoPop 0.2.x. These versions have no updater, so users install the first updater-capable version by hand once.
- A Windows installer (NSIS / MSI). The app stays a portable exe.
- Update UI in the tray menu.

## Anti-goals

These count as failures even if the update technically works:

- The app that comes back after Restart has lost Accessibility or Screen Recording on macOS.
- An update leaves no working app: a half-replaced bundle or exe, or nothing at the original path.
- Code that is not signed by the project's update key gets executed.
- A newer signed asset can be swapped for an older one (replay), which would amount to a downgrade.
- The app looks updated, but the old version keeps running.
- The update replaces or touches the user's data in `%APPDATA%\LingoPop` or `~/Library/Application Support/LingoPop-Tauri`, or the keychain item.

## Constraints and facts

- **Platform.** Tauri 2.12.5, Rust workspace. The core stays OS-agnostic and `forbid(unsafe_code)`. OS code lives in `lingopop-platform`. No network in tests. See AGENTS.md.
- **Update check today.** Core `updates.rs` asks `GET https://api.github.com/repos/slucheninov/lingopop-app/releases/latest` and reads `tag_name`. The shell `updates.rs` (`UpdateState`) caches the result in `ui.json`, and About shows `Version %@ available` with a **Download** link.
- **Release JSON.** The same release JSON lists `assets[]` with `name`, `size` and `browser_download_url`. No separate manifest file is needed.
- **Asset download redirects.** A download from `github.com/<owner>/<repo>/releases/download/...` redirects to a GitHub object-storage host on another domain. The shell's `ReqwestTransport` follows only same-origin redirects, on purpose, for the providers. **The update download needs its own client with an explicit host allowlist.**
- **Relaunch.** `app_relaunch` spawns the current exe with `--wait-for-pid <pid>` and exits. The new process waits up to 10 s for the old one.
- **macOS install location.** The app lives in `/Applications/LingoPop.app`, which is writable by admin users only.
  - The platform already has `is_installed_bundle_path`, used by the login item: it rejects App Translocation, `/Volumes/...` and non-`.app` paths.
- **No Mark-of-the-Web / quarantine.** A file LingoPop downloads itself has no Mark-of-the-Web on Windows and no `com.apple.quarantine` on macOS, because the app is not quarantine-aware. So SmartScreen and Gatekeeper do not stop an updated app from launching.
- **Release assets today.**
  - `LingoPop-<v>-windows-x64-portable.exe`;
  - `LingoPop-<v>-macos-{arm64,x64}.{dmg,zip}` (the zip holds only `LingoPop.app/`, made with `ditto -c -k --keepParent`);
  - `SHA256SUMS.txt`;
  - each leg's `signing-<slug>.txt` (`adhoc` / `developer-id`).
  - `verify-release-assets.py` enforces an exact file set.
- **Swift app.** The Swift app is also named `LingoPop.app`, but it has a different bundle id. The updater must only ever replace a bundle whose id is `app.lingopop.tauri.macos`.

## Design

### 1. Update key and signed assets

- **The key.** A **minisign** (Ed25519) key pair is the update key.
  - The public key is a constant in core, `updates::UPDATE_PUBLIC_KEY`, committed to the repo.
  - The secret key and its password are GitHub Actions secrets: `UPDATE_SIGNING_KEY`, `UPDATE_SIGNING_KEY_PASSWORD`.
  - The owner keeps an offline backup. Losing the secret key means installed apps can never accept an update again, and users must reinstall by hand. Losing the public key is impossible, it is in the code.
- **What gets signed.** Every updatable asset gets a detached signature `<asset>.minisig` in the release:
  - `LingoPop-<v>-windows-x64-portable.exe.minisig`;
  - `LingoPop-<v>-macos-arm64.zip.minisig`;
  - `LingoPop-<v>-macos-x64.zip.minisig`.

  macOS updates use the **zip**, not the dmg. dmgs are not signed for updating.
- **Trusted comment against replay.** The minisign trusted comment of each signature is exactly `lingopop-update <asset file name>`, for example `lingopop-update LingoPop-0.2.2-macos-arm64.zip`. The asset file name contains the version.
  - The app rejects a signature whose trusted comment differs from the name it expects for the target version. An old, validly signed asset therefore cannot be served as the new version.
- **Verification in the app.** The `minisign-verify` crate (pure Rust, no `unsafe` in our code) does it. The signed bytes are the whole downloaded file.

### 2. Core: update plan (pure, OS-agnostic)

Core gets pure functions with tests on fixtures:

- **Parse the release JSON** into `{version, assets: [{name, size, url}]}`, with the stable-only rules from PR #28. Prerelease or draft = no update.
- **`update_plan(release, current_version, target: UpdateTarget)`.**
  - `UpdateTarget` = `WindowsX64 | MacosArm64 | MacosX64`, injected by the shell; the core never inspects the OS.
  - The result is `None` when the version is not newer, or when the asset or its `.minisig` is missing.
  - Otherwise it is `{version, asset_name, asset_url, asset_size, sig_url, expected_comment}`.
  - Asset names come from the same naming rule as the release contract: `LingoPop-<v>-<slug>...`.
- **Download URL check.** HTTPS only; host `github.com` with the path prefix `/slucheninov/lingopop-app/releases/download/v<version>/`. Anything else = `None`.
- **Size limit.** Above 300 MB = `None`.
- **`verify_update(bytes, sig_text, expected_comment) -> Result<(), UpdateError>`.** Wraps `minisign-verify` with the committed public key and the trusted-comment rule.
- **`UpdateError`.** A fixed set of variants, with catalog keys for the UI and no file paths or URLs in user text.

### 3. Shell: download

- **A dedicated update download client.** reqwest + rustls, the same TLS stack as the providers.
  - Redirects are followed only to an allowlist: `github.com`, `objects.githubusercontent.com` and `release-assets.githubusercontent.com`, HTTPS only, at most 5 hops.
  - **(Builder: confirm the current object-storage host names before pinning them.)**
- **Streaming.** The download streams to a staging file and reports progress (bytes / total) to the About tab.
- **Size checks.** The download is aborted when the body exceeds the planned size.
- **Cancel.** Cancel is possible while downloading.
- **Signature first.** The signature file is downloaded first. The asset bytes are verified with `verify_update` before any install step. A failure deletes the staging files.
- **Temporary files.** The staging location is described per OS below. Leftovers of a previous attempt are removed at startup, but only through the update journal.
- **Update journal.** Before the first file of an attempt is created (the signature and asset downloads included), the updater writes `update-journal.json` atomically in the data folder (`fsio::atomic_write`).
  - It names every path this attempt creates or moves: `{"version", "site", "staged", "old", "scratch": [..]}`. `site` is the install path recorded at startup; `scratch` lists the download files and, on macOS, the staging folder `<parent>/.LingoPop-update-<version>/`. Recovery deletes `scratch` entries like `staged` (never `running` or `site`).
  - Startup acts only on this file and never matches names alone, so a user file that happens to be called `<stem>.old.exe` is never touched. Without a journal, startup deletes nothing.
  - **Phase.** The journal also has a `phase`.
    - `pending` when it is first written.
    - It becomes `committed` (rewritten atomically) only after the swap AND every check after it succeeded.
    - Only a `committed` journal means the version at `site` is the one to keep.
  - **Identity, not position.** After the new item passed every check before the swap, its SHA-256 goes into the journal as `newHash`, in an atomic rewrite done before the swap.
    - The new item is the exe on Windows, or the bundle's main executable `Contents/MacOS/<CFBundleExecutable>` on macOS.
    - Recovery never infers state from which path a bundle sits at. It hashes what is at `site`: equal to `newHash` means "the new version is installed", anything else means "the previous version is installed".
    - A journal without `newHash` means no swap can have happened.
  - **Recovery** (any copy of the app, at startup, and before every new update attempt). Every step is idempotent, so an interrupted recovery is just run again:
    - `site` missing (Windows only, between the two renames): rename `old` to `site` when `old` exists, then continue below. When `old` is missing too, nothing is touched, the journal stays and `update recovery failed` is logged.
    - The new version is at `site` and the phase is `committed`: delete `staged`, `old` and `scratch`, then the journal.
    - The new version is at `site` and the phase is `pending` (the swap happened, a later check or its swap-back failed): put the previous version back.
      - This happens only when the item at the other path exists and is NOT the new one: on macOS `RENAME_SWAP` of `site` and `staged`; on Windows `site` to `staged`, then `old` to `site`.
      - Then hash `site` again and continue with the next rule.
    - The previous version is at `site` (no swap yet, or swapped back): the remaining copies are the new or unchecked download. Delete `staged`, `old` and `scratch`, then the journal. An unchecked staged bundle is therefore never installed by recovery.
    - **Running copy.** A path is never deleted when it equals the running exe or bundle or contains it (path-prefix test, so a running bundle inside a scratch folder is safe). Such a delete is skipped and the journal stays.
    - Every error is logged only; the journal then stays so the next start tries again.
  - A completed update leaves the journal for that cleanup. A failed or cancelled attempt whose files never moved, or whose rollback succeeded, deletes its own files and the journal at once. When a rollback FAILS, nothing is deleted: the journal and the file or bundle it names as `old` stay, so the next start restores them.
  - **No new attempt over an open journal.** `updates_install` first runs recovery. If a journal is still present afterwards, it is Failed with the install error and writes nothing: no new journal, no download. The open journal is never overwritten or removed by a retry or a cancel.

### 4. Windows install (portable exe)

- **Names.** They are derived from the running exe, whatever the user called it (the shipped file is `LingoPop-<v>-windows-x64-portable.exe`). With `<stem>` = its file name without `.exe`:
  - staging is `<exe dir>\<stem>.update.exe`;
  - the old file is `<exe dir>\<stem>.old.exe`.

  Both sit on the same volume, so the renames are atomic. The file keeps its name after the update, even when that name carries the old version number: renaming it would break shortcuts and the Run value.
  - **A non-writable exe folder** (for example `Program Files`) is an error, "LingoPop can't update itself in this folder", with the **Download** link as the fallback.
- **Checks after the download.** Signature OK and the PE machine is AMD64 (the same check as the release contract).
- **Swap.**
  1. Rename the running `<stem>.exe` to `<stem>.old.exe`. Windows allows renaming a running exe.
  2. Rename `<stem>.update.exe` to `<stem>.exe`.
  3. If step 2 fails, rename `<stem>.old.exe` back.

  The journal (section 3) is written before step 1. If the process dies between steps 1 and 2, or the rollback in step 3 fails, the journal remains. The user's shortcut is then broken, but `<stem>.old.exe` is still a visible, runnable file in the same folder. Starting it, or any copy of the app, recovers through the journal. Windows cannot replace a running exe in one step, so this short gap cannot be closed.
- **Cleanup.** At startup the paths named in the journal are deleted (section 3); nothing is deleted by name. Errors are logged only. The process still running from the old file never deletes it.
- **What the user sees.** The About tab enters the **ready** state: **Restart** relaunches the exe at the original path through the existing `--wait-for-pid` path.
- **Not restarting.** If the user quits without pressing Restart, the next start is the new version, which is fine.
- **Launch at login.** The Run value already points to the original path, so nothing changes.

### 5. macOS install (app bundle)

- **Preconditions.** In-app update is offered only when all of these hold; otherwise About shows only the **Download** link, as today:
  - the running bundle passes `is_installed_bundle_path`;
  - its bundle id is `app.lingopop.tauri.macos`;
  - its parent folder is writable;
  - the running app's own designated requirement is exactly `identifier "app.lingopop.tauri.macos" and certificate leaf = H"<RELEASE_CERT_SHA1>"`.
    - `RELEASE_CERT_SHA1` is a constant in core next to `UPDATE_PUBLIC_KEY`, and CI checks that it equals `release-signing.json`.
    - An ad-hoc or otherwise-signed local build never self-updates.
- **Staging.** `<parent>/.LingoPop-update-<version>/`, on the same volume as the app. The zip is extracted there with `ditto -x -k --noqtn`.
- **Checks before the swap.** The extracted bundle must:
  - contain exactly one `LingoPop.app`;
  - have `CFBundleIdentifier` = `app.lingopop.tauri.macos`;
  - have `CFBundleShortVersionString` = the planned version;
  - pass `codesign --verify --strict --deep`;
  - **satisfy that same pinned designated requirement** (`codesign --verify -R="<pinned DR>"`). This check guarantees TCC continuity: an update the system would treat as a different app is refused.
  - have an executable that contains the running architecture.
- **Install path.** `<app>` below is the validated running bundle path recorded at startup (the journal's `site`), wherever it is. It is never a hard-coded `/Applications/LingoPop.app`. The swap, the rollback and the restart all use this same path.
- **Swap.**
  1. Write the journal: `site` = `<app>`, `staged` = the extracted `<staging>/LingoPop.app`, `old` = that same staged path after the swap.
  2. Exchange `<app>` and the extracted bundle in one atomic call: `renamex_np(staged, app, RENAME_SWAP)` (macOS 10.12+, same volume). There is no moment without an app at `<app>`. Afterwards the old bundle sits in the staging folder.
  3. If the swap fails, nothing has moved. If a later check fails, the same swap is made again to roll back.
  4. The old bundle stays in the staging folder until the next start, because the process still running from it may load resources from it. Startup cleanup then deletes the staging folder named by the journal; errors are logged only.

  A running macOS app can be moved; the running process keeps its open files. The `renamex_np` call is the only new `unsafe` and lives under `crates/lingopop-platform/src/macos/`.
- **Restart.** **Restart** relaunches through `/usr/bin/open -n <app> --args --wait-for-pid <pid>` and exits.
  - Going through LaunchServices makes the new bundle the responsible process.
  - **(Builder: confirm the existing `--wait-for-pid` + single-instance path still works when started through `open`.)**
- **Keychain.** The keychain item `app.lingopop.tauri.macos` / `secrets-key-v1` is untouched.
  - Its access list was set for the previous signature. With a stable certificate the requirement is the same, so normally there is no new prompt. If macOS asks once, "Always Allow" covers it.

### 6. Shell commands and state

- **Update state machine.** One managed state machine: `Idle -> Checking -> Available(plan) -> Downloading(progress) -> Verifying -> Installing -> Ready(version) | Failed(error)`. Only one update runs at a time.
- **Commands** (async, catalog-key errors):
  - `updates_check`, which exists today; it returns the plan, or the version when only the Download fallback applies;
  - `updates_install`, which starts the download and install;
  - `updates_cancel`;
  - `updates_restart`;
  - `updates_state`, a read of the current state that changes nothing.
- **Event.** `updates://state` carries the state; progress events are throttled to about 10 per second.
  - Every state value carries a `revision` that grows with each change.
  - The About tab first registers the listener, then calls `updates_state`. It applies a value, from the event or the read, only when its revision is higher than the last one applied. So a change during registration or during the read is never missed, and a stale read never overwrites a newer event.
- **Ready survives closing Settings.** `Ready` is kept in memory while the app runs. Reopening Settings shows **Restart** through `updates_state`.
- **After a swap, every relaunch uses the original install path.**
  - The old process's `current_exe()` may point at the moved bundle (`.LingoPop-update-*/old.app`) or at `<stem>.old.exe`.
  - So both the language-change Relaunch (`app_relaunch`) and `updates_restart` start the installed path recorded before the swap; on macOS they go through `open -n <app>`.
  - The login item is unchanged: SMAppService names the bundle by id, and the Windows Run value already holds the original path.
- **Background check.** It is unchanged. It still only sets the banner.

### 7. About tab UI

| State | What the user sees |
|---|---|
| idle | Check for updates (as today) |
| available | `Version %@ available`, **Update**, and a **What's new** link to the release page |
| downloading | progress bar with percent, **Cancel** |
| verifying / installing | spinner, "Installing…" |
| ready | "LingoPop %@ is installed. Restart to finish.", **Restart** |
| failed | the error sentence, **Try again**, and the **Download** link as a manual fallback |
| unsupported | today's banner with **Download** only (dev builds, non-writable folder, disk image, ad-hoc running app) |

- **Translations.** All new strings go into the shared catalogs with pl / uk / de, plus platform files where the text differs.
- **Window size.** The tab keeps the fixed Settings window size.

### 8. Release pipeline

- **New signing mode `selfsigned`** in `package-macos.sh`:
  - inputs: secrets `MACOS_SIGNING_P12` (base64 PKCS#12) and `MACOS_SIGNING_P12_PASSWORD`;
  - signing: a temporary keychain, as the Developer ID path already does, then `codesign --force --sign <sha1> --options runtime --timestamp=none`.
- **Pinned certificate.**
  - The certificate's SHA-1 is pinned in a committed file, `release-signing.json`: `{"macosCertificateSha1": "<40 hex>"}`.
  - The leg fails when the imported identity differs.
  - The native checks assert that the designated requirement is exactly `identifier "app.lingopop.tauri.macos" and certificate leaf = H"<pinned sha1>"`.
  - `signing-<slug>.txt` = `selfsigned`.
- **Mode rules.**
  - Both self-signed secrets present: `selfsigned`.
  - Otherwise: `adhoc`.
  - **A tag run whose release includes the Mac assets (`macosPreview: true`) must be `selfsigned` with the pinned certificate, or it fails.** An ad-hoc or Developer ID release changes the designated requirement, and every installed copy would refuse it.
  - `adhoc` stays allowed on `workflow_dispatch` and when the Mac assets are left off the release.
  - The Developer ID code path stays in the script, but tag runs never select it. A later switch to Developer ID is a deliberate change of this spec and costs every user one more permission grant.
- **Update signatures.** **Only the publish job holds the update key**: one Ubuntu job, and no third-party npm or cargo build runs there before signing. The order is fixed, and each step stops the release on any error:
  1. `assemble` the build-leg artifacts.
  2. **Pre-signing check.** `verify_assets` on the unsigned set, the build assets plus their manifest, without `.minisig` files.
  3. **Sign** the three updatable assets with the trusted comment from section 1.
  4. **Post-signing check.**
     - The cryptographic verification of each `.minisig` against the committed public key.
     - Then the manifest is regenerated to include the `.minisig` files.
     - Then `verify_assets` runs on the complete set: names, sizes, checksums and trusted-comment lines.
  5. Only then is the draft created.

  The details of the checks:
  - `verify-release-assets.py` learns the three `.minisig` names in the exact file set.
  - Their trusted-comment line must name the matching asset, a text check in Python.
  - The **cryptographic** verification against the committed public key runs before the draft is created, in the publish job, with a small verifier built from the same `minisign-verify` code. It is a workspace binary built with `--locked` in a separate job without secrets; the publish job checks the transferred file's SHA-256 against that job's output (a transfer check, not a trust check) and builds nothing itself. Python's stdlib has no Ed25519. The signing tool is the builder's call: a pinned binary with a checked SHA-256, or the same workspace binary.
  - `SHA256SUMS.txt` lists the `.minisig` files too.
- **Gatekeeper.** The certificate is untrusted, so the first manual install still needs Open Anyway. The `macos-adhoc` blocks in `docs/INSTALL.md` become "not notarized" blocks, used for both `adhoc` and `selfsigned`.
- **Docs.**
  - `docs/RELEASING.md` describes key and certificate custody, backup, and the "never rotate" rule. Rotating either one is a breaking event and has to be planned.
  - AGENTS.md gets the new modules.

### 9. First updater-capable release (transition)

- **Version.** The first updater-capable version is **0.2.1**, signed `selfsigned`. Under semver a patch is not strictly right for a feature; the owner chose it on purpose.
- **Users of 0.2.0 and Preview.** They install 0.2.1 by hand once and grant the permissions once more; the 0.2.1 release notes and README say so. From 0.2.1 on, updates go through About.
- **End-to-end check, after the merge, on public releases:**
  1. Publish 0.2.1. The owner installs it by hand into `/Applications` on macOS and into a writable folder on Windows, then grants Accessibility and Screen Recording.
  2. Publish **0.2.2**: the next real release, with at least a CHANGELOG entry.
  3. In 0.2.1, open About and run Check, Update, Restart. Confirm that the version is 0.2.2 and that a text operation, Replace, Esc and Screen Translate work without System Settings. On Windows, confirm the exe at the same path is 0.2.2 and the leftovers are gone after the next start.
  4. **If the in-app update in 0.2.1 turns out broken**, users of 0.2.1 install the fix by hand once, and the release notes say so. This is the accepted risk of testing on public releases.
- **No update-URL override.** No build has one, debug included. The update source is the constant release API URL.

### 10. Tests

- **Core.** Release JSON parsing, `update_plan` for each target and version case, URL and host rules, the size limit, `verify_update` with fixture keys.
  - The fixtures are a test-only minisign key generated once and committed under `tests/fixtures/updates/`; it is never the real key.
  - Cases: good, tampered byte, wrong key, wrong trusted comment, old-version comment.
- **Shell.** The state machine over `FakeTransport` and a fake installer: cancel, failure, and ready after install. The redirect allowlist is tested with a loopback server.
- **Windows.**
  - The swap and the rollback are tested in a temp dir, by default on the Windows CI leg.
  - Startup cleanup.
  - An end-to-end relaunch of a copied exe in an ignored desktop test.
- **macOS.**
  - The swap and rollback, and the bundle checks, use fake bundles in a temp dir, signed ad hoc in tests. The designated-requirement mismatch is a required test.
  - A real certificate-signed update is owner-run only.
- **Release scripts.** `test_release_assets.py` gets `.minisig` set and comment cases, and a self-signed mode for the package script's mode selection.

## Left to the builder

- The crate versions (`minisign-verify`, reqwest streaming) and module layout inside core / platform / shell, following the existing rules.
- The exact progress UI styling and whether an updated About keeps today's Download link visible next to Update.
- Throttling, timeouts and retry counts for the download. My suggestion: no automatic retry; the user presses Try again.

## My calls (owner to confirm while reading)

- The update key is minisign / Ed25519 in GitHub secrets, with an offline backup by the owner.
- macOS updates use the zip, not the dmg.
- The update key is held only by the publish job.
- The release certificate SHA-1 is pinned in the app (a core constant), not only in CI.
- Tag releases with Mac assets must be `selfsigned`; Developer ID is not used for tags until a later spec changes that.
- The old bundle or exe is deleted at the next start, not right after the swap. Every relaunch after an update uses the original install path.
- On Windows the exe keeps its file name across updates.
- A custom updater instead of `tauri-plugin-updater`. That plugin cannot update a portable exe, and it cannot check that the new bundle satisfies the running app's designated requirement, which is the property that keeps the permissions.
- No automatic download; the user presses Update.
- A non-writable install folder, a disk image or an ad-hoc running app gets the Download link instead of Update.
