//! LingoPop tray app: a thin Tauri shell. All flow logic lives in `lingopop-orchestrator`;
//! this crate only wires the real platform, stores, popup window, hotkeys and tray into it.

pub mod commands;
pub mod dismiss;
pub mod hotkeys;
pub mod logging;
pub mod paths;
pub mod picker;
pub mod popup_shell;
pub mod provider_cmds;
pub mod settings_cmds;
pub mod state;
pub mod tray;
pub mod windows;

use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use lingopop_core::http::{HttpTimeouts, ReqwestTransport};
use lingopop_core::paths::DataPaths;
use lingopop_core::providers::ProviderRegistry;
use lingopop_core::runner::OperationRunner;
use lingopop_core::time::{Clock, SystemClock};
use lingopop_orchestrator::{ConfigSource, Deps, GateTimeouts, Orchestrator, PlatformGate};
use lingopop_platform::LogFn;
use tauri::{App, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

use crate::commands::ShellState;
use crate::popup_shell::{ShellPopup, POPUP_LABEL};
use crate::settings_cmds::SettingsHooks;

/// Runs a login-item update off the caller. Production uses the Tauri blocking pool.
trait LoginSpawn: Send + Sync {
    fn spawn_login(&self, job: Box<dyn FnOnce() + Send>);
}

struct BlockingLogin;

impl LoginSpawn for BlockingLogin {
    fn spawn_login(&self, job: Box<dyn FnOnce() + Send>) {
        drop(tauri::async_runtime::spawn_blocking(job));
    }
}

/// Test executor: the registry write finishes before `launch_at_login` returns, so
/// `FakeLoginItem::calls` is visible without a Tauri runtime.
#[cfg(test)]
struct InlineLogin;

#[cfg(test)]
impl LoginSpawn for InlineLogin {
    fn spawn_login(&self, job: Box<dyn FnOnce() + Send>) {
        job();
    }
}

pub(crate) struct ShellHooks {
    login: Arc<dyn lingopop_platform::LoginItem>,
    spawn: Arc<dyn LoginSpawn>,
    serial: Arc<std::sync::Mutex<()>>,
}

impl ShellHooks {
    pub(crate) fn new(login: Arc<dyn lingopop_platform::LoginItem>) -> Self {
        Self {
            login,
            spawn: Arc::new(BlockingLogin),
            serial: Arc::new(std::sync::Mutex::new(())),
        }
    }

    /// Same as [`Self::new`], but runs `set_enabled` on the calling thread.
    #[cfg(test)]
    pub(crate) fn synchronous(login: Arc<dyn lingopop_platform::LoginItem>) -> Self {
        Self {
            login,
            spawn: Arc::new(InlineLogin),
            serial: Arc::new(std::sync::Mutex::new(())),
        }
    }
}

impl SettingsHooks for ShellHooks {
    fn launch_at_login(&self, enabled: bool) {
        let login = Arc::clone(&self.login);
        let serial = Arc::clone(&self.serial);
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(_) => {
                log::warn!("login item update failed: current executable is unavailable");
                return;
            }
        };
        self.spawn.spawn_login(Box::new(move || {
            let _guard = serial
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Err(error) = login.set_enabled(enabled, &exe) {
                log::warn!("login item update failed: {error}");
            }
        }));
    }

    fn hotkeys(
        &self,
        _hotkeys: &std::collections::BTreeMap<
            lingopop_core::operation::AIOperation,
            lingopop_core::hotkey::Hotkey,
        >,
    ) {
    }
}

/// Applies the loaded `launch_at_login` flag once, the way `setup` does at startup.
pub(crate) fn apply_startup_login(hooks: &dyn SettingsHooks, enabled: bool) {
    hooks.launch_at_login(enabled);
}

/// `--wait-for-pid <pid>` as its own argument. A missing or non-u32 value is `None`.
pub fn parse_wait_pid(args: &[String]) -> Option<u32> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--wait-for-pid" {
            return args.next()?.parse().ok();
        }
    }
    None
}

