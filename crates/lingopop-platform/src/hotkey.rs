//! Global hotkeys: the pure mapping from the core `Hotkey` to a chord, the `HotkeyService` trait and
//! the `global-hotkey` based service shared by Windows and macOS.
//!
//! Threading: `GlobalHotkeyService` owns a `GlobalHotKeyManager`, which on Windows holds an `HWND`
//! and is `!Send`. It must be created on the thread that runs the OS event loop (Phase 4: the Tauri
//! event-loop thread) and stays there; the compiler enforces the second half. Only `Pressed` events
//! are delivered (releases are dropped) and the id is the `global-hotkey` id of the chord.

use std::str::FromStr;

use keyboard_types::{Code, Modifiers};
use lingopop_core::hotkey::{Hotkey, Modifier};

use crate::error::PlatformError;

/// Identifies one registration; for the real service it is the `global-hotkey` id of the chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HotkeyId(pub u32);

/// A registered hotkey was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyEvent {
    pub id: HotkeyId,
}

/// Registers global hotkeys. Deliberately NOT `Send + Sync`: the Windows manager holds an `HWND`,
/// so the real service must be created and used on the event-loop thread.
pub trait HotkeyService {
    fn register(&self, hotkey: &Hotkey) -> Result<HotkeyId, PlatformError>;
    fn unregister(&self, id: HotkeyId) -> Result<(), PlatformError>;
    /// Presses of hotkeys registered through this service. The receiver can move to another thread.
    fn events(&self) -> crossbeam_channel::Receiver<HotkeyEvent>;
}

/// Maps a core hotkey to modifiers and a `KeyboardEvent.code`. An invalid hotkey (unsupported key
/// or no modifier) or a key without a `Code` is `InvalidHotkey`.
pub fn to_chord(hotkey: &Hotkey) -> Result<(Modifiers, Code), PlatformError> {
    if !hotkey.is_valid() {
        return Err(PlatformError::InvalidHotkey);
    }
    let code = Code::from_str(&hotkey.key).map_err(|_| PlatformError::InvalidHotkey)?;
    let mut modifiers = Modifiers::empty();
    for modifier in &hotkey.modifiers {
        modifiers |= match modifier {
            Modifier::Ctrl => Modifiers::CONTROL,
            Modifier::Alt => Modifiers::ALT,
            Modifier::Shift => Modifiers::SHIFT,
            // `SUPER` is deprecated in keyboard-types; global-hotkey maps META to MOD_WIN / Cmd.
            Modifier::Win => Modifiers::META,
        };
    }
    Ok((modifiers, code))
}

#[cfg(any(windows, target_os = "macos"))]
pub use real::{map_error, to_global_hotkey, GlobalHotkeyService};

#[cfg(any(windows, target_os = "macos"))]
mod real {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use global_hotkey::hotkey::HotKey;
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use lingopop_core::hotkey::Hotkey;

    use super::{to_chord, HotkeyEvent, HotkeyId, HotkeyService};
    use crate::error::PlatformError;

    /// `ERROR_HOTKEY_ALREADY_REGISTERED`.
    const ERROR_HOTKEY_ALREADY_REGISTERED: i32 = 1409;
    const POLL: Duration = Duration::from_millis(100);

    pub fn to_global_hotkey(hotkey: &Hotkey) -> Result<HotKey, PlatformError> {
        let (modifiers, code) = to_chord(hotkey)?;
        Ok(HotKey::new(Some(modifiers), code))
    }

    /// Maps a `global-hotkey` error; public only so the integration tests can pin the table.
    #[doc(hidden)]
    pub fn map_error(error: global_hotkey::Error) -> PlatformError {
        use global_hotkey::Error;
        match error {
            Error::AlreadyRegistered(_) => PlatformError::HotkeyConflict,
            Error::OsError(e) if e.raw_os_error() == Some(ERROR_HOTKEY_ALREADY_REGISTERED) => {
                PlatformError::HotkeyConflict
            }
            Error::UnrecognizedHotKeyCode(_) | Error::FailedToRegister(_) => {
                PlatformError::InvalidHotkey
            }
            Error::OsError(e) => PlatformError::Os {
                api: "RegisterHotKey",
                code: e.raw_os_error().unwrap_or(0),
            },
            _ => PlatformError::Os {
                api: "RegisterHotKey",
                code: 0,
            },
        }
    }

