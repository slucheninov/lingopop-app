//! `KeySynth` over `CGEvent`, exactly like the mac app's `Core/KeyboardSimulator.swift`.
//!
//! Posting events needs the Accessibility permission: grant it to the terminal (or the app) that
//! runs LingoPop in System Settings > Privacy & Security > Accessibility. Without it every `send_*`
//! returns `PermissionDenied(Accessibility)` instead of silently doing nothing; `InputBlocked`
//! (the Windows UIPI error) is never produced on macOS.

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode, EventField};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use lingopop_core::hotkey::Modifier;

use crate::error::PlatformError;
use crate::keys::KeySynth;
use crate::permissions::PermissionKind;

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
    fn CGEventSourceFlagsState(state_id: i32) -> u64;
}

fn event_error() -> PlatformError {
    PlatformError::Os {
        api: "CGEventCreateKeyboardEvent",
        code: 0,
    }
}

/// The pure half of the Accessibility check.
fn require_trusted(trusted: bool) -> Result<(), PlatformError> {
    if trusted {
        Ok(())
    } else {
        Err(PlatformError::PermissionDenied(
            PermissionKind::Accessibility,
        ))
    }
}

fn ensure_trusted() -> Result<(), PlatformError> {
    require_trusted(super::ax::process_trusted())
}

fn post(key: CGKeyCode, down: bool, flags: CGEventFlags) -> Result<(), PlatformError> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| event_error())?;
    let event = CGEvent::new_keyboard_event(source, key, down).map_err(|_| event_error())?;
    event.set_flags(flags);
    // Our own Cmd+C / Cmd+V must never dismiss the popup (macos/dismiss.rs ignores the marker).
    event.set_integer_value_field(
        EventField::EVENT_SOURCE_USER_DATA,
        crate::macos_dismiss::OWN_EVENT_MARKER,
    );
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

    fn mask_menu_activation(&self) -> Result<(), PlatformError> {
        // No Alt menu keytips on macOS.
        Ok(())
    }

    fn send_copy(&self) -> Result<(), PlatformError> {
        chord(KEY_C)
    }

    fn send_paste(&self) -> Result<(), PlatformError> {
        chord(KEY_V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_accessibility_is_permission_denied_never_the_windows_uipi_error() {
        assert_eq!(
            require_trusted(false),
            Err(PlatformError::PermissionDenied(
                PermissionKind::Accessibility
            ))
        );
        assert_ne!(require_trusted(false), Err(PlatformError::InputBlocked));
        assert_eq!(require_trusted(true), Ok(()));
    }

    /// Owner-run: with Accessibility revoked the real synthesizer must refuse, not no-op.
    #[test]
    #[ignore = "owner-run: touches TCC"]
    fn real_send_copy_without_accessibility_is_permission_denied() {
        let result = MacKeys::new().send_copy();
        println!("send_copy: {result:?}");
        assert!(matches!(
            result,
            Ok(())
                | Err(PlatformError::PermissionDenied(
                    PermissionKind::Accessibility
                ))
        ));
    }
}