fn setup(app: &mut App, data_dir: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let platform = lingopop_platform::native();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let (config, stores, ui) = state::load(&DataPaths::new(data_dir), &platform, clock.clone());

    // Created hidden at startup so WebView2's first navigation (which can take focus) happens
    // long before the first hotkey.
    let (width, height) = lingopop_orchestrator::INITIAL_POPUP_SIZE;
    let window = WebviewWindowBuilder::new(app, POPUP_LABEL, WebviewUrl::App("index.html".into()))
        .visible(false)
        .focused(false)
        .focusable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        // With a shadow tao insets the client area of an undecorated window, so the
        // SetWindowPos rect would no longer equal the webview size (scrollbars, clipped close).
        .shadow(false)
        .inner_size(width, height)
        .build()?;

    // Cached once, here on the main thread: `hwnd()` blocks on the main thread on Windows.
    let handle = popup_shell::native_handle(&window);
    let popup = Arc::new(ShellPopup::new(
        app.handle().clone(),
        platform.clone(),
        handle,
    ));

    let registry = Arc::new(ProviderRegistry::standard());
    let runner = OperationRunner::new(
        ProviderRegistry::standard(),
        Arc::new(ReqwestTransport::new(HttpTimeouts::default())?),
    );
    let log: Arc<LogFn> = Arc::new(|message: &str| log::info!("{message}"));
    #[cfg(windows)]
    lingopop_platform::set_dismiss_log(log.clone());
    let hotkeys = config.settings().hotkeys;
    let dismiss = platform.dismiss.clone();
    let orchestrator = Orchestrator::new(Deps {
        platform: platform.clone(),
        runner: Arc::new(runner),
        registry_has: Arc::new(move |provider| registry.get(provider).is_ok()),
        config: config.clone(),
        stores: stores.clone(),
        popup: popup.clone(),
        gate: PlatformGate::new(GateTimeouts::default()),
        clock,
        log: Some(log),
        picker: Arc::new(picker::NoPicker),
    });

    let events = app.handle().clone();
    stores.set_on_change(Arc::new(move |kind| {
        let event = match kind {
            state::StoreKind::History => "history://changed",
            state::StoreKind::Usage => "usage://changed",
        };
        let _ = events.emit(event, ());
    }));
    let hooks: Arc<dyn SettingsHooks> = Arc::new(ShellHooks::new(platform.login_item.clone()));
    apply_startup_login(hooks.as_ref(), config.settings().launch_at_login);
    app.manage(ShellState {
        orchestrator: orchestrator.clone(),
        popup,
        config,
        stores,
        ui,
        platform,
        hooks,
        settings_write: tokio::sync::Mutex::new(()),
        window_locks: [windows::SETTINGS_LABEL, windows::HISTORY_LABEL]
            .into_iter()
            .map(|label| (label, tokio::sync::Mutex::new(())))
            .collect(),
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
    if let Some(pid) = parse_wait_pid(&std::env::args().collect::<Vec<_>>()) {
        let platform = lingopop_platform::native();
        if !platform
            .system
            .wait_for_process_exit(pid, Duration::from_secs(10))
        {
            log::warn!("previous instance still running");
        }
    }

    let app = tauri::Builder::default()
        // Registered first; callbacks only schedule window creation.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            windows::open_settings(app, None)
        }))
        .invoke_handler(tauri::generate_handler![
            commands::popup_action,
            commands::popup_resized,
            commands::popup_ready,
            settings_cmds::settings_get,
            settings_cmds::settings_update,
            settings_cmds::ui_update,
            settings_cmds::ui_bootstrap,
            settings_cmds::open_url,
            provider_cmds::provider_state,
            provider_cmds::provider_validate,
            provider_cmds::provider_save,
            provider_cmds::provider_clear_key,
            commands::app_relaunch
        ])
        .setup(move |app| setup(app, &data_dir))
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(e) => {
            // `windows_subsystem = "windows"` hides stderr: the log file is the only trace.
            // Tauri build errors carry no user text.
            log::error!("LingoPop failed to start: {e}");
            log::logger().flush();
            std::process::exit(1);
        }
    };
    app.run(|_app, event| {
        // A tray app keeps running when its last window closes.
        if let RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            api.prevent_exit();
        }
    });
}

#[cfg(test)]
mod task4_tests;
#[cfg(test)]
mod task5_tests;
#[cfg(test)]
mod task7_tests;
