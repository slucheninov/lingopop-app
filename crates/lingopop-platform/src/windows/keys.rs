//! `KeySynth` over `GetAsyncKeyState` and `SendInput`.

use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
};
use lingopop_core::hotkey::Modifier;

use crate::error::PlatformError;
use crate::keys::{
    chord_events, ensure_inserted, menu_mask_events, release_events, KeySynth, VkEvent, VK_C,
    VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_V,
};

/// Windows keyboard: reads the physical modifier state and synthesizes input with `SendInput`.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsKeys;

impl WindowsKeys {
    pub fn new() -> Self {
        Self
    }
}

fn is_down(vk: u16) -> bool {
    // SAFETY: a pure query of the asynchronous key state.
    let state = unsafe { GetAsyncKeyState(i32::from(vk)) };
    state as u16 & 0x8000 != 0
}

fn to_input(e: VkEvent) -> INPUT {
    // Every event carries its scan code (apps and remote sessions that read scan codes see a
    // real key); the virtual key stays authoritative (no KEYEVENTF_SCANCODE).
    // SAFETY: a pure table lookup.
    let scan = unsafe { MapVirtualKeyW(u32::from(e.vk), MAPVK_VK_TO_VSC) } as u16;
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if e.extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if e.up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(e.vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Sends all events in one `SendInput` call; fewer inserted than requested is `InputBlocked`.
fn send(events: &[VkEvent]) -> Result<(), PlatformError> {
    if events.is_empty() {
        return Ok(());
    }
    let inputs: Vec<INPUT> = events.iter().copied().map(to_input).collect();
    // SAFETY: `inputs` is a valid slice of keyboard INPUTs and the size is that of one INPUT.
    let inserted = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    ensure_inserted(inputs.len(), inserted)
}

impl KeySynth for WindowsKeys {
    fn modifiers_down(&self) -> Vec<Modifier> {
        let mut held = Vec::new();
        if is_down(VK_CONTROL) {
            held.push(Modifier::Ctrl);
        }
        if is_down(VK_MENU) {
            held.push(Modifier::Alt);
        }
        if is_down(VK_SHIFT) {
            held.push(Modifier::Shift);
        }
        if is_down(VK_LWIN) || is_down(VK_RWIN) {
            held.push(Modifier::Win);
        }
        held
    }

    fn release_modifiers(&self, modifiers: &[Modifier]) -> Result<(), PlatformError> {
        send(&release_events(modifiers))
    }

    fn mask_menu_activation(&self) -> Result<(), PlatformError> {
        if is_down(VK_MENU) || is_down(VK_LWIN) || is_down(VK_RWIN) {
            send(&menu_mask_events())
        } else {
            Ok(())
        }
    }

    fn send_copy(&self) -> Result<(), PlatformError> {
        send(&chord_events(VK_CONTROL, VK_C))
    }

    fn send_paste(&self) -> Result<(), PlatformError> {
        send(&chord_events(VK_CONTROL, VK_V))
    }
}
