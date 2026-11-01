//! Keyboard seam: modifier state and the two synthesized chords.

use lingopop_core::hotkey::Modifier;

use crate::error::PlatformError;

pub trait KeySynth: Send + Sync {
    /// Modifiers physically held right now.
    fn modifiers_down(&self) -> Vec<Modifier>;
    /// Synthesizes key-up for the given modifiers.
    fn release_modifiers(&self, modifiers: &[Modifier]) -> Result<(), PlatformError>;
    /// If Alt or a Win key is physically down, taps the unassigned `VK_MASK` key so the later
    /// physical release is no "lone Alt / Win" and opens neither the menu bar (access-key tips)
    /// nor the Start menu. Does nothing otherwise. Call as soon as a hotkey with such a modifier
    /// fired (the hotkey's own key press was consumed by `RegisterHotKey`).
    fn mask_menu_activation(&self) -> Result<(), PlatformError>;
    fn send_copy(&self) -> Result<(), PlatformError>;
    fn send_paste(&self) -> Result<(), PlatformError>;
}

// Pure key-event planning shared by the Windows `KeySynth` (tested on every OS). Virtual-key codes
// are the Win32 `VK_*` values.

#[doc(hidden)]
pub const VK_SHIFT: u16 = 0x10;
#[doc(hidden)]
pub const VK_CONTROL: u16 = 0x11;
#[doc(hidden)]
pub const VK_MENU: u16 = 0x12;
#[doc(hidden)]
pub const VK_LWIN: u16 = 0x5B;
#[doc(hidden)]
pub const VK_RWIN: u16 = 0x5C;
#[doc(hidden)]
pub const VK_LSHIFT: u16 = 0xA0;
#[doc(hidden)]
pub const VK_RSHIFT: u16 = 0xA1;
#[doc(hidden)]
pub const VK_LCONTROL: u16 = 0xA2;
#[doc(hidden)]
pub const VK_RCONTROL: u16 = 0xA3;
#[doc(hidden)]
pub const VK_LMENU: u16 = 0xA4;
#[doc(hidden)]
pub const VK_RMENU: u16 = 0xA5;
#[doc(hidden)]
pub const VK_C: u16 = 0x43;
#[doc(hidden)]
pub const VK_V: u16 = 0x56;
/// Unassigned virtual key (0xE8 is reserved-unassigned in `winuser.h`, the same "menu mask" key
/// AutoHotkey uses): tapping it between an Alt/Win press and its release stops the release from
/// opening the window menu bar or the Start menu.
#[doc(hidden)]
pub const VK_MASK: u16 = 0xE8;

/// One synthesized key event. `extended` = the key needs `KEYEVENTF_EXTENDEDKEY` (right Ctrl,
/// right Alt, both Win keys).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VkEvent {
    pub vk: u16,
    pub up: bool,
    pub extended: bool,
}

const fn key(vk: u16, up: bool, extended: bool) -> VkEvent {
    VkEvent { vk, up, extended }
}

/// Modifier down, key down, key up, modifier up: sent in ONE `SendInput` call so no foreign event
/// can interleave.
#[doc(hidden)]
pub fn chord_events(modifier_vk: u16, key_vk: u16) -> [VkEvent; 4] {
    [
        key(modifier_vk, false, false),
        key(key_vk, false, false),
        key(key_vk, true, false),
        key(modifier_vk, true, false),
    ]
}

/// Key-up events for both sides of every given modifier (duplicates ignored). Before the first
/// Alt or Win key-up a `VK_MASK` down+up tap is emitted, so the release does not open the menu bar
/// or the Start menu.
#[doc(hidden)]
pub fn release_events(modifiers: &[Modifier]) -> Vec<VkEvent> {
    let mut seen: Vec<Modifier> = Vec::new();
    let mut masked = false;
    let mut out = Vec::new();
    for &m in modifiers {
        if seen.contains(&m) {
            continue;
        }
        seen.push(m);
        let (left, right) = match m {
            Modifier::Ctrl => (key(VK_LCONTROL, true, false), key(VK_RCONTROL, true, true)),
            Modifier::Shift => (key(VK_LSHIFT, true, false), key(VK_RSHIFT, true, false)),
            Modifier::Alt => (key(VK_LMENU, true, false), key(VK_RMENU, true, true)),
            Modifier::Win => (key(VK_LWIN, true, true), key(VK_RWIN, true, true)),
        };
        if matches!(m, Modifier::Alt | Modifier::Win) && !masked {
            out.push(key(VK_MASK, false, false));
            out.push(key(VK_MASK, true, false));
            masked = true;
        }
        out.push(left);
        out.push(right);
    }
    out
}

