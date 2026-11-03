//! History's webview projection: timestamps are Unix seconds and names are display names.
use crate::commands::ShellState;
use lingopop_core::{i18n, operation::AIOperation, records::HistoryEntry, ui_prefs::Language};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    id: String,
    original: String,
    translated: String,
    source_language: Option<String>,
    target_language: String,
    provider_name: String,
    operation: Option<AIOperation>,
    operation_name: String,
    timestamp_unix: f64,
}
fn row(entry: HistoryEntry, language: Language) -> HistoryRow {
    let operation = entry.operation_type();
    let operation_name = i18n::t(
        language,
        operation.map_or(entry.operation.as_str(), |o| o.short_name()),
        &[],
    );
    let provider_name = entry
        .provider_type()
        .map_or(entry.provider.clone(), |p| p.display_name().into());
    HistoryRow {
        id: entry.id.to_string().to_uppercase(),
        original: entry.original,
        translated: entry.translated,
        source_language: entry.source_language,
        target_language: entry.target_language,
        provider_name,
        operation,
        operation_name,
        timestamp_unix: entry.timestamp.unix_seconds(),
    }
}
fn parse_id(id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(id).map_err(|_| "Invalid id.".into())
}
#[tauri::command]
pub async fn history_list(state: State<'_, ShellState>) -> Result<Vec<HistoryRow>, String> {
    Ok(state
        .stores
        .history_entries()
        .into_iter()
        .map(|e| row(e, state.ui.language()))
        .collect())
}
#[tauri::command]
pub async fn history_remove(state: State<'_, ShellState>, id: String) -> Result<(), String> {
    state.stores.remove_history(parse_id(&id)?);
    Ok(())
}
#[tauri::command]
pub async fn history_clear(state: State<'_, ShellState>) -> Result<(), String> {
    state.stores.clear_history();
    Ok(())
}
#[tauri::command]
pub async fn clipboard_write(state: State<'_, ShellState>, text: String) -> Result<(), String> {
    state
        .orchestrator
        .write_clipboard(text)
        .await
        .map_err(|_| "Couldn't copy.".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_remove_rejects_bad_uuid() {
        assert_eq!(parse_id("bad").unwrap_err(), "Invalid id.");
    }
    #[test]
    fn history_row_uses_display_names_and_unix() {
        let mut e = HistoryEntry::new(
            "a".into(),
            "b".into(),
            None,
            "German".into(),
            lingopop_core::provider::ProviderType::DeepL,
            AIOperation::Translate,
            lingopop_core::time::Timestamp::from_unix_seconds(1_700_000_000.0),
        );
        assert_eq!(row(e.clone(), Language::En).provider_name, "DeepL");
        assert_eq!(row(e.clone(), Language::En).operation_name, "Translation");
        assert_eq!(row(e.clone(), Language::De).operation_name, "Übersetzung");
        e.provider = "X".into();
        let r = row(e, Language::En);
        assert_eq!(r.provider_name, "X");
        assert_eq!(r.timestamp_unix, 1_700_000_000.0);
    }
}
