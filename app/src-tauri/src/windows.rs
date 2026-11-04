//! Settings/history creation always runs on a worker, serialized per window label.
use crate::commands::ShellState;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
pub const SETTINGS_LABEL: &str = "settings";
pub const HISTORY_LABEL: &str = "history";
pub fn open_settings(app: &AppHandle, tab: Option<&str>) {
    app.state::<ShellState>()
        .ui
        .set_pending_tab(tab.map(str::to_owned));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        open_window(app, SETTINGS_LABEL).await;
    });
}
pub fn open_history(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        open_window(app, HISTORY_LABEL).await;
    });
}
async fn open_window(app: AppHandle, label: &'static str) {
    #[cfg(target_os = "macos")]
    if app
        .run_on_main_thread(lingopop_platform::activate_app)
        .is_err()
    {
        log::warn!("application activation could not be posted");
    }
    let state = app.state::<ShellState>();
    let _guard = state.window_locks[label].lock().await;
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.set_focus();
        if label == SETTINGS_LABEL {
            if let Some(tab) = state.ui.take_pending_tab() {
                let _ = app.emit_to(SETTINGS_LABEL, "settings://tab", tab);
            }
        }
        return;
    }
    let lang = state.ui.language();
    let builder = WebviewWindowBuilder::new(&app, label, WebviewUrl::App("index.html".into()));
    let builder = if label == SETTINGS_LABEL {
        builder
            .title(lingopop_core::i18n::t(lang, "LingoPop Settings", &[]))
            .inner_size(720.0, 560.0)
            .resizable(false)
            .center()
    } else {
        builder
            .title(lingopop_core::i18n::t(lang, "Translation History", &[]))
            .inner_size(540.0, 480.0)
            .min_inner_size(480.0, 360.0)
            .resizable(true)
    };
    match builder.build() {
        Ok(window) if label == SETTINGS_LABEL => {
            let registry = state.hotkeys.clone();
            window.on_window_event(move |event| on_settings_window_event(&registry, event));
        }
        Ok(_) => {}
        Err(_) => log::warn!("window creation failed: {label}"),
    }
}

/// Synchronous window callbacks only post a registry transition; they never wait or create windows.
pub fn on_settings_window_event(
    registry: &crate::hotkeys::HotkeyRegistry,
    event: &tauri::WindowEvent,
) {
    if matches!(
        event,
        tauri::WindowEvent::Destroyed | tauri::WindowEvent::Focused(false)
    ) {
        registry.suspend(false);
    }
}
