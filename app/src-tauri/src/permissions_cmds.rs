//! Permission IPC (macOS Accessibility and Screen Recording; Windows reports both granted).
//!
//! Every command is `async` and runs the platform call on `spawn_blocking`, never on the main
//! thread. Statuses are state reads only: bootstrap, settings reads and tray rebuilds never
//! prompt; only `permissions_request`, called from a button, can show the system prompt. The
//! webview names a kind by its fixed id (`accessibility` / `screenRecording`) and never supplies a
//! URL: the System Settings pane is chosen in `lingopop-platform`. Nothing here sees or logs
//! screenshot or selected-text data.

use crate::commands::ShellState;
use crate::settings_cmds::CATALOG_PLATFORM;
use lingopop_core::{i18n, ui_prefs::Language};
use lingopop_platform::{PermissionKind, PermissionState, Permissions};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::Mutex;
use tauri::State;

/// Catalog keys (macOS-only entries of `i18n/macos.json`) of the rejections.
const KEY_UNKNOWN_KIND: &str = "Couldn't update permissions.";
const KEY_OPEN_FAILED: &str = "Couldn't open System Settings.";

/// Kinds whose System Settings pane was opened in this process and that have not been seen
/// granted since: the user is on the retry-after-grant path. Process memory only.
#[derive(Default)]
pub struct PermissionHints(Mutex<BTreeSet<PermissionKind>>);

impl PermissionHints {
    fn set(&self) -> std::sync::MutexGuard<'_, BTreeSet<PermissionKind>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// One permission as the webview sees it: fixed ids only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    pub kind: &'static str,
    pub state: &'static str,
    /// Screen Recording only: Settings was opened and this process still does not see the grant,
    /// which on macOS 13 needs a relaunch of the app.
    pub relaunch_hint: bool,
}

/// Both kinds in `PermissionKind::ALL` order. State reads only: never prompts.
pub fn permission_statuses(
    permissions: &dyn Permissions,
    hints: &PermissionHints,
) -> Vec<PermissionStatus> {
    PermissionKind::ALL
        .into_iter()
        .map(|kind| {
            let state = permissions.state(kind);
            let mut opened = hints.set();
            if state == PermissionState::Granted {
                opened.remove(&kind);
            }
            PermissionStatus {
                kind: kind.id(),
                state: state.id(),
                relaunch_hint: kind == PermissionKind::ScreenRecording
                    && state != PermissionState::Granted
                    && opened.contains(&kind),
            }
        })
        .collect()
}

/// What the UI calls when the user comes back from System Settings: a fresh state read, which is
/// also where the retry-after-grant hint is decided. Never prompts.
pub fn refresh_permissions(
    permissions: &dyn Permissions,
    hints: &PermissionHints,
) -> Vec<PermissionStatus> {
    permission_statuses(permissions, hints)
}

fn parse_kind(id: &str, language: Language) -> Result<PermissionKind, String> {
    PermissionKind::from_id(id)
        .ok_or_else(|| i18n::t_for(CATALOG_PLATFORM, language, KEY_UNKNOWN_KIND, &[]))
}

/// The one call that may prompt: a user action for exactly `kind_id`.
pub fn request_permission(
    permissions: &dyn Permissions,
    hints: &PermissionHints,
    kind_id: &str,
    language: Language,
) -> Result<Vec<PermissionStatus>, String> {
    let kind = parse_kind(kind_id, language)?;
    permissions
        .request(kind)
        .map_err(|_| i18n::t_for(CATALOG_PLATFORM, language, KEY_UNKNOWN_KIND, &[]))?;
    Ok(permission_statuses(permissions, hints))
}

/// Opens the privacy pane of `kind_id` and remembers it for the retry-after-grant hint.
pub fn open_permission_settings(
    permissions: &dyn Permissions,
    hints: &PermissionHints,
    kind_id: &str,
    language: Language,
) -> Result<(), String> {
    let kind = parse_kind(kind_id, language)?;
    permissions.open_settings(kind).map_err(|_| {
        log::warn!("permission settings could not be opened");
        i18n::t_for(CATALOG_PLATFORM, language, KEY_OPEN_FAILED, &[])
    })?;
    hints.set().insert(kind);
    Ok(())
}

/// Runs a possibly blocking permission call off the async worker and the main thread.
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|_| "Couldn't update permissions.".to_string())
}

#[tauri::command]
pub async fn permissions_status(
    state: State<'_, ShellState>,
) -> Result<Vec<PermissionStatus>, String> {
    let (permissions, hints) = (
        state.platform.permissions.clone(),
        state.permission_hints.clone(),
    );
    blocking(move || permission_statuses(permissions.as_ref(), &hints)).await
}

#[tauri::command]
pub async fn permissions_refresh(
    state: State<'_, ShellState>,
) -> Result<Vec<PermissionStatus>, String> {
    let (permissions, hints) = (
        state.platform.permissions.clone(),
        state.permission_hints.clone(),
    );
    blocking(move || refresh_permissions(permissions.as_ref(), &hints)).await
}

#[tauri::command]
pub async fn permissions_request(
    state: State<'_, ShellState>,
    kind: String,
) -> Result<Vec<PermissionStatus>, String> {
    let (permissions, hints, language) = (
        state.platform.permissions.clone(),
        state.permission_hints.clone(),
        state.ui.language(),
    );
    blocking(move || request_permission(permissions.as_ref(), &hints, &kind, language)).await?
}

#[tauri::command]
pub async fn permissions_open_settings(
    state: State<'_, ShellState>,
    kind: String,
) -> Result<(), String> {
    let (permissions, hints, language) = (
        state.platform.permissions.clone(),
        state.permission_hints.clone(),
        state.ui.language(),
    );
    blocking(move || open_permission_settings(permissions.as_ref(), &hints, &kind, language))
        .await?
}