    /// `global-hotkey` based service. Create it on the OS event-loop thread (see the module docs).
    ///
    /// `global-hotkey` has one process-wide event channel, so only one service may be alive at a
    /// time: a second live service's forwarder would compete for the same events. Dropping a
    /// service joins its forwarder, so dropping and recreating one (settings change) is safe.
    pub struct GlobalHotkeyService {
        manager: GlobalHotKeyManager,
        registered: Arc<Mutex<HashMap<u32, HotKey>>>,
        events: crossbeam_channel::Receiver<HotkeyEvent>,
        stop: Arc<AtomicBool>,
        forwarder: Option<std::thread::JoinHandle<()>>,
    }

    impl GlobalHotkeyService {
        pub fn new() -> Result<Self, PlatformError> {
            let manager = GlobalHotKeyManager::new().map_err(map_error)?;
            let registered: Arc<Mutex<HashMap<u32, HotKey>>> = Arc::default();
            let stop = Arc::new(AtomicBool::new(false));
            let (tx, rx) = crossbeam_channel::unbounded();
            let thread_registered = Arc::clone(&registered);
            let thread_stop = Arc::clone(&stop);
            // Forwards `Pressed` events of our ids from the global channel. Never use
            // `GlobalHotKeyEvent::set_event_handler`: it replaces the channel for the whole process.
            let forwarder = std::thread::Builder::new()
                .name("lingopop-hotkey-events".into())
                .spawn(move || {
                    let global = GlobalHotKeyEvent::receiver();
                    while !thread_stop.load(Ordering::Acquire) {
                        let Ok(event) = global.recv_timeout(POLL) else {
                            continue;
                        };
                        let ours = thread_registered
                            .lock()
                            .map(|map| map.contains_key(&event.id))
                            .unwrap_or(false);
                        if event.state == HotKeyState::Pressed
                            && ours
                            && tx
                                .send(HotkeyEvent {
                                    id: HotkeyId(event.id),
                                })
                                .is_err()
                        {
                            break;
                        }
                    }
                })
                .map_err(|_| PlatformError::Os {
                    api: "thread::spawn",
                    code: 0,
                })?;
            Ok(Self {
                manager,
                registered,
                events: rx,
                stop,
                forwarder: Some(forwarder),
            })
        }
    }

    impl Drop for GlobalHotkeyService {
        fn drop(&mut self) {
            // Join the forwarder (at most one poll interval) so it cannot outlive the service and
            // take a press meant for a later service; the manager is dropped after this body, on
            // the creating thread.
            self.stop.store(true, Ordering::Release);
            if let Some(handle) = self.forwarder.take() {
                let _ = handle.join();
            }
        }
    }

    impl HotkeyService for GlobalHotkeyService {
        fn register(&self, hotkey: &Hotkey) -> Result<HotkeyId, PlatformError> {
            let chord = to_global_hotkey(hotkey)?;
            self.manager.register(chord).map_err(map_error)?;
            self.registered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(chord.id(), chord);
            Ok(HotkeyId(chord.id()))
        }

        fn unregister(&self, id: HotkeyId) -> Result<(), PlatformError> {
            let chord = self
                .registered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id.0)
                .ok_or(PlatformError::InvalidHotkey)?;
            if let Err(e) = self.manager.unregister(chord) {
                // The OS still owns the chord: keep tracking it so a retry can succeed.
                self.registered
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(id.0, chord);
                return Err(map_error(e));
            }
            Ok(())
        }

        fn events(&self) -> crossbeam_channel::Receiver<HotkeyEvent> {
            self.events.clone()
        }
    }
}
