//! History's webview projection: timestamps are Unix seconds and names are display names.
use crate::{commands::ShellState, state::ShellStores};
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
// `Err` strings are catalog keys already translated into the launched language, like
// `provider_cmds` / `settings_cmds` (the History window shows its own `t()` text and ignores them).
fn parse_id(language: Language, id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(id).map_err(|_| i18n::t(language, "Invalid id.", &[]))
}
fn copy_error(language: Language) -> String {
    i18n::t(language, "Couldn't copy.", &[])
}
fn read_error(language: Language) -> String {
    i18n::t(language, "History couldn't be read.", &[])
}
fn update_error(language: Language) -> String {
    i18n::t(language, "History couldn't be updated.", &[])
}
// An unavailable store is an error, never an empty list (the window would show the empty state).
fn list_rows(stores: &ShellStores, language: Language) -> Result<Vec<HistoryRow>, String> {
    stores
        .history_entries_checked()
        .map(|entries| entries.into_iter().map(|e| row(e, language)).collect())
        .ok_or_else(|| read_error(language))
}
fn remove_row(stores: &ShellStores, language: Language, id: &str) -> Result<(), String> {
    let id = parse_id(language, id)?;
    stores
        .remove_history(id)
        .map_err(|_| update_error(language))
}
fn clear_rows(stores: &ShellStores, language: Language) -> Result<(), String> {
    stores.clear_history().map_err(|_| update_error(language))
}
#[tauri::command]
pub async fn history_list(state: State<'_, ShellState>) -> Result<Vec<HistoryRow>, String> {
    list_rows(&state.stores, state.ui.language())
}
#[tauri::command]
pub async fn history_remove(state: State<'_, ShellState>, id: String) -> Result<(), String> {
    remove_row(&state.stores, state.ui.language(), &id)
}
#[tauri::command]
pub async fn history_clear(state: State<'_, ShellState>) -> Result<(), String> {
    clear_rows(&state.stores, state.ui.language())
}
#[tauri::command]
pub async fn clipboard_write(state: State<'_, ShellState>, text: String) -> Result<(), String> {
    state
        .orchestrator
        .write_clipboard(text)
        .await
        .map_err(|_| copy_error(state.ui.language()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_remove_rejects_bad_uuid() {
        assert_eq!(parse_id(Language::En, "bad").unwrap_err(), "Invalid id.");
    }
    #[test]
    fn history_errors_use_the_launched_language() {
        assert_eq!(parse_id(Language::De, "bad").unwrap_err(), "Ungültige ID.");
        assert_eq!(copy_error(Language::En), "Couldn't copy.");
        assert_eq!(copy_error(Language::Pl), "Nie udało się skopiować.");
        assert_eq!(copy_error(Language::Uk), "Не вдалося скопіювати.");
    }
    fn unavailable_stores(dir: &std::path::Path) -> std::sync::Arc<ShellStores> {
        // A directory where `history.json` should be makes the store fail to open.
        std::fs::create_dir(dir.join("history.json")).unwrap();
        let (_, stores, _) = crate::task4_tests::rig(dir);
        assert!(stores.history_entries_checked().is_none());
        stores
    }
    #[test]
    fn history_list_rejects_an_unavailable_store_but_accepts_an_empty_one() {
        let d = tempfile::tempdir().unwrap();
        let (_, opened, _) = crate::task4_tests::rig(d.path());
        assert!(list_rows(&opened, Language::En).unwrap().is_empty());
        let d = tempfile::tempdir().unwrap();
        let stores = unavailable_stores(d.path());
        assert_eq!(
            list_rows(&stores, Language::En).err().unwrap(),
            "History couldn't be read."
        );
        assert_eq!(
            list_rows(&stores, Language::De).err().unwrap(),
            "Der Verlauf konnte nicht gelesen werden."
        );
        // The best-effort tray reader still sees an empty list.
        assert!(stores.history_entries().is_empty());
    }
    #[test]
    fn history_remove_and_clear_reject_an_unavailable_store() {
        let d = tempfile::tempdir().unwrap();
        let stores = unavailable_stores(d.path());
        let id = "00000000-0000-0000-0000-000000000001";
        assert_eq!(
            remove_row(&stores, Language::En, id).unwrap_err(),
            "History couldn't be updated."
        );
        assert_eq!(
            clear_rows(&stores, Language::Pl).unwrap_err(),
            "Nie udało się zaktualizować historii."
        );
        assert_eq!(
            remove_row(&stores, Language::En, "bad").unwrap_err(),
            "Invalid id."
        );
    }
    #[test]
    fn history_remove_and_clear_report_a_failed_save_without_firing_the_hook() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let d = tempfile::tempdir().unwrap();
        let (_, stores, _) = crate::task4_tests::rig(d.path());
        let e: HistoryEntry = serde_json::from_value(serde_json::json!({"id":"00000000-0000-0000-0000-000000000001","timestamp":0,"original":"a","translated":"b","operation":"translate","provider":"Claude","targetLanguage":"German"})).unwrap();
        let id = e.id;
        lingopop_orchestrator::Persist::add_history(&*stores, e, 10);
        let fired = std::sync::Arc::new(AtomicUsize::new(0));
        let f = fired.clone();
        stores.set_on_change(std::sync::Arc::new(move |_| {
            f.fetch_add(1, Ordering::SeqCst);
        }));
        // Force the failure independently of permissions (root ignores read-only directories):
        // the atomic write renames its temp file onto `history.json`, and a directory sitting
        // at that path makes the rename fail on Unix and Windows alike.
        let target = lingopop_core::paths::DataPaths::new(d.path()).history_json();
        std::fs::remove_file(&target).unwrap();
        std::fs::create_dir(&target).unwrap();
        let removed = remove_row(&stores, Language::En, &id.to_string());
        let cleared = clear_rows(&stores, Language::En);
        std::fs::remove_dir(&target).unwrap();
        assert_eq!(removed.unwrap_err(), "History couldn't be updated.");
        assert_eq!(cleared.unwrap_err(), "History couldn't be updated.");
        assert_eq!(fired.load(Ordering::SeqCst), 0);
        // The failed saves left the entry in memory; a retry now succeeds and fires the hook.
        assert_eq!(stores.history_entries().len(), 1);
        remove_row(&stores, Language::En, &id.to_string()).unwrap();
        assert_eq!(fired.load(Ordering::SeqCst), 1);
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
