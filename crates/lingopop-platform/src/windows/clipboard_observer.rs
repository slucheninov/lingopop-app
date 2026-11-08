//! Clipboard notifications are coalesced on a message thread; all clipboard data reads run on
//! the separate observer worker. No clipboard data is fetched in a window procedure.
use super::util::{map_err, wide, ClipboardGuard, MessageWindow};
use crate::{
    clipboard_observer::{
        windows_history_allowed, ClipboardObservationSource, MAX_HISTORY_TEXT_BYTES,
    },
    PlatformError, SystemSleeper,
};
use ::windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HANDLE, HGLOBAL, LPARAM, WPARAM},
        System::{
            DataExchange::{
                AddClipboardFormatListener, GetClipboardData, GetClipboardSequenceNumber,
                IsClipboardFormatAvailable, RegisterClipboardFormatW,
                RemoveClipboardFormatListener,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::CF_UNICODETEXT,
            Threading::GetCurrentThreadId,
        },
        UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, MSG, WM_APP,
            WM_CLIPBOARDUPDATE,
        },
    },
};
use crossbeam_channel::Sender;
use std::{sync::mpsc::sync_channel, thread::JoinHandle};
const QUIT: u32 = WM_APP + 73;

pub(crate) struct NotificationListener {
    thread: Option<JoinHandle<()>>,
    id: u32,
}
impl NotificationListener {
    pub(crate) fn start(events: Sender<()>) -> Result<Self, PlatformError> {
        let (ready, receive) = sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("lingopop-clipboard-notifications".into())
            .spawn(move || {
                let owner = match MessageWindow::new() {
                    Ok(w) => w,
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                // SAFETY: the window belongs to this thread and lives until removal below.
                if let Err(e) = unsafe { AddClipboardFormatListener(owner.0) } {
                    let _ = ready.send(Err(map_err("AddClipboardFormatListener")(e)));
                    return;
                }
                // SAFETY: pure thread-local query. Creating the window above created the message queue.
                let id = unsafe { GetCurrentThreadId() };
                let _ = ready.send(Ok(id));
                let mut msg = MSG::default();
                // SAFETY: valid writable MSG, pumping this thread's queue.
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
                    if msg.message == QUIT {
                        break;
                    }
                    if msg.message == WM_CLIPBOARDUPDATE {
                        let _ = events.try_send(());
                    }
                    // SAFETY: MSG was obtained from GetMessageW; window remains owned by this thread.
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                // SAFETY: removes our registration before owner destruction on this same thread.
                let _ = unsafe { RemoveClipboardFormatListener(owner.0) };
            })
            .map_err(|_| PlatformError::Unsupported("clipboard notification thread"))?;
        match receive.recv() {
            Ok(Ok(id)) => Ok(Self {
                thread: Some(thread),
                id,
            }),
            result => {
                let _ = thread.join();
                Err(result
                    .ok()
                    .and_then(Result::err)
                    .unwrap_or(PlatformError::Unsupported("clipboard notification startup")))
            }
        }
    }
}
impl Drop for NotificationListener {
    fn drop(&mut self) {
        // SAFETY: thread id was acknowledged after the message queue was created.
        let _ = unsafe { PostThreadMessageW(self.id, QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) struct WindowsObservationSource;
fn registered(name: &str) -> Result<u32, PlatformError> {
    let name = wide(name);
    // SAFETY: terminated string is alive for the call.
    let id = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
    if id == 0 {
        return Err(PlatformError::Unsupported("clipboard exclusion format"));
    }
    Ok(id)
}
fn available(id: u32) -> bool {
    // SAFETY: query of a clipboard format id; caller holds clipboard open.
    unsafe { IsClipboardFormatAvailable(id) }.is_ok()
}
fn bytes(id: u32, limit: usize, dword_prefix: bool) -> Result<Vec<u8>, PlatformError> {
    // SAFETY: caller holds clipboard open for the complete read.
    let handle: HANDLE = unsafe { GetClipboardData(id) }.map_err(map_err("GetClipboardData"))?;
    let mem = HGLOBAL(handle.0);
    // SAFETY: clipboard-owned handle remains valid while clipboard stays open.
    let size = unsafe { GlobalSize(mem) };
    if size == 0 || (!dword_prefix && size > limit) || (dword_prefix && size < limit) {
        return Err(PlatformError::Unsupported("clipboard data size"));
    }
    // SAFETY: clipboard-owned global block, valid for this open session.
    let ptr = unsafe { GlobalLock(mem) };
    if ptr.is_null() {
        return Err(PlatformError::ClipboardBusy);
    }
    let read_size = if dword_prefix { limit } else { size };
    // SAFETY: the allocation holds at least read_size bytes, checked above.
    let result = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), read_size) }.to_vec();
    // SAFETY: balances the successful lock; we do not free clipboard-owned memory.
    let _ = unsafe { GlobalUnlock(mem) };
    Ok(result)
}
impl ClipboardObservationSource for WindowsObservationSource {
    fn generation(&self) -> u64 {
        // SAFETY: pure generation query.
        u64::from(unsafe { GetClipboardSequenceNumber() })
    }
    fn read_generation(&self, expected: u64) -> Result<Option<String>, PlatformError> {
        let exclude = registered("ExcludeClipboardContentFromMonitorProcessing")?;
        let history = registered("CanIncludeInClipboardHistory")?;
        let cloud = registered("CanUploadToCloudClipboard")?;
        let _guard = ClipboardGuard::open(None, &SystemSleeper::new())?;
        if self.generation() != expected {
            return Ok(None);
        }
        if available(exclude) {
            return Ok(None);
        }
        let h = if available(history) {
            Some(bytes(history, 4, true)?)
        } else {
            None
        };
        let c = if available(cloud) {
            Some(bytes(cloud, 4, true)?)
        } else {
            None
        };
        if !windows_history_allowed(false, h.as_deref(), c.as_deref()) {
            return Ok(None);
        }
        let text_id = u32::from(CF_UNICODETEXT.0);
        if !available(text_id) {
            return Ok(None);
        }
        let data = bytes(text_id, MAX_HISTORY_TEXT_BYTES * 2 + 2, false)?;
        if !data.len().is_multiple_of(2) {
            return Ok(None);
        }
        let mut units = Vec::with_capacity(data.len() / 2);
        let mut terminated = false;
        for pair in data.as_chunks::<2>().0 {
            let u = u16::from_le_bytes([pair[0], pair[1]]);
            if u == 0 {
                terminated = true;
                break;
            }
            units.push(u);
        }
        if !terminated {
            return Ok(None);
        }
        let text = String::from_utf16(&units)
            .map_err(|_| PlatformError::Unsupported("clipboard text encoding"))?;
        if text.len() > MAX_HISTORY_TEXT_BYTES || self.generation() != expected {
            return Ok(None);
        }
        Ok(Some(text))
    }
}