/// The `VK_MASK` down+up tap that makes a following Alt / Win release not "lone".
#[doc(hidden)]
pub fn menu_mask_events() -> [VkEvent; 2] {
    [key(VK_MASK, false, false), key(VK_MASK, true, false)]
}

/// `SendInput` reports how many events it inserted; fewer than requested means another thread or
/// UIPI blocked the input.
#[doc(hidden)]
pub fn ensure_inserted(requested: usize, inserted: u32) -> Result<(), PlatformError> {
    if (inserted as usize) < requested {
        Err(PlatformError::InputBlocked)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(vk: u16, up: bool, extended: bool) -> VkEvent {
        VkEvent { vk, up, extended }
    }

    #[test]
    fn chord_events_orders_mod_down_key_down_key_up_mod_up() {
        assert_eq!(
            chord_events(VK_CONTROL, VK_C),
            [
                ev(VK_CONTROL, false, false),
                ev(VK_C, false, false),
                ev(VK_C, true, false),
                ev(VK_CONTROL, true, false),
            ]
        );
    }

    #[test]
    fn menu_mask_events_tap_vk_mask_down_then_up() {
        assert_eq!(
            menu_mask_events(),
            [ev(VK_MASK, false, false), ev(VK_MASK, true, false)]
        );
    }

    #[test]
    fn chord_events_for_copy_and_paste_use_vk_c_and_vk_v() {
        assert_eq!(VK_C, 0x43);
        assert_eq!(VK_V, 0x56);
        assert_eq!(VK_CONTROL, 0x11);
        let paste = chord_events(VK_CONTROL, VK_V);
        assert_eq!(paste[1].vk, 0x56);
        assert_eq!(paste[2].vk, 0x56);
        let copy = chord_events(VK_CONTROL, VK_C);
        assert_eq!(copy[1].vk, 0x43);
    }

    #[test]
    fn release_events_for_ctrl_shift_release_both_sides() {
        assert_eq!(
            release_events(&[Modifier::Ctrl, Modifier::Shift]),
            vec![
                ev(VK_LCONTROL, true, false),
                ev(VK_RCONTROL, true, true),
                ev(VK_LSHIFT, true, false),
                ev(VK_RSHIFT, true, false),
            ]
        );
        assert_eq!(
            (VK_LCONTROL, VK_RCONTROL, VK_LSHIFT, VK_RSHIFT),
            (0xA2, 0xA3, 0xA0, 0xA1)
        );
    }

    #[test]
    fn release_events_mask_alt_and_win() {
        assert_eq!(
            release_events(&[Modifier::Shift, Modifier::Alt, Modifier::Win]),
            vec![
                ev(VK_LSHIFT, true, false),
                ev(VK_RSHIFT, true, false),
                ev(VK_MASK, false, false),
                ev(VK_MASK, true, false),
                ev(VK_LMENU, true, false),
                ev(VK_RMENU, true, true),
                ev(VK_LWIN, true, true),
                ev(VK_RWIN, true, true),
            ]
        );
        assert_eq!(
            release_events(&[Modifier::Win]),
            vec![
                ev(VK_MASK, false, false),
                ev(VK_MASK, true, false),
                ev(VK_LWIN, true, true),
                ev(VK_RWIN, true, true),
            ]
        );
        assert!(release_events(&[Modifier::Ctrl, Modifier::Shift])
            .iter()
            .all(|e| e.vk != VK_MASK));
    }

    #[test]
    fn release_events_ignores_duplicate_modifiers() {
        assert_eq!(
            release_events(&[Modifier::Ctrl, Modifier::Ctrl]),
            release_events(&[Modifier::Ctrl])
        );
    }

    #[test]
    fn release_events_empty_for_no_modifiers() {
        assert!(release_events(&[]).is_empty());
    }

    #[test]
    fn ensure_inserted_ok_when_all_events_inserted() {
        assert_eq!(ensure_inserted(4, 4), Ok(()));
        assert_eq!(ensure_inserted(0, 0), Ok(()));
    }

    #[test]
    fn ensure_inserted_fewer_events_is_input_blocked() {
        assert_eq!(ensure_inserted(4, 3), Err(PlatformError::InputBlocked));
    }

    #[test]
    fn ensure_inserted_zero_is_input_blocked() {
        assert_eq!(ensure_inserted(4, 0), Err(PlatformError::InputBlocked));
    }
}
