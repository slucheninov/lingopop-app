//! Global hotkeys on the Tauri event-loop thread.
//!
//! THREADING RULE: `GlobalHotkeyService` is `!Send` on Windows (it owns an `HWND`). It is
//! created in `setup` (which runs on the event-loop thread) and parked in a `thread_local!` on
//! that thread; it is never moved. Only its `events()` receiver (which is `Send`) goes to a std
//! forwarder thread that spawns `Orchestrator::trigger` on the async runtime.
//! `tauri-plugin-global-shortcut` is not used.
//!
//! Screen Translate is NOT registered in Phase 4a: the capture overlay arrives with 4b.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use lingopop_core::hotkey::Hotkey;
use lingopop_core::operation::AIOperation;
use lingopop_orchestrator::Orchestrator;
use lingopop_platform::{GlobalHotkeyService, HotkeyId, HotkeyService, PlatformError};

thread_local! {
    /// Lives on the event-loop thread only (`setup`); dropping it unregisters the hotkeys.
    static SERVICE: RefCell<Option<GlobalHotkeyService>> = const { RefCell::new(None) };
}

fn registered_operations(
    hotkeys: &BTreeMap<AIOperation, Hotkey>,
) -> impl Iterator<Item = (AIOperation, &Hotkey)> {
    hotkeys
        .iter()
        .filter(|(operation, _)| **operation != AIOperation::ScreenTranslate)
        .map(|(operation, hotkey)| (*operation, hotkey))
}

/// Must be called on the event-loop thread (from `setup`).
pub fn install(
    orchestrator: Orchestrator,
    hotkeys: &BTreeMap<AIOperation, Hotkey>,
) -> Result<(), PlatformError> {
    let service = GlobalHotkeyService::new()?;
    let mut by_id: HashMap<HotkeyId, AIOperation> = HashMap::new();
    for (operation, hotkey) in registered_operations(hotkeys) {
        match service.register(hotkey) {
            Ok(id) => {
                by_id.insert(id, operation);
            }
            // Fixed text, the operation name and the key chord; never user text.
            Err(e) => log::warn!(
                "hotkey for {} ({}) was not registered: {e}",
                operation.english_name(),
                hotkey.display_string()
            ),
        }
    }
    log::info!("{} hotkeys registered", by_id.len());

    let events = service.events();
    SERVICE.with(|slot| *slot.borrow_mut() = Some(service));

    let spawned = std::thread::Builder::new()
        .name("lingopop-hotkey-forwarder".into())
        .spawn(move || {
            for event in events.iter() {
                let Some(operation) = by_id.get(&event.id).copied() else {
                    continue;
                };
                let orchestrator = orchestrator.clone();
                tauri::async_runtime::spawn(async move { orchestrator.trigger(operation).await });
            }
        });
    if spawned.is_err() {
        log::error!("hotkey forwarder thread could not start");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_translate_is_not_registered() {
        let hotkeys = lingopop_core::settings::Settings::default().hotkeys;
        assert!(registered_operations(&hotkeys).all(|(op, _)| op != AIOperation::ScreenTranslate));
        assert!(registered_operations(&hotkeys).count() > 0);
    }
}
