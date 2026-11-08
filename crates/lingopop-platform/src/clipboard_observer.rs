//! Passive text observation. Reads never modify the clipboard. Native reads may invoke a delayed
//! provider: byte limits bound our allocations, not time spent inside another application's API.
use crate::PlatformError;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::{fmt, sync::Arc, thread::JoinHandle, time::Duration};

pub const MAX_HISTORY_TEXT_BYTES: usize = 64 * 1024;
const INTERVAL: Duration = Duration::from_millis(250);

pub struct ClipboardObservation {
    pub generation: u64,
    /// Coordinator revision captured before reading native content.
    pub revision: u64,
    pub text: String,
}
impl fmt::Debug for ClipboardObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClipboardObservation")
            .field("generation", &self.generation)
            .field("bytes", &self.text.len())
            .finish()
    }
}
/// Implementations must check all exclusion metadata BEFORE requesting text, fail closed on
/// unreadable metadata, bound reads, and return only text belonging to `expected`.
pub trait ClipboardObservationSource: Send + Sync + 'static {
    fn generation(&self) -> u64;
    fn read_generation(&self, expected: u64) -> Result<Option<String>, PlatformError>;
}
/// Deterministic change/deduplication state, shared by the notification and polling workers.
/// Startup establishes a baseline; existing contents are never captured merely on launch.
pub struct ObservationCursor {
    last: u64,
}
impl ObservationCursor {
    pub fn new(source: &dyn ClipboardObservationSource) -> Self {
        Self {
            last: source.generation(),
        }
    }
    pub fn observe(
        &mut self,
        source: &dyn ClipboardObservationSource,
    ) -> Result<Option<ClipboardObservation>, PlatformError> {
        let generation = source.generation();
        if generation == self.last {
            return Ok(None);
        }
        let text = match source.read_generation(generation) {
            Ok(text) => text,
            Err(PlatformError::Unsupported("clipboard data size" | "clipboard text encoding")) => {
                if source.generation() == generation {
                    self.last = generation;
                }
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        // A race does not advance the cursor; the next notification/tick tries the current generation.
        if source.generation() != generation {
            return Ok(None);
        }
        self.last = generation;
        Ok(text
            .filter(|s| !s.is_empty() && s.len() <= MAX_HISTORY_TEXT_BYTES)
            .map(|text| ClipboardObservation {
                generation,
                revision: 0,
                text,
            }))
    }
}
/// Callback runs on the dedicated reader worker, serially, after validation. It should commit
/// directly (or attach its own revision fence before enqueueing work). Clipboard generations
/// wrap and cannot serve as history delete/clear revisions.
pub type ObservationCallback = dyn Fn(ClipboardObservation) + Send + Sync + 'static;
/// False asks the worker to retry the generation after a coordinator revision changed.
pub type FencedObservationCallback = dyn Fn(ClipboardObservation) -> bool + Send + Sync + 'static;

/// Owned observer lifecycle. `stop` signals both threads and joins them: after it returns no
/// callback is running (except when called from the callback itself: that worker exits after the
/// callback returns, without attempting to join itself). Native delayed providers
/// can stall a read, and hence shutdown; no time-bounded native read guarantee is made.
pub struct ClipboardObserver {
    stop: Sender<()>,
    worker: Option<JoinHandle<()>>,
    #[cfg(windows)]
    listener: Option<crate::windows::clipboard_observer::NotificationListener>,
}
impl ClipboardObserver {
    /// Poll a generation counter. Only changed generations request content; useful for macOS and
    /// deterministic injected sources. Read failures retry on the next tick.
    pub fn start(
        source: Arc<dyn ClipboardObservationSource>,
        callback: Arc<ObservationCallback>,
    ) -> Result<Self, PlatformError> {
        Self::with_notifications(source, callback, None)
    }
    pub(crate) fn with_notifications(
        source: Arc<dyn ClipboardObservationSource>,
        callback: Arc<ObservationCallback>,
        changes: Option<Receiver<()>>,
    ) -> Result<Self, PlatformError> {
        Self::with_fence(
            source,
            Arc::new(move |item| {
                callback(item);
                true
            }),
            changes,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
        )
    }
    fn with_fence(
        source: Arc<dyn ClipboardObservationSource>,
        callback: Arc<FencedObservationCallback>,
        changes: Option<Receiver<()>>,
        revision: Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<Self, PlatformError> {
        let mut cursor = ObservationCursor::new(source.as_ref());
        let (stop, stopped) = bounded(1);
        let worker = std::thread::Builder::new()
            .name("lingopop-clipboard-reader".into())
            .spawn(move || {
                let ticks = crossbeam_channel::tick(INTERVAL);
                let events = changes.unwrap_or_else(crossbeam_channel::never);
                loop {
                    crossbeam_channel::select! {
                        recv(stopped) -> _ => break,
                        recv(ticks) -> _ => {},
                        recv(events) -> result => { if result.is_err() { break; } }
                    }
                    if !stopped.is_empty() {
                        break;
                    }
                    let previous_generation = cursor.last;
                    let read_revision = revision.load(std::sync::atomic::Ordering::Acquire);
                    if let Ok(Some(mut observation)) = cursor.observe(source.as_ref()) {
                        observation.revision = read_revision;
                        if stopped.is_empty() && !callback(observation) {
                            cursor.last = previous_generation;
                        }
                    }
                }
            })
            .map_err(|_| PlatformError::Unsupported("clipboard observer thread"))?;
        Ok(Self {
            stop,
            worker: Some(worker),
            #[cfg(windows)]
            listener: None,
        })
    }
    pub fn stop(&mut self) {
        let _ = self.stop.try_send(());
        #[cfg(windows)]
        {
            self.listener.take();
        }
        if let Some(worker) = self.worker.take() {
            if worker.thread().id() != std::thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}
impl Drop for ClipboardObserver {
    fn drop(&mut self) {
        self.stop()
    }
}

#[cfg(target_os = "macos")]
pub fn native_clipboard_observer(
    callback: Arc<ObservationCallback>,
) -> Result<ClipboardObserver, PlatformError> {
    ClipboardObserver::start(
        Arc::new(crate::macos::clipboard_observer::MacObservationSource),
        callback,
    )
}
#[cfg(windows)]
pub fn native_clipboard_observer(
    callback: Arc<ObservationCallback>,
) -> Result<ClipboardObserver, PlatformError> {
    // Establish the baseline before registering, so a copy during registration is observed.
    let source = Arc::new(crate::windows::clipboard_observer::WindowsObservationSource);
    let (send, recv) = bounded(1);
    let mut observer = ClipboardObserver::with_notifications(source, callback, Some(recv))?;
    observer.listener =
        Some(crate::windows::clipboard_observer::NotificationListener::start(send)?);
    Ok(observer)
}

/// Known privacy/temporary pasteboard types, matched by presence on any item.
pub fn macos_exclusion_marker(name: &str) -> bool {
    matches!(
        name,
        "org.nspasteboard.TransientType"
            | "org.nspasteboard.ConcealedType"
            | "org.nspasteboard.AutoGeneratedType"
            | "com.agilebits.onepassword"
    )
}
/// Windows exclusion is presence-only for the monitor marker; DWORD flags must be readable,
/// exactly four bytes, and nonzero. Unknown or malformed flags fail closed.
pub fn windows_history_allowed(
    exclude: bool,
    history: Option<&[u8]>,
    cloud: Option<&[u8]>,
) -> bool {
    !exclude
        && [history, cloud].into_iter().all(|flag| {
            flag.is_none_or(|b| <[u8; 4]>::try_from(b).is_ok_and(|a| u32::from_le_bytes(a) != 0))
        })
}

/// Performs a marker-first, bounded read only if `expected` still names the current generation.
/// Like observer callbacks, a returned generation needs a coordinator fence before persistence.
#[cfg(any(windows, target_os = "macos"))]
pub fn read_native_clipboard_generation(expected: u64) -> Result<Option<String>, PlatformError> {
    #[cfg(windows)]
    let source = crate::windows::clipboard_observer::WindowsObservationSource;
    #[cfg(target_os = "macos")]
    let source = crate::macos::clipboard_observer::MacObservationSource;
    source.read_generation(expected)
}

/// The generation counter only; never requests clipboard contents.
#[cfg(any(windows, target_os = "macos"))]
pub fn native_clipboard_generation() -> u64 {
    #[cfg(windows)]
    let source = crate::windows::clipboard_observer::WindowsObservationSource;
    #[cfg(target_os = "macos")]
    let source = crate::macos::clipboard_observer::MacObservationSource;
    source.generation()
}
/// Captures the history revision before each native read so destructive edits can reject it.
#[cfg(any(windows, target_os = "macos"))]
pub fn native_clipboard_observer_fenced(
    callback: Arc<FencedObservationCallback>,
    revision: Arc<std::sync::atomic::AtomicU64>,
) -> Result<ClipboardObserver, PlatformError> {
    #[cfg(target_os = "macos")]
    {
        ClipboardObserver::with_fence(
            Arc::new(crate::macos::clipboard_observer::MacObservationSource),
            callback,
            None,
            revision,
        )
    }
    #[cfg(windows)]
    {
        let (send, recv) = bounded(1);
        let mut observer = ClipboardObserver::with_fence(
            Arc::new(crate::windows::clipboard_observer::WindowsObservationSource),
            callback,
            Some(recv),
            revision,
        )?;
        observer.listener =
            Some(crate::windows::clipboard_observer::NotificationListener::start(send)?);
        Ok(observer)
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    };
    struct Source(AtomicU64);
    impl ClipboardObservationSource for Source {
        fn generation(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
        fn read_generation(&self, _: u64) -> Result<Option<String>, PlatformError> {
            Ok(Some("copy".into()))
        }
    }
    #[test]
    fn notification_drives_one_callback_and_stop_is_idempotent() {
        let source = Arc::new(Source(AtomicU64::new(1)));
        let (notify, events) = bounded(1);
        let (sent, received) = bounded(1);
        let mut observer = ClipboardObserver::with_notifications(
            source.clone(),
            Arc::new(move |item| {
                sent.send(item.generation).unwrap();
            }),
            Some(events),
        )
        .unwrap();
        source.0.store(2, Ordering::SeqCst);
        notify.send(()).unwrap();
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        observer.stop();
        observer.stop();
        source.0.store(3, Ordering::SeqCst);
        let _ = notify.try_send(());
        assert!(received.try_recv().is_err());
    }
    #[test]
    fn coordinator_rejection_retries_a_foreign_generation_with_a_fresh_revision() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let source = Arc::new(Source(AtomicU64::new(1)));
        let revision = Arc::new(AtomicU64::new(0));
        let (notify, events) = bounded(1);
        let (sent, received) = bounded(1);
        let (release, barrier) = bounded(1);
        let calls = Arc::new(AtomicU64::new(0));
        let callback_calls = calls.clone();
        let mut observer = ClipboardObserver::with_fence(
            source.clone(),
            Arc::new(move |item| {
                sent.send((item.generation, item.revision)).unwrap();
                if callback_calls.fetch_add(1, Ordering::AcqRel) == 0 {
                    barrier.recv().unwrap();
                    false
                } else {
                    true
                }
            }),
            Some(events),
            revision.clone(),
        )
        .unwrap();
        source.0.store(2, Ordering::Release);
        notify.send(()).unwrap();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(5)).unwrap(),
            (2, 0)
        );
        revision.store(2, Ordering::Release);
        // The generation has not changed: a rejected callback must not consume it.
        release.send(()).unwrap();
        notify.send(()).unwrap();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(5)).unwrap(),
            (2, 2)
        );
        observer.stop();
        assert_eq!(calls.load(Ordering::Acquire), 2);
    }
    #[test]
    fn callback_can_stop_its_own_worker_without_joining_itself() {
        let source = Arc::new(Source(AtomicU64::new(1)));
        let holder: Arc<Mutex<Option<ClipboardObserver>>> = Arc::new(Mutex::new(None));
        let stop_holder = holder.clone();
        let (notify, events) = bounded(1);
        let (sent, received) = bounded(1);
        let observer = ClipboardObserver::with_notifications(
            source.clone(),
            Arc::new(move |_| {
                stop_holder.lock().unwrap().as_mut().unwrap().stop();
                sent.send(()).unwrap();
            }),
            Some(events),
        )
        .unwrap();
        *holder.lock().unwrap() = Some(observer);
        source.0.store(2, Ordering::SeqCst);
        notify.send(()).unwrap();
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        holder.lock().unwrap().take();
    }
}
