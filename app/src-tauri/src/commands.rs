//! Tauri commands called by the popup frontend.
//!
//! RULE: every command is `async`. Tauri runs sync commands on the main thread; the popup
//! commands take the orchestrator's session mutex, while async-runtime workers holding that
//! mutex do cross-thread `SetWindowPos` calls into the popup window, which the main thread
//! must be free to service. Actions are spawned so the webview never waits for a whole run.

use std::sync::Arc;

use lingopop_core::provider::ProviderType;
use lingopop_orchestrator::Orchestrator;
use serde::Deserialize;
use tauri::State;

use crate::popup_shell::ShellPopup;

/// Managed state shared by the commands.
pub struct ShellState {
    pub orchestrator: Orchestrator,
    pub popup: Arc<ShellPopup>,
    pub config: Arc<crate::state::ShellConfig>,
    pub stores: Arc<crate::state::ShellStores>,
    pub ui: Arc<crate::state::UiState>,
    pub platform: lingopop_platform::Platform,
    pub hooks: Arc<dyn crate::settings_cmds::SettingsHooks>,
    /// Serializes settings/UI saves through their side effects and emitted views.
    pub settings_write: tokio::sync::Mutex<()>,
    pub window_locks: std::collections::BTreeMap<&'static str, tokio::sync::Mutex<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PopupAction {
    Copy,
    Replace,
    Retry { provider: ProviderType },
    Rewrite,
    Explain,
    Retranslate { language: String },
    TranslateOcr { provider: ProviderType },
    Close,
}

#[tauri::command]
pub async fn popup_action(state: State<'_, ShellState>, action: PopupAction) -> Result<(), String> {
    let orchestrator = state.orchestrator.clone();
    tauri::async_runtime::spawn(async move {
        match action {
            PopupAction::Copy => orchestrator.copy().await,
            PopupAction::Replace => orchestrator.replace().await,
            PopupAction::Retry { provider } => orchestrator.retry(provider).await,
            PopupAction::Rewrite => orchestrator.rewrite().await,
            PopupAction::Explain => orchestrator.explain().await,
            PopupAction::Retranslate { language } => orchestrator.retranslate(language).await,
            PopupAction::TranslateOcr { provider } => orchestrator.translate_ocr(provider).await,
            PopupAction::Close => orchestrator.close().await,
        }
    });
    Ok(())
}

/// The measured popup content size in LOGICAL pixels.
#[tauri::command]
pub async fn popup_resized(
    state: State<'_, ShellState>,
    width: f64,
    height: f64,
) -> Result<(), String> {
    state.orchestrator.popup_resized((width, height));
    Ok(())
}

#[tauri::command]
pub async fn popup_ready(state: State<'_, ShellState>) -> Result<(), String> {
    state.popup.ready();
    Ok(())
}

fn spawn_relaunch() -> std::io::Result<()> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("--wait-for-pid")
        .arg(std::process::id().to_string());
    #[cfg(windows)]
    detach_windows(&mut command);
    command.spawn().map(|_| ())
}

#[cfg(windows)]
fn detach_windows(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    // `CREATE_NEW_PROCESS_GROUP` (0x200) and `DETACHED_PROCESS` (0x8),
    // `windows` 0.62.2 `Win32_System_Threading`.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
}

/// Spawn this executable with `--wait-for-pid` and exit. Spawn failure is the mac catalog key.
#[tauri::command]
pub async fn app_relaunch(app: tauri::AppHandle) -> Result<(), String> {
    if spawn_relaunch().is_err() {
        return Err("Could not relaunch LingoPop".to_string());
    }
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_action_json_shape() {
        let retry: PopupAction =
            serde_json::from_str(r#"{"type":"retry","provider":"Google (Free)"}"#).unwrap();
        assert_eq!(
            retry,
            PopupAction::Retry {
                provider: ProviderType::GoogleFree
            }
        );
        let retranslate: PopupAction =
            serde_json::from_str(r#"{"type":"retranslate","language":"Polish"}"#).unwrap();
        assert_eq!(
            retranslate,
            PopupAction::Retranslate {
                language: "Polish".into()
            }
        );
        let ocr: PopupAction =
            serde_json::from_str(r#"{"type":"translateOcr","provider":"DeepL"}"#).unwrap();
        assert_eq!(
            ocr,
            PopupAction::TranslateOcr {
                provider: ProviderType::DeepL
            }
        );
        for (json, expected) in [
            (r#"{"type":"copy"}"#, PopupAction::Copy),
            (r#"{"type":"replace"}"#, PopupAction::Replace),
            (r#"{"type":"rewrite"}"#, PopupAction::Rewrite),
            (r#"{"type":"explain"}"#, PopupAction::Explain),
            (r#"{"type":"close"}"#, PopupAction::Close),
        ] {
            assert_eq!(serde_json::from_str::<PopupAction>(json).unwrap(), expected);
        }
        assert!(serde_json::from_str::<PopupAction>(r#"{"type":"nope"}"#).is_err());
    }
}
