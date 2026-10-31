//! Keyboard seam: modifier state and the two synthesized chords.

use lingopop_core::hotkey::Modifier;

use crate::error::PlatformError;

pub trait KeySynth: Send + Sync {
    /// Modifiers physically held right now.
    fn modifiers_down(&self) -> Vec<Modifier>;
    /// Synthesizes key-up for the given modifiers.
    fn release_modifiers(&self, modifiers: &[Modifier]) -> Result<(), PlatformError>;
    fn send_copy(&self) -> Result<(), PlatformError>;
    fn send_paste(&self) -> Result<(), PlatformError>;
}
