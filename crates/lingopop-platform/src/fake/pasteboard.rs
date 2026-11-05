//! Scriptable in-memory pasteboard backend: items, a read log, a clear counter, an unreadable
//! type, foreign writes and a failing own write.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::PlatformError;
use crate::pasteboard::{PasteboardBackend, PasteboardItem, TEXT_TYPE};

type Hook = Arc<dyn Fn(&FakePasteboard) + Send + Sync>;

struct State {
    items: Vec<PasteboardItem>,
    count: u64,
    reads: Vec<(u32, String)>,
    clears: usize,
    unreadable: HashSet<(u32, String)>,
    fail_write: bool,
    fail_enumeration: bool,
    read_hook: Option<Hook>,
}

/// Clones share the same pasteboard.
#[derive(Clone)]
pub struct FakePasteboard {
    state: Arc<Mutex<State>>,
}

impl FakePasteboard {
    pub fn new() -> Self {
        Self::with_items(Vec::new())
    }

    pub fn with_items(items: Vec<PasteboardItem>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                items,
                count: 1,
                reads: Vec::new(),
                clears: 0,
                unreadable: HashSet::new(),
                fail_write: false,
                fail_enumeration: false,
                read_hook: None,
            })),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn change_count(&self) -> u64 {
        self.state().count
    }

    pub fn items(&self) -> Vec<PasteboardItem> {
        self.state().items.clone()
    }

    /// Another application replaces the contents: the change count goes up by one.
    pub fn foreign_write(&self, items: Vec<PasteboardItem>) {
        let mut s = self.state();
        s.items = items;
        s.count += 1;
    }

    /// Another application writes without changing what is readable (only the count moves).
    pub fn foreign_touch(&self) {
        self.state().count += 1;
    }

    /// Every `(item, type)` whose data was read, in order.
    pub fn read_calls(&self) -> Vec<(u32, String)> {
        self.state().reads.clone()
    }

    pub fn read_count_of(&self, type_name: &str) -> usize {
        self.state()
            .reads
            .iter()
            .filter(|(_, t)| t == type_name)
            .count()
    }

    /// How many times `clearContents` ran.
    pub fn clear_calls(&self) -> usize {
        self.state().clears
    }

    /// The owner of this type provides no data (`dataForType:` returns nil).
    pub fn set_unreadable(&self, item: u32, type_name: &str) {
        self.state().unreadable.insert((item, type_name.to_owned()));
    }

    /// The next `write_items` fails after the pasteboard was cleared (a partial own write).
    pub fn fail_next_write(&self) {
        self.state().fail_write = true;
    }

    /// `item_types` fails like `pasteboardItems` returning nil (until `set_enumeration_failure(false)`).
    pub fn set_enumeration_failure(&self, fail: bool) {
        self.state().fail_enumeration = fail;
    }

    /// Runs `hook` at the start of every data read (to simulate a foreign write at that moment).
    pub fn on_read(&self, hook: Box<dyn Fn(&FakePasteboard) + Send + Sync>) {
        self.state().read_hook = Some(Arc::from(hook));
    }
}

impl Default for FakePasteboard {
    fn default() -> Self {
        Self::new()
    }
}

/// Never prints types or bytes.
impl fmt::Debug for FakePasteboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.state();
        write!(
            f,
            "FakePasteboard {{ items: {}, count: {} }}",
            s.items.len(),
            s.count
        )
    }
}

impl PasteboardBackend for FakePasteboard {
    fn change_count(&self) -> u64 {
        FakePasteboard::change_count(self)
    }

    fn item_types(&self) -> Result<Vec<Vec<String>>, PlatformError> {
        let s = self.state();
        if s.fail_enumeration {
            return Err(PlatformError::Unsupported(
                crate::pasteboard::UNSUPPORTED_SNAPSHOT,
            ));
        }
        Ok(s.items
            .iter()
            .map(|item| item.iter().map(|(t, _)| t.clone()).collect())
            .collect())
    }

    fn read(&self, item: u32, type_name: &str) -> Result<Option<Vec<u8>>, PlatformError> {
        let hook = {
            let mut s = self.state();
            s.reads.push((item, type_name.to_owned()));
            s.read_hook.clone()
        };
        if let Some(h) = hook {
            h(self);
        }
        let s = self.state();
        if s.unreadable.contains(&(item, type_name.to_owned())) {
            return Ok(None);
        }
        Ok(s.items
            .get(item as usize)
            .and_then(|it| it.iter().find(|(t, _)| t == type_name))
            .map(|(_, d)| d.clone()))
    }

    fn clear(&self) -> u64 {
        let mut s = self.state();
        s.items.clear();
        s.clears += 1;
        s.count += 1;
        s.count
    }

    fn write_items(&self, items: &[PasteboardItem]) -> Result<(), PlatformError> {
        let mut s = self.state();
        if std::mem::take(&mut s.fail_write) {
            return Err(PlatformError::Os {
                api: "NSPasteboard.writeObjects",
                code: 0,
            });
        }
        // Like AppKit: writing after a clear does not move the change count again.
        s.items.extend(items.iter().cloned());
        Ok(())
    }

    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        let s = self.state();
        Ok(s.items
            .iter()
            .flat_map(|it| it.iter())
            .find(|(t, _)| t == TEXT_TYPE)
            .map(|(_, d)| String::from_utf8_lossy(d).into_owned()))
    }
}
