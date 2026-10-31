//! LingoPop tray app: a thin Tauri shell. All flow logic lives in `lingopop-orchestrator`;
//! this crate only wires the real platform, stores, popup window, hotkeys and tray into it.

pub mod commands;
pub mod dismiss;
pub mod hotkeys;
pub mod logging;
pub mod paths;
pub mod picker;
pub mod popup_shell;
pub mod state;
pub mod tray;

use std::error::Error;
use std::sync::Arc;

use lingopop_core::http::{HttpTimeouts, ReqwestTransport};
use lingopop_core::paths::DataPaths;
use lingopop_core::providers::ProviderRegistry;
use lingopop_core::runner::OperationRunner;
use lingopop_core::time::{Clock, SystemClock};
use lingopop_orchestrator::{ConfigSource, Deps, GateTimeouts, Orchestrator, PlatformGate};
use lingopop_platform::LogFn;
use tauri::{App, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

use crate::commands::ShellState;
use crate::popup_shell::{ShellPopup, POPUP_LABEL};

fn setup(app: &mut App, data_dir: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let platform = lingopop_platform::native();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let (config, stores) = state::load(&DataPaths::new(data_dir), &platform, clock.clone());

    // Created hidden at startup so WebView2's first navigation (which can take focus) happens
    // long before the first hotkey.
    let (width, height) = lingopop_orchestrator::INITIAL_POPUP_SIZE;
    WebviewWindowBuilder::new(app, POPUP_LABEL, WebviewUrl::App("index.html".into()))
        .visible(false)
        .focused(false)
        .focusable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(true)
        .inner_size(width, height)
        .build()?;

    let popup = Arc::new(ShellPopup::new(app.handle().clone(), platform.clone()));

    let registry = Arc::new(ProviderRegistry::standard());
    let runner = OperationRunner::new(
        ProviderRegistry::standard(),
        Arc::new(ReqwestTransport::new(HttpTimeouts::default())?),
    );
    let log: Arc<LogFn> = Arc::new(|message: &str| log::info!("{message}"));
    let hotkeys = config.settings().hotkeys;
    let dismiss = platform.dismiss.clone();
    let orchestrator = Orchestrator::new(Deps {
        platform,
        runner: Arc::new(runner),
        registry_has: Arc::new(move |provider| registry.get(provider).is_ok()),
        config,
        stores,
        popup: popup.clone(),
        gate: PlatformGate::new(GateTimeouts::default()),
        clock,
        log: Some(log),
        picker: Arc::new(picker::NoPicker),
    });

    app.manage(ShellState {
        orchestrator: orchestrator.clone(),
        popup,
    });

    dismiss::spawn_forwarder(dismiss.as_ref(), orchestrator.clone());
    // `setup` runs on the event-loop thread, as `GlobalHotkeyService` requires.
    if let Err(e) = hotkeys::install(orchestrator, &hotkeys) {
        log::error!("hotkeys are unavailable: {e}");
    }
    tray::build(app)?;
    log::info!("LingoPop started");
    Ok(())
}

pub fn run() {
    let data_dir = paths::data_dir();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("cannot create the data directory: {e}");
    }
    logging::init(&data_dir);

    tauri::Builder::default()
        // A second launch just exits (Phase 4b may open Settings here). Registered first.
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {}))
        .invoke_handler(tauri::generate_handler![
            commands::popup_action,
            commands::popup_resized,
            commands::popup_ready
        ])
        .setup(move |app| setup(app, &data_dir))
        .build(tauri::generate_context!())
        .expect("error while building LingoPop")
        .run(|_app, event| {
            // A tray app keeps running when its last window closes.
            if let RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
}
