# In-app updates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Settings > About runs Check, then Update (download, verify, install), then Restart on Windows and macOS. Releases from 0.2.1 on are signed so that macOS permissions survive updates.

**Architecture:**
- Pure update rules (release parsing, plan, signature check) live in `lingopop-core`.
- The streaming download is a core HTTP helper with a redirect allowlist.
- File-system install (exe swap, bundle checks and swap, startup cleanup) lives in a portable `lingopop-platform` module, with macOS `codesign` calls behind a seam.
- The Tauri shell owns one update state machine, the commands and the relaunch path.
- The Release workflow signs macOS builds with one pinned self-signed certificate and signs the updatable assets with a minisign key, held only in the publish job.

**Tech Stack:** Rust (core / platform / app), `minisign-verify` (app side) and `minisign` (signing tool), reqwest 0.12 streaming (`Response::chunk`), Svelte 5, bash `package-macos.sh`, Python `verify-release-assets.py`, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-06-in-app-updates-design.md`

## Global Constraints

- **Release and version.** The first updater-capable release is `0.2.1`, then `0.2.2` for the end-to-end check. Only stable `X.Y.Z` counts (`updates::stable_version`).
- **Key and trusted comment.** The update key is minisign / Ed25519, and its public key is the core constant `updates::UPDATE_PUBLIC_KEY`. The trusted comment of each signature is exactly `lingopop-update <asset file name>`.
- **Updatable assets:**
  - `LingoPop-<v>-windows-x64-portable.exe`;
  - `LingoPop-<v>-macos-arm64.zip`;
  - `LingoPop-<v>-macos-x64.zip`;
  - plus each one's `<asset>.minisig`.

  dmgs are not updatable.
- **Download URL rules:**
  - HTTPS only;
  - the first URL is host `github.com`, path prefix `/slucheninov/lingopop-app/releases/download/v<version>/`;
  - redirects are followed only to `github.com`, `objects.githubusercontent.com` and `release-assets.githubusercontent.com` (confirm the object host names in Task 4), at most 5 hops;
  - size limit 300 MB (`MAX_UPDATE_BYTES = 300 * 1024 * 1024`).
- **macOS designated requirement (exact):** `identifier "app.lingopop.tauri.macos" and certificate leaf = H"<RELEASE_CERT_SHA1>"`.
  - `RELEASE_CERT_SHA1` is a core constant: 40 lowercase hex.
  - It must equal `release-signing.json` `macosCertificateSha1`.
- **macOS install.**
  - Only bundle id `app.lingopop.tauri.macos` is ever replaced.
  - Staging is `<parent>/.LingoPop-update-<version>/`, extracted with `ditto -x -k --noqtn`.
  - The old bundle is kept until the next start.
  - Restart is `/usr/bin/open -n <app> --args --wait-for-pid <pid>`.
- **Windows install.**
  - `<stem>` is the running exe's file name without `.exe`.
  - Staging is `<stem>.update.exe`, the old file is `<stem>.old.exe`, both next to the exe.
  - The exe keeps its file name.
- **Data and settings.** No user data (`%APPDATA%\LingoPop`, `~/Library/Application Support/LingoPop-Tauri`, keychain item) is touched. There is no update-URL override in any build, and no automatic download.
- **Tag runs.** A tag run with Mac assets (`macosPreview: true`) must sign `selfsigned` with the pinned certificate. `adhoc` is allowed only on `workflow_dispatch` or without Mac assets.
- **Repository rules** (AGENTS.md):
  - core stays OS-agnostic and `forbid(unsafe_code)`;
  - no network in tests (loopback only);
  - every new UI literal goes into the catalogs (pl / uk / de) in the same commit;
  - every new module gets its AGENTS.md Layout line in the same commit;
  - dependency versions are pinned exactly.

## Review Focus

1. **Update pressed twice, or Check pressed during a download.** Exactly one update runs; a second `updates_install` returns the current state and does not start a second download (Task 7: `install_is_single_flight`).
2. **Download cut off midway, disk full, or a body shorter or longer than `size`.** The state becomes Failed, the staging files are gone, and the installed app is byte-identical (Task 4: `truncated_or_oversized_body_fails`; Task 5: `failed_stage_leaves_exe_untouched`).
3. **GitHub answers 403 or 404 or HTML** (rate limit, a release without `.minisig`, for example Mac assets left off). Check shows the Download-only state for this platform, never an Update button that cannot work (Task 3: `release_without_signature_has_no_plan`).
4. **The language-change Relaunch is pressed after an update installed but before Restart.** It must start the installed new version, not the old bundle left in the staging folder or `<stem>.old.exe` (Task 7: `relaunch_after_swap_uses_install_path`).
5. **The app runs from a folder it cannot write to, from a dmg, or as an ad-hoc dev build.** It shows Download only, and nothing is written next to the app (Task 5: `read_only_folder_is_unsupported`; Task 6: `adhoc_running_app_is_unsupported`).

---

### Task 1: Production signing material (owner-authorized)

Nothing in this task is committed except public values.

**Files:**
- Create: `release-signing.json`
- Modify: `crates/lingopop-core/src/updates.rs` (constants only)

**Interfaces:**
- Produces:
  - `pub const UPDATE_PUBLIC_KEY: &str` (minisign public key, base64 line `RW...`);
  - `pub const RELEASE_CERT_SHA1: &str` (40 lowercase hex);
  - `release-signing.json` = `{"macosCertificateSha1": "<same 40 hex>"}`;
  - GitHub secrets `MACOS_SIGNING_P12` (base64 PKCS#12), `MACOS_SIGNING_P12_PASSWORD`, `UPDATE_SIGNING_KEY` (minisign secret key file contents), `UPDATE_SIGNING_KEY_PASSWORD`.

- [ ] **Step 1: Generate the certificate** in a new folder outside the repo, `~/LingoPop-release-keys/` (mode 700).
  - Use OpenSSL: RSA 3072, 20 years, CN `LingoPop Release Signing`, extensions `basicConstraints=critical,CA:false`, `keyUsage=critical,digitalSignature`, `extendedKeyUsage=critical,codeSigning`.
  - Export PKCS#12 with a random 32-character password: `-legacy`, or PBE-SHA1-3DES, so `security import` reads it.
  - Record the SHA-1 with `openssl x509 -fingerprint -sha1 -noout`, lowercased, without colons.
- [ ] **Step 2: Generate the minisign key pair** into the same folder with a random 32-character password, using the `minisign` CLI or the Task 2 tool once it exists. Write `README.txt` there, listing the files, the passwords and the rule "never rotate".
- [ ] **Step 3: Set the four secrets** with `gh secret set <NAME> --repo slucheninov/lingopop-app` from the files and stdin, never echoed.
  - Run `gh secret list`; expected: all four names present.
- [ ] **Step 4: Check the certificate signs and pins.** Sign a throwaway bundle with it from a temporary keychain, then run `codesign -d -r-`.
  - Expected: `designated => identifier "<id>" and certificate leaf = H"<sha1>"` with the recorded SHA-1.
- [ ] **Step 5: Commit the public values.**
  - Write `release-signing.json` and the two constants with doc comments: "rotating either breaks updates or macOS permissions for every user".
  - Run `git add release-signing.json crates/lingopop-core/src/updates.rs && git commit -m "release: pin the update public key and macOS signing certificate"`.
- [ ] **Step 6: Tell the owner** the path `~/LingoPop-release-keys/`, to move it into a password manager.

### Task 2: Update signature verification (core) and signing tool

**Files:**
- Create: `crates/lingopop-core/src/update_sig.rs`
- Create: `crates/lingopop-release-tools/Cargo.toml`
- Create: `crates/lingopop-release-tools/src/main.rs` (bin `lingopop-release-tools`)
- Create: `crates/lingopop-core/tests/fixtures/updates/test.pub`
- Create: `crates/lingopop-core/tests/fixtures/updates/test.key`, a TEST key with the password `test`, never used for releases
- Create: signed fixture files under the same folder
- Modify: `crates/lingopop-core/src/lib.rs`
- Modify: `crates/lingopop-core/Cargo.toml` (`minisign-verify = "=<latest 0.2.x>"`)
- Modify: `Cargo.toml` (workspace members already glob `crates/*`)

**Interfaces:**
- Produces:
  - `pub fn update_comment(asset_name: &str) -> String` = `format!("lingopop-update {asset_name}")`.
  - `pub fn verify_update_with(public_key: &str, bytes: &[u8], signature: &str, expected_comment: &str) -> Result<(), UpdateError>`.
  - `pub fn verify_update(bytes, signature, expected_comment)`, which uses `UPDATE_PUBLIC_KEY`.
  - `pub enum UpdateError { Signature, Comment, Download, TooLarge, Network, Unsupported(&'static str), Install(&'static str), Cancelled }`.
    - `Display` is fixed English text only.
    - `catalog_key()` returns the UI keys from Task 8.
  - The CLI:
    - `lingopop-release-tools sign --key <file> --password-env <VAR> <asset>...` writes `<asset>.minisig`, with the trusted comment `update_comment(basename)`;
    - `lingopop-release-tools verify --public-key <file|const> <asset>...` calls core `verify_update_with` and exits 1 on any failure.
  - The tool depends on `minisign = "=<latest>"` and `lingopop-core`.

- [ ] **Step 1: Write the failing tests** in `update_sig.rs` `#[cfg(test)]` over the committed fixtures:
  - `good_signature_verifies` → `Ok(())`;
  - `tampered_byte_is_rejected` → `Err(Signature)`;
  - `other_key_is_rejected` → `Err(Signature)`;
  - `wrong_comment_is_rejected`: signed for `LingoPop-0.2.1-…zip`, expected `LingoPop-0.2.2-…zip` → `Err(Comment)`;
  - `garbage_signature_is_rejected` → `Err(Signature)`.
- [ ] **Step 2: Run** `cargo test -p lingopop-core update_sig`. Expected: FAIL, the module is missing.
- [ ] **Step 3: Implement** `update_sig.rs` with `minisign_verify::{PublicKey, Signature}`.
  - Check the trusted comment after a valid signature: `sig.trusted_comment() == expected_comment`.
  - Implement the release-tools CLI.
  - Generate the fixtures with it, documenting the command in a fixture `README.md`.
- [ ] **Step 4: Run** `cargo test -p lingopop-core update_sig` and then `cargo run -p lingopop-release-tools -- verify --public-key crates/lingopop-core/tests/fixtures/updates/test.pub <fixture>`. Expected: PASS, and exit 0.
- [ ] **Step 5: Commit** with the AGENTS.md Layout lines for `update_sig.rs` and `crates/lingopop-release-tools`. Run `scripts/check-core-deps.sh`: still OK.

### Task 3: Release parsing and update plan (core)

**Files:**
- Modify: `crates/lingopop-core/src/updates.rs`

**Interfaces:**
- Consumes: `stable_version`, `is_newer` (PR #28) and `update_comment` (Task 2).
- Produces:
  - `pub enum UpdateTarget { WindowsX64, MacosArm64, MacosX64 }` with `asset_name(&self, version: &str) -> String`, using the names from Global Constraints.
  - `pub struct Release { pub version: String, pub assets: Vec<ReleaseAsset> }` and `pub struct ReleaseAsset { pub name: String, pub size: u64, pub url: String }`.
  - `pub fn parse_release(json: &[u8]) -> Option<Release>`. It returns `None` for prerelease / draft / non-stable tags.
  - `pub struct UpdatePlan { pub version: String, pub asset_name: String, pub asset_url: String, pub asset_size: u64, pub signature_url: String, pub expected_comment: String }`.
  - `pub fn update_plan(release: &Release, current: &str, target: UpdateTarget) -> Option<UpdatePlan>`.
  - `pub async fn fetch_release_with(transport, deadline) -> Option<Release>`. `fetch_latest_version_with` becomes `fetch_release_with(..).map(|r| r.version)`.
  - `pub const MAX_UPDATE_BYTES: u64`.

- [ ] **Step 1: Write the failing tests** with JSON fixtures inline:
  - `plan_for_each_target`: the asset and `.minisig` are found, and `expected_comment == "lingopop-update LingoPop-0.2.2-macos-arm64.zip"`;
  - `release_without_signature_has_no_plan`;
  - `not_newer_has_no_plan`;
  - `foreign_url_has_no_plan`: another repo, `http:`, another host, or a different tag path;
  - `oversized_asset_has_no_plan`: `size` above 300 MB;
  - `prerelease_or_draft_release_is_none`.
- [ ] **Step 2: Run** `cargo test -p lingopop-core updates`. Expected: FAIL.
- [ ] **Step 3: Implement.** The URL check uses `url::Url`. It requires scheme `https`, host `github.com`, and a path equal to `/slucheninov/lingopop-app/releases/download/v<version>/<asset name>`.
- [ ] **Step 4: Run** `cargo test -p lingopop-core updates`. Expected: PASS, including the existing update tests.
- [ ] **Step 5: Commit.**

### Task 4: Streaming update download (core HTTP)

**Files:**
- Create: `crates/lingopop-core/src/http/download.rs`
- Test: `crates/lingopop-core/tests/update_download.rs` (wiremock on loopback)

**Interfaces:**
- Consumes: `UpdatePlan`, `MAX_UPDATE_BYTES` and `UpdateError` (Tasks 2–3).
- Produces:
  - `pub struct UpdateDownloader` with:
    - `new() -> Self`: the production allowlist and rustls;
    - `#[doc(hidden)] pub fn for_tests(allowed: &[&str]) -> Self`: allows `http://127.0.0.1`.
  - `pub async fn download(&self, url: &str, max_bytes: u64, dest: &Path, progress: &(dyn Fn(u64, Option<u64>) + Send + Sync)) -> Result<u64, UpdateError>`.
    - It streams with `Response::chunk()` into `dest`, then calls `sync_all`.
    - It returns the bytes written. The caller (Task 7) fails with `Download` when the count differs from `UpdatePlan.asset_size`, shorter or longer.
    - It removes `dest` on any error.
    - A redirect outside the allowlist, or more than 5 hops, is `Network`.
    - A body over `max_bytes` is `TooLarge`.
  - Cancellation is dropping the future; `dest` is removed by a drop guard.
  - The User-Agent is `LingoPop/<VERSION>`.

- [ ] **Step 1: Write the failing tests:**
  - `downloads_and_reports_progress`;
  - `redirect_to_allowed_host_is_followed`;
  - `redirect_to_foreign_host_is_rejected`;
  - `truncated_or_oversized_body_fails` (dest absent afterwards);
  - `short_content_length_returns_its_own_count` (`Content-Length` 10 with a plan size of 20 → `Ok(10)`; Task 7 `size_mismatch_is_failed` rejects it);
  - `http_error_fails`;
  - `cancel_removes_partial_file` (drop the future mid-body).
- [ ] **Step 2: Run** `cargo test -p lingopop-core --test update_download`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Use `reqwest::redirect::Policy::custom` with the allowlist and HTTPS-only check (loopback `http` only in `for_tests`).
  - Confirm the GitHub object-storage host names: run one manual `curl -sI` of a current 0.2.0 asset URL, outside tests, and record the result in the module doc.
- [ ] **Step 4: Run** the tests. Expected: PASS.
- [ ] **Step 5: Commit** with the AGENTS.md line.

### Task 5: Install site and Windows exe swap (platform, portable)

**Files:**
- Create: `crates/lingopop-platform/src/update_install/mod.rs`
- Create: `crates/lingopop-platform/src/update_install/exe.rs`
- Test: `crates/lingopop-platform/tests/update_install_exe.rs` (temp dirs, every OS)

**Interfaces:**
- Produces:
  - `pub struct InstallSite { pub path: PathBuf, pub kind: SiteKind }`, with `pub enum SiteKind { WindowsExe, MacBundle }`.
    - `path` is the exe or `.app` path recorded at startup.
  - `pub enum Support { Supported(InstallSite), Unsupported(&'static str) }`. The reason is a fixed log text.
  - `pub fn exe_support(exe: &Path) -> Support`:
    - `Unsupported("folder not writable")` when a probe file `<stem>.update.probe` cannot be created and removed.
  - `pub fn stage_exe(site: &InstallSite) -> PathBuf`: `<stem>.update.exe`.
  - `pub fn install_exe(site: &InstallSite, staged: &Path) -> Result<(), UpdateError>`:
    - it checks the PE machine is AMD64 (the same rule as `scripts/verify-release-assets.py` `pe_machine`), then swaps with rollback;
    - after an `Err`, the original exe is byte-identical at its path.
  - `pub struct Journal { pub version: String, pub site: PathBuf, pub staged: PathBuf, pub old: PathBuf, pub scratch: Vec<PathBuf>, pub phase: Phase, pub new_hash: Option<String> }` with `pub enum Phase { Pending, Committed }` (`pub fn commit_journal(data_dir) -> Result<(), UpdateError>` rewrites it as `Committed`` (serde camelCase, file `update-journal.json` in the data dir).
    - `pub fn write_journal(data_dir: &Path, j: &Journal) -> Result<(), UpdateError>` uses core `fsio::atomic_write`.
    - `pub fn remove_journal(data_dir: &Path)`.
  - `pub fn recover(data_dir: &Path, running: &Path) -> Recovery` (`enum Recovery { Nothing, Cleaned, Restored, Kept }`) implements spec section 3 exactly:
    - with no journal it touches nothing;
    - `site` missing and `old` present: restore it;
    - the decision follows spec section 3: `site` is identified by `item_hash(site) == new_hash` (SHA-256 of the exe; on macOS `<CFBundleShortVersionString>:<SHA-256 of Contents/_CodeSignature/CodeResources>`, through an injected identity function), never by position;
    - new at `site` + `Pending`: put the previous version back (Windows: `site`→`staged`, `old`→`site`; macOS: `BundleTools::swap(old, site)` through an injected swap closure), then clean;
    - `site` present otherwise: delete `staged`, `old` and every `scratch` entry (files, or directories with `remove_dir_all`);
    - it never deletes a path that equals or contains `running` (`running.starts_with(path)`); such a delete is skipped and the journal kept;
    - errors keep the journal.
  - `install_exe` takes `data_dir` and expects the journal Task 7 wrote before the download. On a failure before the first rename, or after a successful rollback, it deletes its files and the journal. When the rollback fails it deletes NOTHING and returns `Err(Install)`: the journal and `<stem>.old.exe` stay for `recover`.

- [ ] **Step 1: Write the failing tests:**
  - `swap_replaces_exe_and_keeps_name`;
  - `no_journal_deletes_nothing` (user files `<stem>.old.exe` and `<stem>.update.exe` survive);
  - `crash_between_renames_is_restored` (site missing, old present, journal → `Restored`, site has the old bytes);
  - `completed_swap_is_cleaned` (`Committed` journal + old → `Cleaned`, old and journal gone);
  - `uncommitted_swap_is_restored` (`Pending` journal, new file at site, old present → `Restored`, site has the old bytes);
  - `running_old_file_is_kept` (`running == old` → `Kept`, journal stays);
  - `running_bundle_inside_scratch_is_kept` (`Committed` journal, `running` = `<scratch dir>/old.app/Contents/MacOS/LingoPop` → `Kept`, the folder and the journal stay);
  - `crash_before_swap_never_installs_staged` (`Pending`, no `new_hash`, site = old bytes, staged present → site unchanged, staged deleted);
  - `crash_after_hash_before_swap_never_installs_staged` (`Pending`, `new_hash` set, site = old bytes, staged = new → site unchanged, staged deleted, journal gone);
  - `bundles_with_identical_executables_are_told_apart` (old and new bundles with the same main executable but a different version and `CodeResources` → identified correctly);
  - `interrupted_recovery_after_swap_back_is_idempotent` (run `recover` with a hook that stops right after the swap-back, then run it again → site keeps the old bytes, the new copy deleted, the journal gone);
  - `failed_stage_leaves_exe_untouched` (non-PE bytes, `Install`);
  - `failed_rollback_keeps_journal_and_old` is covered by `both_renames_fail_leaves_old_exe_and_journal`, which also asserts nothing was deleted;
  - `both_renames_fail_leaves_old_exe_and_journal` (second rename and rollback both forced to fail → `Err(Install)`, `<stem>.old.exe` holds the original bytes, the journal names it, and `recover` then restores it to the site);
  - `rollback_when_second_rename_fails` (staged path removed between the steps through a test hook `#[doc(hidden)] install_exe_with(.., before_second_rename: impl FnOnce())`);
  - `read_only_folder_is_unsupported` (Unix permissions; ignore on Windows);
  - `stem_comes_from_running_name` (`LingoPop-0.2.1-windows-x64-portable.exe` → `LingoPop-0.2.1-windows-x64-portable.old.exe`).
- [ ] **Step 2: Run** `cargo test -p lingopop-platform --test update_install_exe`. Expected: FAIL.
- [ ] **Step 3: Implement** with `std::fs::rename` only. The module is portable: no `unsafe`, compiled on every OS.
- [ ] **Step 4: Run** the tests on this Mac, then push so the Windows CI leg runs them too. Expected: PASS.
- [ ] **Step 5: Commit** with the AGENTS.md line.

### Task 6: macOS bundle checks and swap (platform)

**Files:**
- Create: `crates/lingopop-platform/src/update_install/bundle.rs` (portable logic over a seam)
- Create: `crates/lingopop-platform/src/macos/codesign.rs` (`/usr/bin/codesign` and `/usr/bin/ditto` through `std::process::Command`, no shell)
- Test: `crates/lingopop-platform/tests/update_install_bundle.rs` (fake seam, every OS)
- Test: `crates/lingopop-platform/tests/macos_update_bundle.rs` (real codesign on ad-hoc fake bundles, macOS)

**Interfaces:**
- Consumes: `InstallSite`, `Support`, `UpdateError` (Task 5), `RELEASE_CERT_SHA1` (Task 1) and `is_installed_bundle_path`.
- Produces:
  - `pub trait BundleTools: Send + Sync` with:
    - `designated_requirement(&self, app: &Path) -> Option<String>` (`codesign -d -r-`, the text after `designated => `);
    - `verify(&self, app: &Path, requirement: &str) -> bool` (`codesign --verify --strict --deep -R=<req>`);
    - `extract_zip(&self, zip: &Path, into: &Path) -> Result<(), UpdateError>` (`ditto -x -k --noqtn`);
    - `bundle_info(&self, app: &Path) -> Option<(String, String)>` (`CFBundleIdentifier`, `CFBundleShortVersionString` from `Info.plist`, read with the `plist`-free parser already used by the shell, or `defaults read`, the builder's call);
    - `executable_archs(&self, app: &Path) -> Vec<String>` (`lipo -archs`).
  - `pub fn pinned_requirement() -> String`: the exact DR from Global Constraints.
  - `pub fn bundle_support(app: &Path, tools: &dyn BundleTools) -> Support`. It is `Unsupported` unless:
    - the path passes `is_installed_bundle_path`;
    - the parent is writable;
    - `designated_requirement(app) == Some(pinned_requirement())`.
  - `pub fn install_bundle(site: &InstallSite, zip: &Path, version: &str, arch: &str, tools: &dyn BundleTools) -> Result<(), UpdateError>`.
    - It stages into `<parent>/.LingoPop-update-<version>/`.
    - It checks: exactly one `LingoPop.app`, the id, the version, `verify(new, pinned_requirement())`, and that the arch is present.
    - Then it swaps atomically with `BundleTools::swap(staged, site.path)`: afterwards the old bundle sits at the staged path, and a failed late check swaps back.
  - The swap is `BundleTools::swap(&self, a: &Path, b: &Path) -> Result<(), UpdateError>`, an atomic exchange.
    - The production version is `renamex_np(a, b, RENAME_SWAP)` through `libc` in `macos/codesign.rs`: the only new `unsafe`, with a `// SAFETY:` comment.
    - The rollback is the same swap again.
    - `site.path`, the validated running bundle path, is used for the swap, the rollback and the restart; `/Applications` is never hard-coded.
  - `install_bundle` takes `data_dir` and writes the Task 5 `Journal` before the swap. `recover` from Task 5 also handles bundles: directories are removed with `remove_dir_all`. No separate cleanup function exists.
  - `macos::codesign::SystemBundleTools` implements `BundleTools`.

- [ ] **Step 1: Write the failing tests** (fake tools, temp dir with a fake `LingoPop.app` folder tree):
  - `adhoc_running_app_is_unsupported` (DR `cdhash H"…"`);
  - `pinned_running_app_is_supported`;
  - `wrong_bundle_id_is_refused` (`Install`, old app untouched);
  - `wrong_version_is_refused`;
  - `requirement_mismatch_is_refused`;
  - `two_apps_in_zip_are_refused`;
  - `swap_keeps_old_app_until_cleanup`;
  - `failed_swap_leaves_app_untouched`;
  - `bundle_outside_applications_swaps_at_its_own_path` (site `~/Apps/LingoPop.app`, an unrelated `/tmp/x/Applications/LingoPop.app` untouched);
  - `failed_late_check_swaps_back`;
  - `failed_late_check_and_failed_swap_back_is_restored_by_recover` (both swaps fail after the first one → the journal stays `Pending`, and `recover` returns the old bundle to the site);
  - in `macos_update_bundle.rs`: `real_rename_swap_exchanges_two_folders`;
  - in `macos_update_bundle.rs`: `real_codesign_reads_the_adhoc_requirement` (ad-hoc-signed fake bundle → `cdhash`) and `real_verify_rejects_the_pinned_requirement_for_adhoc`.
- [ ] **Step 2: Run** `cargo test -p lingopop-platform --test update_install_bundle --test macos_update_bundle`. Expected: FAIL.
- [ ] **Step 3: Implement** `bundle.rs` and `macos/codesign.rs`.
  - Command output is parsed, never logged.
  - Logs carry fixed text only.
- [ ] **Step 4: Run** the tests. Expected: PASS. Run `bash scripts/check-macos-production.sh`: ok.
- [ ] **Step 5: Commit** with the AGENTS.md lines.

### Task 7: Shell update state machine, commands, relaunch and cleanup

**Files:**
- Modify: `app/src-tauri/src/updates.rs`
- Modify: `app/src-tauri/src/commands.rs:82-110` (`spawn_relaunch`, `app_relaunch`)
- Modify: `app/src-tauri/src/lib.rs` (setup: compute the `InstallSite` from `current_exe()` once, run the leftover cleanup, manage the state, register the commands at line ~424)
- Modify: `app/src-tauri/Cargo.toml` (no new deps expected)
- Test: `app/src-tauri/src/update_install_tests.rs`

**Interfaces:**
- Consumes: Tasks 3–6.
- Produces:
  - `pub trait Installer: Send + Sync` with:
    - `support(&self) -> &Support`;
    - `install(&self, plan: &UpdatePlan, downloaded: &Path) -> Result<(), UpdateError>`;
    - `relaunch(&self) -> std::io::Result<()>`.
  - Production `NativeInstaller` builds the relaunch:
    - Windows: the site exe with `--wait-for-pid <pid>`, detached as today;
    - macOS: `/usr/bin/open -n <site .app> --args --wait-for-pid <pid>`.
  - Commands, all async, errors as catalog keys:
    - `updates_check() -> UpdateView`;
    - `updates_install() -> UpdateView`;
    - `updates_cancel() -> UpdateView`;
    - `updates_restart() -> Result<(), String>`.
  - Event `updates://state` carries the payload `UpdateView`.
  - The `UpdateView` wire shape (camelCase) is `{ state: "idle" | "checking" | "available" | "downloading" | "installing" | "ready" | "failed" | "unsupported", version?: string, received?: number, total?: number, error?: string }`.
  - `app_relaunch` uses `Installer::relaunch`, never `current_exe()` after setup.
  - Startup calls `update_install::recover(data_dir, current_exe)` before the install site is computed.
    - After a `Restored` result the site is the journal's `site`.
    - The result is logged with fixed text.
  - `updates_state() -> UpdateView` is a read only. `UpdateView` also carries `revision: number`, which grows with every state change.
  - `UpdateTarget` is chosen by `cfg(target_os, target_arch)`.

- [ ] **Step 1: Write the failing tests** (`FakeTransport` for the release JSON, a local file server is not needed: inject a fake downloader closure and a fake `Installer`):
  - `check_available_then_install_reaches_ready`;
  - `install_is_single_flight`;
  - `cancel_returns_to_available_and_removes_files`;
  - `signature_failure_is_failed_and_installer_not_called`;
  - `retry_after_failed_rollback_is_blocked` (a journal that `recover` cannot resolve → `updates_install` is Failed, no download, the journal unchanged; a cancel afterwards leaves it too);
  - `journal_written_before_first_download` (the fake downloader sees the journal on disk, naming its destination in `scratch`);
  - `cancel_after_download_removes_scratch_and_journal`;
  - `size_mismatch_is_failed` (the downloader returns fewer bytes than `asset_size` → Failed, installer not called, files removed);
  - `unsupported_site_shows_download_only`;
  - `relaunch_after_swap_uses_install_path`;
  - `state_read_returns_ready_after_settings_reopen`;
  - `revision_grows_on_every_change`;
  - `progress_events_are_throttled` (at most 10 per second with `ManualClock`).
- [ ] **Step 2: Run** `cargo test -p lingopop-app update_install`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - First call `recover`; a journal left afterwards = Failed, nothing written.
  - After the checks that come before the swap, rewrite the journal with `new_hash`. After `Installer::install` and its checks succeeded, call `commit_journal`.
  - BEFORE any download, write the Task 5 `Journal` (`Pending`) with `site`, `staged`, `old` and `scratch` = the signature file, the download file and, on macOS, the staging folder.
  - The download goes to a temp file next to the site: Windows `stage_exe`, macOS `<parent>/.LingoPop-update-<v>/download.zip`.
  - The signature is downloaded first, then the asset, then `verify_update(read(file), sig, plan.expected_comment)`, then `Installer::install`.
  - The existing background loop and `SettingsView.latestVersion` stay unchanged.
- [ ] **Step 4: Run** `cargo test -p lingopop-app` and `cargo clippy --workspace --all-targets --locked -- -D warnings`. Expected: PASS, no warnings.
- [ ] **Step 5: Commit** with the AGENTS.md lines.

### Task 8: About tab update UI and catalogs

**Files:**
- Modify: `app/src/settings/AboutTab.svelte`
- Modify: `app/src/lib/settingsIpc.ts`
- Modify: `app/src/lib/settingsTypes.ts`
- Modify: `i18n/catalogs/{pl,uk,de}.json`
- Test: `app/src/settings/magicAbout.test.ts` (extend) or new `app/src/settings/aboutUpdate.test.ts`

**Interfaces:**
- Consumes: the Task 7 commands, the event and `UpdateView`.
- Produces:
  - IPC wrappers `checkUpdate()`, `installUpdate()`, `cancelUpdate()`, `restartForUpdate()`, `getUpdateState()`;
  - the type `UpdateView`.

- [ ] **Step 1: Write the failing tests** (mocked invoke / listen):
  - `available_shows_update_and_whats_new`;
  - `downloading_shows_percent_and_cancel`;
  - `ready_shows_restart`;
  - `failed_shows_try_again_and_download`;
  - `unsupported_shows_download_only`;
  - `listener_removed_on_unmount`;
  - `mount_registers_listener_before_reading_state` (`updates_state` is invoked only after `listen` resolved; a `ready` read shows Restart);
  - `stale_read_does_not_overwrite_newer_event` (an event with revision 5 arrives, then a read with revision 4 → still the revision 5 state).
- [ ] **Step 2: Run** `npm test --prefix app`. Expected: FAIL.
- [ ] **Step 3: Implement** the states from the spec section 7 table. The new shared catalog keys, with pl / uk / de:
  - `Update`;
  - `What's new`;
  - `Downloading… %lld%`;
  - `Installing…`;
  - `LingoPop %@ is installed. Restart to finish.`;
  - `Restart`;
  - `Try again`;
  - `Cancel` (exists);
  - `The update could not be verified.`;
  - `The update could not be downloaded.`;
  - `The update could not be installed. Your current version is unchanged.`;
  - `LingoPop can't update itself here. Download the new version instead.`
- [ ] **Step 4: Run** `npm test --prefix app`, `npm run check --prefix app` and `python3 scripts/gen-localizations.py check`. Expected: PASS and `ok:`.
- [ ] **Step 5: Commit.**

### Task 9: macOS `selfsigned` signing mode and tag-run gate

**Files:**
- Modify: `scripts/package-macos.sh` (mode selection at about line 81–94, the signing branch at about line 174–198, native checks, `signing-<slug>.txt`)
- Modify: `scripts/verify-release-assets.py` (`SIGNING_MODES`, `CONDITIONS` / `render_notes` condition `macos-unnotarized` for `adhoc` and `selfsigned`)
- Modify: `docs/INSTALL.md` (`macos-adhoc` blocks → `macos-unnotarized`; the re-grant paragraph stays `adhoc` only)
- Modify: `.github/workflows/release.yml` (the mac leg passes the two new secrets plus `RELEASE_TAG_RUN=1` on tag runs); unset `MACOS_SIGNING_P12` and `MACOS_SIGNING_P12_PASSWORD` for the Tauri build invocation
- Modify: `scripts/check-macos-production.sh`, if its rules need the new mode
- Test: `scripts/tests/test_release_assets.py`
- Test: `scripts/tests/test_package_macos_mode.py` (new, mode selection run as a function of env)

**Interfaces:**
- Produces:
  - `signing-<slug>.txt` ∈ `adhoc | selfsigned | developer-id`.
  - Mode selection:
    - `MACOS_SIGNING_P12` and `MACOS_SIGNING_P12_PASSWORD` both set → `selfsigned`;
    - only one of them set → fail, naming the missing one;
    - neither set → `adhoc`.
  - The workflow sets `RELEASE_TAG_RUN=1` only on tag runs AND only when `release-platforms.json` (read with `verify-release-assets.py`'s `read_release_platforms`, which fails closed) has `macosPreview: true`. With `macosPreview: false` the leg still builds and may be `adhoc`.
  - Tauri's environment: `MACOS_SIGNING_P12` and `MACOS_SIGNING_P12_PASSWORD` are removed from the `tauri build` invocation (`env -u`), exactly like the `APPLE_*` variables today; only the signing step reads them.
  - With `RELEASE_TAG_RUN=1` and the mode not `selfsigned` → fail with the message `tag releases must be signed with the pinned certificate`. The Developer ID path is never chosen when `RELEASE_TAG_RUN=1`.
  - Native checks for `selfsigned`: the imported identity SHA-1 equals `release-signing.json`, and `codesign -d -r-` equals the pinned DR.

- [ ] **Step 1: Write the failing tests:**
  - mode selection for none / one / both / tag-run with `macosPreview` true (no p12 → fail) and false (no `RELEASE_TAG_RUN`, `adhoc` accepted), and a check that the `tauri build` line in `package-macos.sh` unsets both p12 variables (by sourcing the selection function with `PACKAGE_MACOS_SELECT_ONLY=1`, which prints the mode and exits);
  - `selfsigned` is accepted in `assemble`;
  - `render_notes` for `selfsigned` contains the Open Anyway paragraph but not the ad-hoc re-grant paragraph;
  - `test_committed_install_template_renders_for_every_mode` covers `selfsigned`.
- [ ] **Step 2: Run** `python3 -m unittest discover -s scripts/tests`. Expected: FAIL.
- [ ] **Step 3: Implement.** Reuse the existing temporary-keychain code for the p12, sign with `codesign --force --sign <sha1> --keychain <kc> --options runtime --timestamp=none`, and keep `shellcheck` clean.
- [ ] **Step 4: Run** the unit tests, `shellcheck scripts/*.sh`, `bash scripts/check-macos-production.sh` and `actionlint`. Expected: all pass.
- [ ] **Step 5: Commit.**

### Task 10: Update signatures in the release

**Files:**
- Modify: `scripts/verify-release-assets.py`:
  - `asset_names` stays the UNSIGNED build set (used by the build legs and by `assemble`); a new `signed_asset_names` = that set plus the three `.minisig` names, used only by the post-signing check;
  - the manifest lists them;
  - a new check that the trusted-comment line of each `.minisig` is `trusted comment: lingopop-update <asset>`.
- Modify: `.github/workflows/release.yml` publish job:
  1. (A separate job `build-release-tools`, ubuntu, no secrets, runs before publish: `cargo build -p lingopop-release-tools --release --locked`, uploads the binary as an artifact, and exposes its SHA-256 as a job output.)
  2. In the publish job: download the tool artifact and compare its SHA-256 with that job output; a mismatch stops the release. The hash only proves the file was not swapped in transit; it does not make the binary or its dependencies trusted. The publish job builds nothing itself.
  3. Download the build artifacts.
  4. Run `assemble`.
  5. **Pre-signing check:** `verify_assets` on the unsigned set, without `.minisig`, against its manifest.
  6. **Sign** with `UPDATE_SIGNING_KEY` from a temp file (deleted in an `always()` step) and `UPDATE_SIGNING_KEY_PASSWORD`.
  7. **Post-signing check:**
     - run `verify` cryptographically against the committed `UPDATE_PUBLIC_KEY`;
     - regenerate `SHA256SUMS.txt` with the `.minisig` files;
     - run `verify_assets` on the full set.
  8. Create the draft.
  - The order is pinned by `test_release_workflow_publishes_a_checked_draft`.
- Modify: `scripts/verify-release-assets.py` `publish`. A `--sign-command` hook runs between `assemble` and `verify_assets`, or the workflow splits `publish` into `assemble` / `verify-and-create` subcommands (builder's call; keep "gh only after zero errors").
- Test: `scripts/tests/test_release_assets.py`

**Interfaces:**
- Consumes: the Task 2 CLI and Task 3 asset names.
- Produces: a release asset set of 9 files with Mac assets (exe, 2 dmg, 2 zip, 3 minisig, `SHA256SUMS.txt`), or 3 files Windows-only (exe, exe.minisig, `SHA256SUMS.txt`).

- [ ] **Step 1: Write the failing tests:**
  - the exact set with and without Mac assets;
  - a missing `.minisig` → error;
  - a `.minisig` with a wrong trusted comment → `<name>.minisig: trusted comment does not name the asset`;
  - the workflow order: sign before verify, verify before `gh release create`, and the key file removed in an `always()` step.
- [ ] **Step 2: Run** `python3 -m unittest discover -s scripts/tests`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the unit tests and `actionlint`. Then run a `workflow_dispatch` of Release on the branch: it builds all three legs with `selfsigned` (the secrets are set), and the publish job is skipped on dispatch. Expected: green. Download the mac artifact and check `codesign -d -r-` shows the pinned DR.
- [ ] **Step 5: Commit.**

### Task 11: Version 0.2.1, docs, PR

**Files:**
- Modify: version files (`Cargo.toml`, `app/src-tauri/tauri.conf.json`, `app/package.json`, `app/package-lock.json`, `Cargo.lock`)
- Modify: `CHANGELOG.md`:
  - the `## [0.2.1] - <date>` section takes over the Unreleased entries from PR #28;
  - plus "In-app updates" and the one-time manual install and re-grant note for 0.2.0 / Preview users.
- Modify: `README.md` (Updates on both OSes, the FAQ)
- Modify: `docs/settings.md` (About)
- Modify: `docs/RELEASING.md` (key and certificate custody, backup, never rotate, secrets list, tag-run rule)
- Modify: `SECURITY.md` (signed updates, self-signed certificate)
- Modify: `AGENTS.md` (release workflow bullet)

- [ ] **Step 1:** Run `python3 scripts/check-version.py --tag v0.2.1`. Expected: `0.2.1`.
- [ ] **Step 2:** Run the full local check list from AGENTS.md "Build / test". Expected: all pass.
- [ ] **Step 3:** Open the PR. Drive CodeRabbit to approval and resolve the threads.
  - Run a fresh whole-branch review: a delegate on another provider, Codex, if available.
  - Merge once green, approved and resolved (owner-authorized).

### Task 12: Release 0.2.1, then 0.2.2, end-to-end check (after merge)

- [ ] **Step 1: Release 0.2.1.**
  - Push the annotated tag `v0.2.1` on the merged main commit.
  - Watch Release until it is green.
  - Check that `releases/latest` is `v0.2.1` and that all 9 assets are there.
  - Run `lingopop-release-tools verify` on the downloaded exe and zips.
  - Check that `codesign -d -r-` on the zip's app shows the pinned DR.
- [ ] **Step 2: Install on this Mac.**
  - Quit 0.2.0 and Preview.
  - Install 0.2.1 into `/Applications/LingoPop.app` (Open Anyway once).
  - **The owner grants Accessibility and Screen Recording.**
- [ ] **Step 3: Release 0.2.2.**
  - Open a small PR: bump to 0.2.2 and add a CHANGELOG entry. Merge it.
  - Tag `v0.2.2` and verify it as in Step 1.
- [ ] **Step 4: Update in place.**
  - In 0.2.1, open Settings > About and run Check, Update, Restart.
  - Expected:
    - About shows 0.2.2;
    - `/Applications/LingoPop.app` `CFBundleShortVersionString` is 0.2.2;
    - Settings > Main > Permissions shows both permissions Granted;
    - a Translate from another app and a Screen Translate both work without System Settings;
    - after the next start, no `.LingoPop-update-*` folder is left.
- [ ] **Step 5: Report** the results to the owner, plus the optional Windows manual check: put the 0.2.1 exe in a writable folder, run Update, then Restart, and it shows 0.2.2.
