//! Scriptable key synthesizer wired to a `FakeClipboard`.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use lingopop_core::hotkey::Modifier;

use super::clipboard::{FakeClipboard, Mutation};
use super::log::Event;
use super::sleeper::FakeSleeper;
use crate::error::PlatformError;
use crate::keys::KeySynth;
use crate::sleeper::Sleeper;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCall {
    ReleaseModifiers(Vec<Modifier>),
    SendCopy,
    SendPaste,
    MaskMenuActivation,
}

/// What the "application" does when it receives Ctrl+C.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyBehavior {
    /// Empties the clipboard, then sets the text, immediately (two sequence bumps, like real apps).
    Select(String),
    SelectAfter(Duration, String),
    ClearThenSetLate {
        first: Duration,
        second: Duration,
        text: String,
    },
    NothingSelected,
    ImageOnly,
    /// UIPI drops the input: `send_copy` reports `InputBlocked`.
    Blocked,
}

type PasteHook = Arc<dyn Fn(&FakeClipboard) + Send + Sync>;

struct State {
    copy: CopyBehavior,
    held: Vec<Modifier>,
    release_at: Option<Duration>,
    paste_blocked: bool,
    paste_hook: Option<PasteHook>,
    pasted: Vec<Option<String>>,
}

pub struct FakeKeys {
    clip: Arc<FakeClipboard>,
    sleeper: Arc<FakeSleeper>,
    state: Mutex<State>,
}

impl FakeKeys {
    pub fn new(clip: Arc<FakeClipboard>, sleeper: Arc<FakeSleeper>) -> Arc<Self> {
        Arc::new(Self {
            clip,
            sleeper,
            state: Mutex::new(State {
                copy: CopyBehavior::NothingSelected,
                held: Vec::new(),
                release_at: None,
                paste_blocked: false,
                paste_hook: None,
                pasted: Vec::new(),
            }),
        })
    }

    pub fn on_copy(&self, behavior: CopyBehavior) {
        self.state().copy = behavior;
    }

    /// `release_after`: the user lets go after this much virtual time; `None` = held forever
    /// (until `release_modifiers`).
    pub fn hold_modifiers(&self, modifiers: Vec<Modifier>, release_after: Option<Duration>) {
        let mut s = self.state();
        s.held = modifiers;
        s.release_at = release_after.map(|d| self.sleeper.now() + d);
    }

    pub fn block_paste(&self) {
        self.state().paste_blocked = true;
    }

    /// Runs after the paste snapshot is taken, with the clipboard (to simulate a foreign write).
    pub fn on_paste(&self, hook: Box<dyn Fn(&FakeClipboard) + Send + Sync>) {
        self.state().paste_hook = Some(Arc::from(hook));
    }

    pub fn events(&self) -> Vec<KeyCall> {
        self.clip
            .log()
            .events()
            .into_iter()
            .filter_map(|e| match e.event {
                Event::Key(k) => Some(k),
                _ => None,
            })
            .collect()
    }

    /// Clipboard text at each paste.
    pub fn pasted(&self) -> Vec<Option<String>> {
        self.state().pasted.clone()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn record(&self, call: KeyCall) {
        self.clip.log().record(Event::Key(call));
    }
}

impl KeySynth for FakeKeys {
    fn modifiers_down(&self) -> Vec<Modifier> {
        let mut s = self.state();
        if let Some(at) = s.release_at {
            if self.sleeper.now() >= at {
                s.held.clear();
                s.release_at = None;
            }
        }
        s.held.clone()
    }

    fn release_modifiers(&self, modifiers: &[Modifier]) -> Result<(), PlatformError> {
        self.record(KeyCall::ReleaseModifiers(modifiers.to_vec()));
        self.state().held.retain(|m| !modifiers.contains(m));
        Ok(())
    }

    fn mask_menu_activation(&self) -> Result<(), PlatformError> {
        self.record(KeyCall::MaskMenuActivation);
        Ok(())
    }

    fn send_copy(&self) -> Result<(), PlatformError> {
        self.record(KeyCall::SendCopy);
        let behavior = self.state().copy.clone();
        let zero = Duration::ZERO;
        match behavior {
            CopyBehavior::Select(t) => {
                self.clip.schedule(zero, Mutation::Clear);
                self.clip.schedule(zero, Mutation::SetText(t));
            }
            CopyBehavior::SelectAfter(d, t) => {
                self.clip.schedule(d, Mutation::Clear);
                self.clip.schedule(d, Mutation::SetText(t));
            }
            CopyBehavior::ClearThenSetLate {
                first,
                second,
                text,
            } => {
                self.clip.schedule(first, Mutation::Clear);
                self.clip.schedule(second, Mutation::SetText(text));
            }
            CopyBehavior::NothingSelected => {}
            CopyBehavior::ImageOnly => self.clip.schedule(zero, Mutation::SetImage),
            CopyBehavior::Blocked => return Err(PlatformError::InputBlocked),
        }
        Ok(())
    }

    fn send_paste(&self) -> Result<(), PlatformError> {
        self.record(KeyCall::SendPaste);
        let (blocked, hook) = {
            let s = self.state();
            (s.paste_blocked, s.paste_hook.clone())
        };
        if blocked {
            return Err(PlatformError::InputBlocked);
        }
        let text = self.clip.text();
        self.state().pasted.push(text);
        if let Some(h) = hook {
            h(&self.clip);
        }
        Ok(())
    }
}
