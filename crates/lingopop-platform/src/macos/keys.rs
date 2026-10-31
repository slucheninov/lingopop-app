//! `KeySynth` over `CGEvent`, exactly like the mac app's `Core/KeyboardSimulator.swift`.
//!
//! Posting events needs the Accessibility permission: grant it to the terminal (or the app) that
//! runs LingoPop in System Settings > Privacy & Security > Accessibility. Without it every `send_*`
//! returns `InputBlocked` instead of silently doing nothing.

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use lingopop_core::hotkey::Modifier;

use crate::error::PlatformError;
use crate::keys::KeySynth;

/// `kVK_ANSI_C` / `kVK_ANSI_V`.
const KEY_C: CGKeyCode = 8;
const KEY_V: CGKeyCode = 9;
/// `kVK_Command`, `kVK_Shift`, `kVK_Option`, `kVK_Control`.
const KEY_COMMAND: CGKeyCode = 0x37;
const KEY_SHIFT: CGKeyCode = 0x38;
const KEY_OPTION: CGKeyCode = 0x3A;
const KEY_CONTROL: CGKeyCode = 0x3B;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn CGEventSourceFlagsState(state_id: i32) -> u64;
}

fn event_error() -> PlatformError {
    PlatformError::Os {
        api: "CGEventCreateKeyboardEvent",
        code: 0,
    }
}

fn ensure_trusted() -> Result<(), PlatformError> {
    // SAFETY: AXIsProcessTrusted takes no arguments and only reads the process's TCC state.
    if unsafe { AXIsProcessTrusted() } != 0 {
        Ok(())
    } else {
        Err(PlatformError::InputBlocked)
    }
}

fn post(key: CGKeyCode, down: bool, flags: CGEventFlags) -> Result<(), PlatformError> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| event_error())?;
    let event = CGEvent::new_keyboard_event(source, key, down).map_err(|_| event_error())?;
    event.set_flags(flags);
    event.post(CGEventTapLocation::AnnotatedSession);
    Ok(())
}

fn chord(key: CGKeyCode) -> Result<(), PlatformError> {
    ensure_trusted()?;
    post(key, true, CGEventFlags::CGEventFlagCommand)?;
    post(key, false, CGEventFlags::CGEventFlagCommand)
}

/// Dev only. `Modifier::Win` is the Command key.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacKeys;

impl MacKeys {
    pub fn new() -> Self {
        Self
    }
}

impl KeySynth for MacKeys {
    fn modifiers_down(&self) -> Vec<Modifier> {
        // SAFETY: CGEventSourceFlagsState takes a plain state id (0 = combined session state) and
        // returns a bit mask; it has no memory side effects.
        let state =
            unsafe { CGEventSourceFlagsState(CGEventSourceStateID::CombinedSessionState as i32) };
        let flags = CGEventFlags::from_bits_truncate(state);
        [
            (CGEventFlags::CGEventFlagControl, Modifier::Ctrl),
            (CGEventFlags::CGEventFlagAlternate, Modifier::Alt),
            (CGEventFlags::CGEventFlagShift, Modifier::Shift),
            (CGEventFlags::CGEventFlagCommand, Modifier::Win),
        ]
        .into_iter()
        .filter(|(mask, _)| flags.contains(*mask))
        .map(|(_, m)| m)
        .collect()
    }

    fn release_modifiers(&self, modifiers: &[Modifier]) -> Result<(), PlatformError> {
        ensure_trusted()?;
        for m in modifiers {
            let key = match m {
                Modifier::Ctrl => KEY_CONTROL,
                Modifier::Alt => KEY_OPTION,
                Modifier::Shift => KEY_SHIFT,
                Modifier::Win => KEY_COMMAND,
            };
            post(key, false, CGEventFlags::CGEventFlagNull)?;
        }
        Ok(())
    }

    fn send_copy(&self) -> Result<(), PlatformError> {
        chord(KEY_C)
    }

    fn send_paste(&self) -> Result<(), PlatformError> {
        chord(KEY_V)
    }
}
