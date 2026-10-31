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
    if let Some(rect) = state.popup.resized(width, height) {
        state.orchestrator.popup_resized(rect);
    }
    Ok(())
}

#[tauri::command]
pub async fn popup_ready(state: State<'_, ShellState>) -> Result<(), String> {
    state.popup.ready();
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
