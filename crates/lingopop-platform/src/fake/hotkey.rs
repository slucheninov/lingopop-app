//! In-memory `HotkeyService`: registers chords, simulates conflicts and delivers fired events.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crossbeam_channel::{Receiver, Sender};
use lingopop_core::hotkey::Hotkey;

use crate::error::PlatformError;
use crate::hotkey::{to_chord, HotkeyEvent, HotkeyId, HotkeyService};

pub struct FakeHotkeyService {
    registered: Mutex<HashMap<u32, Hotkey>>,
    blocked: Mutex<HashSet<u32>>,
    tx: Sender<HotkeyEvent>,
    rx: Receiver<HotkeyEvent>,
}

impl Default for FakeHotkeyService {
    fn default() -> Self {
        Self::new()
    }
}

/// Same shape as the `global-hotkey` id: modifier bits above the key code.
fn chord_id(hotkey: &Hotkey) -> Result<u32, PlatformError> {
    let (modifiers, code) = to_chord(hotkey)?;
    Ok((modifiers.bits() << 16) | code as u32)
}

impl FakeHotkeyService {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            registered: Mutex::default(),
            blocked: Mutex::default(),
            tx,
            rx,
        }
    }

    /// Simulates another application owning this chord: registering it is a `HotkeyConflict`.
    pub fn block(&self, hotkey: &Hotkey) -> Result<(), PlatformError> {
        let id = chord_id(hotkey)?;
        self.blocked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id);
        Ok(())
    }

    /// Delivers a press of `id`; ignored when the id is not registered.
    pub fn fire(&self, id: HotkeyId) {
        if self
            .registered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&id.0)
        {
            let _ = self.tx.send(HotkeyEvent { id });
        }
    }
}

impl HotkeyService for FakeHotkeyService {
    fn register(&self, hotkey: &Hotkey) -> Result<HotkeyId, PlatformError> {
        let id = chord_id(hotkey)?;
        let mut registered = self
            .registered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if registered.contains_key(&id)
            || self
                .blocked
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(&id)
        {
            return Err(PlatformError::HotkeyConflict);
        }
        registered.insert(id, hotkey.clone());
        Ok(HotkeyId(id))
    }

    fn unregister(&self, id: HotkeyId) -> Result<(), PlatformError> {
        self.registered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id.0)
            .map(|_| ())
            .ok_or(PlatformError::InvalidHotkey)
    }

    fn events(&self) -> Receiver<HotkeyEvent> {
        self.rx.clone()
    }
}
