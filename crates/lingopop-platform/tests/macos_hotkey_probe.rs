#![cfg(target_os = "macos")]
/// Owner must stop Swift LingoPop before interpreting a conflict as an OS restriction.
#[test]
#[ignore = "owner-run: registers a real global hotkey"]
fn option_t_global_hotkey_probe() {
    use global_hotkey::{
        hotkey::{Code, HotKey, Modifiers},
        GlobalHotKeyManager,
    };
    let manager = match GlobalHotKeyManager::new() {
        Ok(manager) => manager,
        Err(_) => {
            println!("Option+T: error creating manager");
            return;
        }
    };
    let chord = HotKey::new(Some(Modifiers::ALT), Code::KeyT);
    match manager.register(chord) {
        Ok(()) => {
            println!("Option+T: ok");
            manager.unregister(chord).expect("probe cleanup");
        }
        Err(error) => {
            let mapped = lingopop_platform::hotkey::map_error(error);
            if mapped == lingopop_platform::PlatformError::HotkeyConflict {
                println!("Option+T: conflict (another app may own it)");
            } else {
                println!("Option+T: error ({mapped})");
            }
        }
    }
}
