//! Shell coordination for private local clipboard history. Native calls always run outside locks.
use crate::{
    commands::ShellState,
    state::{lock, HistoryWriteError},
};
use lingopop_core::{
    clipboard_history::{ClipboardHistoryEntry, ClipboardHistoryStore},
    i18n,
};
use lingopop_platform::clipboard_observer::native_clipboard_generation;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tauri::State;
use uuid::Uuid;

pub struct ClipboardHistory {
    pub(crate) inner: Mutex<Inner>,
    pub revision: Arc<AtomicU64>,
    // Serializes commands only; platform callbacks and main-thread work never acquire this
    // async gate. Keeping it across copy ensures a later delete cannot be undone by that copy.
    // Store locks are always released before clipboard/platform work.
    pub(crate) actions: tokio::sync::Mutex<()>,
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}
pub(crate) struct Inner {
    store: Option<ClipboardHistoryStore>,
    limit: u32,
    pub(crate) mutating: bool,
    ignored_generation: Option<(u64, Option<std::collections::HashSet<u64>>)>,
}
impl ClipboardHistory {
    pub fn new(store: Option<ClipboardHistoryStore>, limit: u32) -> Self {
        Self {
            inner: Mutex::new(Inner {
                store,
                limit,
                mutating: false,
                ignored_generation: None,
            }),
            revision: Arc::new(AtomicU64::new(0)),
            actions: tokio::sync::Mutex::new(()),
            on_change: Mutex::new(None),
        }
    }
    pub fn set_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        *lock(&self.on_change) = Some(callback);
    }
    fn changed(&self) {
        let callback = lock(&self.on_change).clone();
        if let Some(callback) = callback {
            callback();
        }
    }
    pub fn entries(&self) -> Result<Vec<ClipboardHistoryEntry>, HistoryWriteError> {
        lock(&self.inner)
            .store
            .as_ref()
            .map(|s| s.entries().to_vec())
            .ok_or(HistoryWriteError::Unavailable)
    }
    pub fn observe(
        &self,
        revision: u64,
        generation: u64,
        text: String,
    ) -> Result<bool, HistoryWriteError> {
        {
            let mut inner = lock(&self.inner);
            if inner.mutating || revision != self.revision.load(Ordering::Acquire) {
                return Ok(false);
            }
            if inner
                .ignored_generation
                .as_ref()
                .is_some_and(|(ignored, fingerprints)| {
                    *ignored == generation
                        && fingerprints
                            .as_ref()
                            .is_none_or(|values| values.contains(&text_fingerprint(&text)))
                })
            {
                return Ok(true);
            }
            let limit = inner.limit;
            // A copied history entry is explicitly promoted by the command. A second notification
            // for the same top text needs no disk write or event.
            let store = inner.store.as_mut().ok_or(HistoryWriteError::Unavailable)?;
            if store.entries().first().is_some_and(|e| e.text == text) {
                return Ok(true);
            }
            store
                .record(text, limit)
                .map_err(|_| HistoryWriteError::Write)?;
        }
        self.changed();
        Ok(true)
    }
    // Fence both reads already in progress and notifications not yet read. Sampling generation
    // while mutating is set closes the gap without invoking a native API under the store mutex.
    pub(crate) fn destructive(
        &self,
        id: Option<Uuid>,
        generation: impl FnOnce() -> u64,
    ) -> Result<(), HistoryWriteError> {
        {
            let mut inner = lock(&self.inner);
            inner.mutating = true;
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        let generation = generation();
        let result = {
            let mut inner = lock(&self.inner);
            // Keep no deleted plaintext in the fence. For an individual deletion, a
            // pending foreign copy of different text must still be recorded.
            let fingerprint = id.and_then(|id| {
                inner.store.as_ref().and_then(|store| {
                    store
                        .entries()
                        .iter()
                        .find(|entry| entry.id == id)
                        .map(|entry| text_fingerprint(&entry.text))
                })
            });
            let result = inner
                .store
                .as_mut()
                .ok_or(HistoryWriteError::Unavailable)
                .and_then(|store| {
                    match id {
                        Some(id) => store.remove(id),
                        None => store.clear(),
                    }
                    .map_err(|_| HistoryWriteError::Write)
                });
            if result.is_ok() && (id.is_none() || fingerprint.is_some()) {
                if inner
                    .ignored_generation
                    .as_ref()
                    .is_some_and(|(previous, _)| *previous == generation)
                {
                    let (_, ignored) = inner
                        .ignored_generation
                        .as_mut()
                        .expect("checked generation");
                    if id.is_none() {
                        *ignored = None;
                    } else if let (Some(values), Some(value)) = (ignored, fingerprint) {
                        values.insert(value);
                    }
                } else {
                    inner.ignored_generation = Some((
                        generation,
                        fingerprint.map(|value| std::collections::HashSet::from([value])),
                    ));
                }
            }
            inner.mutating = false;
            self.revision.fetch_add(1, Ordering::AcqRel);
            result
        };
        if result.is_ok() {
            self.changed();
        }
        result
    }
    pub fn trim(&self, limit: u32) -> Result<(), HistoryWriteError> {
        {
            let mut inner = lock(&self.inner);
            self.revision.fetch_add(1, Ordering::AcqRel);
            inner.limit = limit;
            inner
                .store
                .as_mut()
                .ok_or(HistoryWriteError::Unavailable)?
                .trim(limit)
                .map_err(|_| HistoryWriteError::Write)?;
        }
        self.changed();
        Ok(())
    }
}
// Ephemeral in-memory equality marker, never persisted or exposed as diagnostic data.
fn text_fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}
fn error(state: &ShellState) -> String {
    i18n::t(
        state.ui.language(),
        "Clipboard history couldn't be updated.",
        &[],
    )
}
fn parse_id(state: &ShellState, id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(id).map_err(|_| i18n::t(state.ui.language(), "Invalid id.", &[]))
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    id: String,
    text: String,
    timestamp_unix: f64,
}
#[tauri::command]
pub async fn clipboard_history_list(state: State<'_, ShellState>) -> Result<Vec<Row>, String> {
    state
        .stores
        .clipboard
        .entries()
        .map(|entries| {
            entries
                .into_iter()
                .map(|e| Row {
                    id: e.id.to_string().to_uppercase(),
                    text: e.text,
                    timestamp_unix: e.timestamp.unix_seconds(),
                })
                .collect()
        })
        .map_err(|_| {
            i18n::t(
                state.ui.language(),
                "Clipboard history couldn't be read.",
                &[],
            )
        })
}
pub async fn copy(state: &ShellState, id: Uuid) -> Result<(), String> {
    let history = &state.stores.clipboard;
    let _action = history.actions.lock().await;
    let text = history
        .entries()
        .map_err(|_| error(state))?
        .into_iter()
        .find(|e| e.id == id)
        .map(|e| e.text)
        .ok_or_else(|| error(state))?;
    state
        .orchestrator
        .write_clipboard(text)
        .await
        .map_err(|_| i18n::t(state.ui.language(), "Couldn't copy.", &[]))?;
    {
        let mut inner = lock(&history.inner);
        let limit = inner.limit;
        let store = inner.store.as_mut().ok_or_else(|| error(state))?;
        if store.entries().first().is_some_and(|entry| entry.id == id) {
            return Ok(());
        }
        store.promote(id, limit).map_err(|_| error(state))?;
    }
    history.changed();
    Ok(())
}
#[tauri::command]
pub async fn clipboard_history_copy(
    state: State<'_, ShellState>,
    id: String,
) -> Result<(), String> {
    copy(&state, parse_id(&state, &id)?).await
}
#[tauri::command]
pub async fn clipboard_history_remove(
    state: State<'_, ShellState>,
    id: String,
) -> Result<(), String> {
    let id = parse_id(&state, &id)?;
    let _action = state.stores.clipboard.actions.lock().await;
    state
        .stores
        .clipboard
        .destructive(Some(id), native_clipboard_generation)
        .map_err(|_| error(&state))
}
#[tauri::command]
pub async fn clipboard_history_clear(state: State<'_, ShellState>) -> Result<(), String> {
    let _action = state.stores.clipboard.actions.lock().await;
    state
        .stores
        .clipboard
        .destructive(None, native_clipboard_generation)
        .map_err(|_| error(&state))
}
pub(crate) fn accept_observation(
    history: &ClipboardHistory,
    item: lingopop_platform::clipboard_observer::ClipboardObservation,
) -> bool {
    history
        .observe(item.revision, item.generation, item.text)
        .unwrap_or(false)
}

pub fn start(app: &tauri::AppHandle) {
    use tauri::Manager;
    let state = app.state::<ShellState>();
    let history = state.stores.clipboard.clone();
    let revision = history.revision.clone();
    match lingopop_platform::clipboard_observer::native_clipboard_observer_fenced(
        Arc::new(move |item| accept_observation(&history, item)),
        revision,
    ) {
        Ok(observer) => {
            app.manage(ObserverHandle(Mutex::new(Some(observer))));
        }
        Err(_) => log::warn!("clipboard observer could not start"),
    }
}
pub struct ObserverHandle(
    pub Mutex<Option<lingopop_platform::clipboard_observer::ClipboardObserver>>,
);
