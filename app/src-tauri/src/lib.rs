//! LingoPop tray app: a thin Tauri shell wiring the real platform into `lingopop-orchestrator`.

pub mod paths;

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running LingoPop");
}
