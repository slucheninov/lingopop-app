//! LingoPop tray app: a thin Tauri shell. All flow logic lives in `lingopop-orchestrator`;
//! this crate only wires the real platform, stores, popup window, hotkeys and tray into it.

pub mod commands;
pub mod dismiss;
mod history_cmds;
pub mod hotkeys;
pub mod logging;
#[cfg(target_os = "macos")]
pub mod macos_main_thread;
pub mod paths;
pub mod permissions_cmds;
pub mod picker;
pub mod popup_shell;
pub mod provider_cmds;
pub mod settings_cmds;
pub mod state;
pub mod swift_source;
pub mod tray;
pub mod updates;
pub mod usage_cmds;
pub mod windows;

use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use lingopop_core::http::{HttpTimeouts, ReqwestTransport, Transport};
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
    hotkeys: Option<hotkeys::HotkeyRegistry>,
    login: Arc<dyn lingopop_platform::LoginItem>,
    spawn: Arc<dyn LoginSpawn>,
    serial: Arc<std::sync::Mutex<()>>,
    /// Newest requested value; an older queued job that lost the race skips its write.
    latest_login: Arc<AtomicBool>,
}

impl ShellHooks {
    pub(crate) fn new(login: Arc<dyn lingopop_platform::LoginItem>) -> Self {
        Self {
            hotkeys: None,
            login,
            spawn: Arc::new(BlockingLogin),
            serial: Arc::new(std::sync::Mutex::new(())),
            latest_login: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Same as [`Self::new`], but runs `set_enabled` on the calling thread.
    #[cfg(test)]
    pub(crate) fn synchronous(login: Arc<dyn lingopop_platform::LoginItem>) -> Self {
        Self {
            hotkeys: None,
            login,
            spawn: Arc::new(InlineLogin),
            serial: Arc::new(std::sync::Mutex::new(())),
            latest_login: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl SettingsHooks for ShellHooks {
    fn launch_at_login(&self, enabled: bool) {
        let login = Arc::clone(&self.login);
        let serial = Arc::clone(&self.serial);
        let latest = Arc::clone(&self.latest_login);
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(_) => {
                log::warn!("login item update failed: current executable is unavailable");
                return;
            }
        };
        self.latest_login.store(enabled, Ordering::Release);
        self.spawn.spawn_login(Box::new(move || {
            let _guard = serial
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if latest.load(Ordering::Acquire) != enabled {
                return;
            }
            if let Err(error) = login.set_enabled(enabled, &exe) {
                log::warn!("login item update failed: {error}");
            }
        }));
    }

    fn hotkeys(
        &self,
        hotkeys: &std::collections::BTreeMap<
            lingopop_core::operation::AIOperation,
            lingopop_core::hotkey::Hotkey,
        >,
    ) {
        if let Some(registry) = &self.hotkeys {
            registry.apply(hotkeys.clone());
        }
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
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    // Before `native()`: the platform logs its own startup failures (e.g. the tracker) through it.
    let log: Arc<LogFn> = Arc::new(|message: &str| log::info!("{message}"));
    lingopop_platform::set_platform_log(log.clone());
    let platform = lingopop_platform::native();
    #[cfg(target_os = "macos")]
    if platform.monitors.initialize_on_main_thread().is_err() {
        log::warn!("display topology initialization failed");
    }
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
        // macOS: the first click into the never-key panel reaches the webview (buttons work
        // without first making the panel key). Ignored on Windows.
        .accept_first_mouse(true)
        .inner_size(width, height)
        .build()?;

    // Cached once, here on the main thread: `hwnd()` blocks on the main thread on Windows;
    // on macOS this converts the window into the non-activating panel (main thread only).
    let handle = popup_shell::native_handle(&window);
    // macOS: NSEvent dismiss monitors are installed through the main-thread dispatcher.
    #[cfg(target_os = "macos")]
    platform
        .dismiss
        .bind_main_thread(Arc::new(macos_main_thread::TauriMacMainThread(
            app.handle().clone(),
        )));
    let popup = Arc::new(ShellPopup::new(
        app.handle().clone(),
        platform.clone(),
        handle,
    ));

    let registry = Arc::new(ProviderRegistry::standard());
    let transport: Arc<dyn Transport> = Arc::new(ReqwestTransport::new(HttpTimeouts::default())?);
    let runner = OperationRunner::new(ProviderRegistry::standard(), transport.clone());
    let hotkeys = config.settings().hotkeys;
    let dismiss = platform.dismiss.clone();
    let tray_sink = Arc::new(tray::TraySink::default());
    let (tray_changes, tray_rx) = tokio::sync::watch::channel(());
    let history_changes = tray_changes.clone();
    let orchestrator = Orchestrator::new(Deps {
        status: Some(tray_sink.clone()),
        platform: platform.clone(),
        runner: Arc::new(runner),
        registry_has: Arc::new(move |provider| registry.get(provider).is_ok()),
        config: config.clone(),
        stores: stores.clone(),
        popup: popup.clone(),
        gate: PlatformGate::new(GateTimeouts::default()),
        clock: clock.clone(),
        log: Some(log),
        picker: Arc::new(picker::OverlayPicker::new(platform.overlay.clone())),
    });

    let events = app.handle().clone();
    stores.set_on_change(Arc::new(move |kind| {
        let event = match kind {
            state::StoreKind::History => "history://changed",
            state::StoreKind::Usage => "usage://changed",
        };
        let _ = events.emit(event, ());
        if matches!(kind, state::StoreKind::History) {
            let _ = history_changes.send(());
        }
    }));
    let events = app.handle().clone();
    let hotkey_registry = hotkeys::HotkeyRegistry::new(
        Arc::new(hotkeys::TauriMainThread(app.handle().clone())),
        Arc::new(move || {
            let _ = events.emit("settings://changed", ());
        }),
    );
    #[cfg(target_os = "macos")]
    let hotkey_registry =
        hotkey_registry.with_swift_source(swift_source::SwiftSource::new(platform.system.clone()));
    #[cfg(target_os = "macos")]
    hotkey_registry.observe_swift();
    let mut shell_hooks = ShellHooks::new(platform.login_item.clone());
    shell_hooks.hotkeys = Some(hotkey_registry.clone());
    let hooks: Arc<dyn SettingsHooks> = Arc::new(shell_hooks);
    apply_startup_login(hooks.as_ref(), config.settings().launch_at_login);
    let update_state = Arc::new(updates::UpdateState::new(transport, ui.clone(), clock));
    app.manage(update_state.clone());
    app.manage(ShellState {
        hotkeys: hotkey_registry.clone(),
        orchestrator: orchestrator.clone(),
        popup,
        config,
        stores,
        ui,
        platform,
        hooks,
        settings_write: tokio::sync::Mutex::new(()),
        permission_hints: Arc::new(permissions_cmds::PermissionHints::default()),
        window_locks: [windows::SETTINGS_LABEL, windows::HISTORY_LABEL]
            .into_iter()
            .map(|label| (label, tokio::sync::Mutex::new(())))
            .collect(),
    });

    dismiss::spawn_forwarder(dismiss.as_ref(), orchestrator.clone());
    hotkeys::install(&hotkey_registry, orchestrator, &hotkeys);
    let tray = tray::TrayController::build(app.handle(), app.state::<ShellState>().ui.language())?;
    let _ = tray_sink.0.set(tray.clone());
    tray::watch(app.handle(), tray, tray_rx, tray_changes);
    updates::spawn_loop(update_state, app.handle().clone());
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
        let system = lingopop_platform::native_system();
        if !system.wait_for_process_exit(pid, Duration::from_secs(10)) {
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
            provider_cmds::provider_reset_secrets,
            provider_cmds::provider_state,
            provider_cmds::provider_validate,
            provider_cmds::provider_save,
            provider_cmds::provider_clear_key,
            hotkeys::hotkeys_suspend,
            permissions_cmds::permissions_status,
            permissions_cmds::permissions_refresh,
            permissions_cmds::permissions_request,
            permissions_cmds::permissions_open_settings,
            history_cmds::history_list,
            history_cmds::history_remove,
            history_cmds::history_clear,
            history_cmds::clipboard_write,
            usage_cmds::usage_get,
            usage_cmds::usage_reset,
            updates::updates_check,
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
    app.run(|app, event| match event {
        // A tray app keeps running when its last window closes.
        RunEvent::ExitRequested {
            code: None, api, ..
        } => api.prevent_exit(),
        // Last main-thread turn: remove the dismiss monitors inline (we are on main) and
        // stop the presentation worker without waiting for it.
        RunEvent::Exit => {
            if let Some(state) = app.try_state::<ShellState>() {
                state.platform.dismiss.shutdown();
                drop(state.popup.shutdown());
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod permission_tests;
#[cfg(test)]
mod task4_tests;
#[cfg(test)]
mod task5_tests;
#[cfg(test)]
mod task7_tests;
