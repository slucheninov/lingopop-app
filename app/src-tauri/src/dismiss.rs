//! Forwards the platform dismiss events (Esc, click outside) to the orchestrator.

use lingopop_orchestrator::Orchestrator;
use lingopop_platform::DismissWatcher;

/// Starts a std thread that blocks on the watcher's events; each one runs on the async
/// runtime. The thread ends when the watcher (and so its sender) is dropped.
pub fn spawn_forwarder(watcher: &dyn DismissWatcher, orchestrator: Orchestrator) {
    let events = watcher.events();
    let spawned = std::thread::Builder::new()
        .name("lingopop-dismiss-events".into())
        .spawn(move || {
            for event in events.iter() {
                let orchestrator = orchestrator.clone();
                tauri::async_runtime::spawn(async move { orchestrator.dismissed(event).await });
            }
        });
    if spawned.is_err() {
        log::error!("dismiss event thread could not start");
    }
}
