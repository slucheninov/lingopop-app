//! Scriptable in-memory clipboard.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::log::{Event, EventLog};
use super::sleeper::FakeSleeper;
use crate::clipboard::{Clipboard, ClipboardEntry, ClipboardFormat, ClipboardSnapshot};
use crate::error::PlatformError;
use crate::sleeper::Sleeper;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipboardCall {
    Snapshot,
    Restore,
    ReadText,
    WriteTransient,
    Write,
}

/// Operations whose next call `fail_next` can make fail.
pub type FakeOp = ClipboardCall;

/// A write by "another application"; each bumps the sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    SetText(String),
    Clear,
    SetImage,
}

type ReadHook = Box<dyn Fn(&FakeClipboard) + Send + Sync>;

struct State {
    entries: Vec<ClipboardEntry>,
    skipped: Vec<String>,
    seq: u64,
    scheduled: Vec<(Duration, Mutation)>,
    fail: HashMap<FakeOp, PlatformError>,
    panic_on_read: bool,
}

pub struct FakeClipboard {
    sleeper: Arc<FakeSleeper>,
    log: Arc<EventLog>,
    state: Mutex<State>,
    read_hook: Mutex<Option<Arc<ReadHook>>>,
}

fn text_entry(text: &str) -> ClipboardEntry {
    ClipboardEntry {
        format: ClipboardFormat::UnicodeText,
        data: text.as_bytes().to_vec(),
    }
}

impl FakeClipboard {
    pub fn new(sleeper: Arc<FakeSleeper>) -> Arc<Self> {
        let log = EventLog::new(sleeper.clone());
        Arc::new(Self {
            sleeper,
            log,
            state: Mutex::new(State {
                entries: Vec::new(),
                skipped: Vec::new(),
                seq: 1,
                scheduled: Vec::new(),
                fail: HashMap::new(),
                panic_on_read: false,
            }),
            read_hook: Mutex::new(None),
        })
    }

    /// The event log this clipboard records into (shared with `FakeKeys`).
    pub fn log(&self) -> Arc<EventLog> {
        self.log.clone()
    }

    pub fn with_text(self: Arc<Self>, text: &str) -> Arc<Self> {
        self.state().entries = vec![text_entry(text)];
        self
    }

    pub fn with_entries(self: Arc<Self>, entries: Vec<ClipboardEntry>) -> Arc<Self> {
        self.state().entries = entries;
        self
    }

    pub fn with_skipped(self: Arc<Self>, skipped: Vec<String>) -> Arc<Self> {
        self.state().skipped = skipped;
        self
    }

    /// Another application writes text: the sequence number goes up by one.
    pub fn set_external_text(&self, text: &str) {
        self.apply(&Mutation::SetText(text.to_owned()));
    }

    pub fn text(&self) -> Option<String> {
        self.settled().text()
    }

    pub fn entries(&self) -> Vec<ClipboardEntry> {
        self.settled().entries.clone()
    }

    pub fn sequence(&self) -> u64 {
        self.settled().seq
    }

    pub fn calls(&self) -> Vec<ClipboardCall> {
        self.log
            .events()
            .into_iter()
            .filter_map(|e| match e.event {
                Event::Clipboard(c) => Some(c),
                _ => None,
            })
            .collect()
    }

    /// The next call of `op` returns `err` (one-shot).
    pub fn fail_next(&self, op: FakeOp, err: PlatformError) {
        self.state().fail.insert(op, err);
    }

    pub fn panic_on_read_text(&self) {
        self.state().panic_on_read = true;
    }

    /// Runs `hook` at the start of every `read_text` (to simulate a foreign write at that moment).
    pub fn on_read_text(&self, hook: Box<dyn Fn(&FakeClipboard) + Send + Sync>) {
        *self.read_hook.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(hook));
    }

    /// Applies `mutation` once virtual time reaches now + `after`.
    pub fn schedule(&self, after: Duration, mutation: Mutation) {
        let due = self.sleeper.now() + after;
        self.state().scheduled.push((due, mutation));
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Locks the state after applying every mutation that is due.
    fn settled(&self) -> MutexGuard<'_, State> {
        let now = self.sleeper.now();
        let mut s = self.state();
        let mut due: Vec<(Duration, Mutation)> = Vec::new();
        s.scheduled.retain(|(at, m)| {
            if *at <= now {
                due.push((*at, m.clone()));
                false
            } else {
                true
            }
        });
        due.sort_by_key(|(at, _)| *at);
        for (_, m) in &due {
            s.mutate(m);
        }
        s
    }

    fn apply(&self, m: &Mutation) {
        self.settled().mutate(m);
    }

    fn record(&self, call: ClipboardCall) {
        self.log.record(Event::Clipboard(call));
    }

    fn take_failure(&self, op: FakeOp) -> Result<(), PlatformError> {
        match self.state().fail.remove(&op) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl State {
    fn mutate(&mut self, m: &Mutation) {
        self.entries = match m {
            Mutation::SetText(t) => vec![text_entry(t)],
            Mutation::Clear => Vec::new(),
            Mutation::SetImage => vec![ClipboardEntry {
                format: ClipboardFormat::Png,
                data: b"fake-png".to_vec(),
            }],
        };
        self.seq += 1;
    }

    fn text(&self) -> Option<String> {
        self.entries
            .iter()
            .find(|e| e.format == ClipboardFormat::UnicodeText)
            .map(|e| String::from_utf8_lossy(&e.data).into_owned())
    }
}

impl Clipboard for FakeClipboard {
    fn sequence_number(&self) -> u64 {
        self.sequence()
    }

    fn snapshot(&self) -> Result<ClipboardSnapshot, PlatformError> {
        self.record(ClipboardCall::Snapshot);
        self.take_failure(ClipboardCall::Snapshot)?;
        let s = self.settled();
        Ok(ClipboardSnapshot {
            entries: s.entries.clone(),
            skipped: s.skipped.clone(),
            sequence: s.seq,
        })
    }

    fn restore(&self, snapshot: &ClipboardSnapshot) -> Result<(), PlatformError> {
        self.record(ClipboardCall::Restore);
        self.take_failure(ClipboardCall::Restore)?;
        let mut s = self.settled();
        s.entries = snapshot.entries.clone();
        s.seq += 1;
        Ok(())
    }

    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        self.record(ClipboardCall::ReadText);
        let hook = self
            .read_hook
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(h) = hook {
            h(self);
        }
        self.take_failure(ClipboardCall::ReadText)?;
        let s = self.settled();
        if s.panic_on_read {
            drop(s);
            panic!("FakeClipboard::read_text panicked on request");
        }
        Ok(s.text())
    }

    fn write_text_transient(&self, text: &str) -> Result<(), PlatformError> {
        self.record(ClipboardCall::WriteTransient);
        if let Err(e) = self.take_failure(ClipboardCall::WriteTransient) {
            // Partial failure like Windows: EmptyClipboard succeeded, SetClipboardData failed.
            self.apply(&Mutation::Clear);
            return Err(e);
        }
        self.apply(&Mutation::SetText(text.to_owned()));
        Ok(())
    }

    fn write_text(&self, text: &str) -> Result<(), PlatformError> {
        self.record(ClipboardCall::Write);
        self.take_failure(ClipboardCall::Write)?;
        self.apply(&Mutation::SetText(text.to_owned()));
        Ok(())
    }
}
