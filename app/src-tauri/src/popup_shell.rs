//! `ShellPopup`: the orchestrator's `PopupView` over the hidden `popup` Tauri window.
//!
//! Commands are consumed on one dedicated worker, never the main thread. HWND is cached
//! during setup. Windows uses only NoActivateWindow; Mac keeps the Tauri dev fallback.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use lingopop_orchestrator::{presentation::*, PopupView, PopupViewModel};
use lingopop_platform::{monitor_at, physical_size, Platform, Point, Rect, WindowHandle};
use std::sync::{Arc, OnceLock};
#[cfg(not(windows))]
use tauri::Manager;
use tauri::{AppHandle, Emitter, WebviewWindow};

pub const POPUP_LABEL: &str = "popup";
pub const MODEL_EVENT: &str = "popup://model";

/// The newest model, kept so `ready` can re-emit one that arrived before the webview listened.
struct Pending {
    last_model: Option<serde_json::Value>,
}

struct NativePopup {
    app: AppHandle,
    platform: Platform,
    /// Native handle cached at construction (see the module docs); `None` = unavailable.
    handle: Option<WindowHandle>,
    visible: AtomicBool,
    pending: Mutex<Pending>,
}

/// Reads the native handle. On Windows this blocks on the main thread: call it only from
/// `setup` (main thread), never from `PopupView` methods.
#[cfg(windows)]
pub fn native_handle(window: &WebviewWindow) -> Option<WindowHandle> {
    window
        .hwnd()
        .ok()
        .map(|hwnd| WindowHandle(hwnd.0 as usize as u64))
}

#[cfg(not(windows))]
pub fn native_handle(_window: &WebviewWindow) -> Option<WindowHandle> {
    Some(WindowHandle(0))
}

impl NativePopup {
    pub fn new(app: AppHandle, platform: Platform, handle: Option<WindowHandle>) -> Self {
        Self {
            app,
            platform,
            handle,
            visible: AtomicBool::new(false),
            pending: Mutex::new(Pending { last_model: None }),
        }
    }

    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(not(windows))]
    fn window(&self) -> Option<WebviewWindow> {
        let window = self.app.get_webview_window(POPUP_LABEL);
        if window.is_none() {
            log::warn!("popup window is missing");
        }
        window
    }

    fn rect_for(&self, origin: Point, logical: (f64, f64)) -> Rect {
        let monitors = self.platform.monitors.monitors().unwrap_or_default();
        let scale = monitor_at(origin, &monitors).map_or(1.0, |m| m.scale_factor);
        let size = physical_size(logical.0, logical.1, scale);
        Rect::new(origin.x, origin.y, size.width, size.height)
    }

    fn emit_model(&self, value: &serde_json::Value) {
        if self.app.emit_to(POPUP_LABEL, MODEL_EVENT, value).is_err() {
            log::warn!("popup model event failed");
        }
    }

    /// Shows or moves the window to `rect` without activating it.
    fn place(&self, rect: Rect) -> Result<(), lingopop_platform::PlatformError> {
        let Some(handle) = self.handle else {
            log::warn!("popup window handle is unavailable");
            return Err(lingopop_platform::PlatformError::Unsupported(
                "popup window",
            ));
        };
        match self.platform.popup_window.show_at(handle, rect) {
            Ok(()) => Ok(()),
            // macOS dev only: the platform has no no-activate show; Tauri `show()` may activate.
            #[cfg(not(windows))]
            Err(lingopop_platform::PlatformError::Unsupported(_)) => {
                let Some(window) = self.window() else {
                    return Err(lingopop_platform::PlatformError::Unsupported(
                        "popup window",
                    ));
                };
                // tao converts LogicalPosition directly from top-left Y-down points to
                // AppKit. PhysicalPosition would divide Quartz points by the backing scale.
                #[cfg(target_os = "macos")]
                let placed = window
                    .set_position(tauri::LogicalPosition::new(
                        f64::from(rect.x),
                        f64::from(rect.y),
                    ))
                    .and_then(|()| {
                        window.set_size(tauri::LogicalSize::new(
                            f64::from(rect.width.max(1)),
                            f64::from(rect.height.max(1)),
                        ))
                    });
                // The enclosing arm excludes Windows, so this is Linux-only (a non-target).
                #[cfg(not(target_os = "macos"))]
                let placed = window
                    .set_position(tauri::PhysicalPosition::new(rect.x, rect.y))
                    .and_then(|()| {
                        window.set_size(tauri::PhysicalSize::new(
                            rect.width.max(1) as u32,
                            rect.height.max(1) as u32,
                        ))
                    });
                placed
                    .and_then(|()| window.show())
                    .map_err(|_| lingopop_platform::PlatformError::Unsupported("popup placement"))
            }
            Err(e) => {
                log::warn!("popup show failed: {e}");
                Err(e)
            }
        }
    }

    /// The frontend mounted and listens: re-emits a model that arrived before that.
    ///
    /// The worker serializes ready with model updates; no cache lock crosses emit.
    pub fn ready(&self) {
        let model = self.pending().last_model.clone();
        if let Some(model) = model {
            self.emit_model(&model);
        }
    }
}

impl NativePopup {
    fn show_at(
        &self,
        origin: Point,
        size_logical: (f64, f64),
        model: &PopupViewModel,
    ) -> Result<(), lingopop_platform::PlatformError> {
        if let Ok(value) = serde_json::to_value(model) {
            self.pending().last_model = Some(value.clone());
            self.emit_model(&value);
        }
        let shown = self.place(self.rect_for(origin, size_logical));
        self.visible.store(shown.is_ok(), Ordering::Release);
        shown
    }

    fn update(&self, model: &PopupViewModel) {
        let Ok(value) = serde_json::to_value(model) else {
            return;
        };
        self.pending().last_model = Some(value.clone());
        self.emit_model(&value);
    }

    fn hide(&self) {
        let _ = self.hide_checked();
    }
    fn hide_checked(&self) -> Result<(), lingopop_platform::PlatformError> {
        let handle = self
            .handle
            .ok_or(lingopop_platform::PlatformError::Unsupported(
                "popup handle",
            ))?;
        let result = match self.platform.popup_window.hide(handle) {
            Ok(()) => Ok(()),
            #[cfg(not(windows))]
            Err(lingopop_platform::PlatformError::Unsupported(_)) => {
                let window = self
                    .window()
                    .ok_or(lingopop_platform::PlatformError::Unsupported(
                        "popup window",
                    ))?;
                window
                    .hide()
                    .map_err(|_| lingopop_platform::PlatformError::Unsupported("popup hide"))
            }
            Err(e) => Err(e),
        };
        if result.is_ok() {
            self.visible.store(false, Ordering::Release);
        }
        result
    }

    fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Acquire)
    }
}

enum WorkerMessage {
    Drain,
    Ready,
    Shutdown(tokio::sync::oneshot::Sender<()>),
}
/// Queue owner. Dropping it requests worker shutdown without waiting on the UI thread.
pub struct ShellPopup {
    native: Arc<NativePopup>,
    queue: Arc<PresentationQueue>,
    sender: std::sync::mpsc::Sender<WorkerMessage>,
    receiver: Mutex<Option<std::sync::mpsc::Receiver<WorkerMessage>>>,
    sink: OnceLock<Arc<AckSink>>,
}
impl ShellPopup {
    pub fn new(app: AppHandle, platform: Platform, handle: Option<WindowHandle>) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        Self {
            native: Arc::new(NativePopup::new(app, platform, handle)),
            queue: Arc::default(),
            sender,
            receiver: Mutex::new(Some(receiver)),
            sink: OnceLock::new(),
        }
    }
    pub fn ready(&self) {
        let _ = self.sender.send(WorkerMessage::Ready);
    }
    /// Explicit close rejects further submissions immediately. Completion means the worker
    /// has finished its active command and resolved all pending acknowledgments.
    #[allow(dead_code)]
    pub fn shutdown(&self) -> tokio::sync::oneshot::Receiver<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.queue.seal();
        let _ = self.sender.send(WorkerMessage::Shutdown(tx));
        rx
    }
}
impl Drop for ShellPopup {
    fn drop(&mut self) {
        drop(self.shutdown());
    }
}
impl PopupView for ShellPopup {
    fn submit(&self, cmd: PresentationCommand) -> Result<AckReceiver, PresentationQueueError> {
        self.queue
            .submit_notified(cmd, || self.sender.send(WorkerMessage::Drain).is_ok())
    }

    fn bind(&self, fence: Arc<PresentationFence>, sink: Arc<AckSink>) {
        if self.sink.set(sink.clone()).is_err() {
            log::warn!("presentation worker already bound");
            return;
        }
        let receiver = match self.receiver.lock() {
            Ok(mut receiver) => receiver.take(),
            Err(_) => {
                log::warn!("presentation worker receiver lock failed");
                self.queue.close(sink.as_ref());
                return;
            }
        };
        let Some(receiver) = receiver else {
            log::warn!("presentation worker receiver unavailable");
            self.queue.close(sink.as_ref());
            return;
        };
        let queue = self.queue.clone();
        let native = self.native.clone();
        let failed_start_sink = sink.clone();
        let start = std::thread::Builder::new()
            .name("lingopop-presentation".into())
            .spawn(move || {
                let mut desired = DesiredPresentation::default();
                while let Ok(message) = receiver.recv() {
                    match message {
                        WorkerMessage::Ready => native.ready(),
                        WorkerMessage::Shutdown(done) => {
                            queue.close(sink.as_ref());
                            let _ = done.send(());
                            return;
                        }
                        WorkerMessage::Drain => {}
                    }
                    queue.complete_superseded(sink.as_ref());
                    while let Some(job) = queue.try_next() {
                        // Retain desired geometry/content even when a pending Show was fenced
                        // before execution; its current Resize must still be able to show it.
                        desired.record(&job.command);
                        consume_job(&fence, job, sink.as_ref(), |command| {
                            match &command.kind {
                                PresentationKind::Show {
                                    origin,
                                    size_logical,
                                    model,
                                } => {
                                    if let Err(error) =
                                        native.show_at(*origin, *size_logical, model)
                                    {
                                        native.hide();
                                        return Err(error);
                                    }
                                }
                                PresentationKind::Update { model } => native.update(model),
                                PresentationKind::Resize {
                                    origin,
                                    size_logical,
                                } => {
                                    let placed = desired.resize_with(|_, _, model| {
                                        if native.is_visible() {
                                            native.place(native.rect_for(*origin, *size_logical))
                                        } else {
                                            native.show_at(*origin, *size_logical, model)
                                        }
                                    });
                                    if let Err(error) = placed {
                                        native.hide();
                                        return Err(error);
                                    }
                                }
                                PresentationKind::Hide => {
                                    native.hide_checked()?;
                                }
                            }
                            Ok(if native.is_visible() {
                                PresentationVisibility::Visible
                            } else {
                                PresentationVisibility::Hidden
                            })
                        });
                        queue.complete_superseded(sink.as_ref());
                    }
                }
                queue.close(sink.as_ref());
            });
        finish_worker_start(start, self.queue.as_ref(), failed_start_sink.as_ref());
    }
}

fn finish_worker_start(
    start: std::io::Result<std::thread::JoinHandle<()>>,
    queue: &PresentationQueue,
    sink: &AckSink,
) {
    if start.is_err() {
        log::warn!("presentation worker failed to start");
        queue.close(sink);
    }
}

#[derive(Default)]
struct DesiredPresentation {
    sequence: u64,
    show: Option<(Point, (f64, f64), PopupViewModel)>,
}
impl DesiredPresentation {
    fn record(&mut self, command: &PresentationCommand) {
        if command.id.sequence <= self.sequence {
            return;
        }
        self.sequence = command.id.sequence;
        match &command.kind {
            PresentationKind::Show {
                origin,
                size_logical,
                model,
            } => self.show = Some((*origin, *size_logical, model.clone())),
            PresentationKind::Update { model } => {
                if let Some((_, _, current)) = self.show.as_mut() {
                    *current = model.clone();
                }
            }
            PresentationKind::Resize {
                origin,
                size_logical,
            } => {
                if let Some((p, size, _)) = self.show.as_mut() {
                    *p = *origin;
                    *size = *size_logical;
                }
            }
            PresentationKind::Hide => self.show = None,
        }
    }
    fn resize_with(
        &self,
        effect: impl FnOnce(
            Point,
            (f64, f64),
            &PopupViewModel,
        ) -> Result<(), lingopop_platform::PlatformError>,
    ) -> Result<(), lingopop_platform::PlatformError> {
        if let Some((origin, size, model)) = &self.show {
            effect(*origin, *size, model)
        } else {
            Ok(())
        }
    }
}

fn consume_job(
    fence: &PresentationFence,
    job: PresentationJob,
    sink: &AckSink,
    effect: impl FnOnce(
        &PresentationCommand,
    ) -> Result<PresentationVisibility, lingopop_platform::PlatformError>,
) {
    let ack = fence
        .apply_if_current(job.command.id, || effect(&job.command))
        .unwrap();
    job.complete(ack, sink);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn worker_effect_and_ack_run_off_submitter_and_sink_precedes_oneshot() {
        let queue = Arc::new(PresentationQueue::default());
        let fence = Arc::new(PresentationFence::default());
        let id = PresentationId {
            generation: 1,
            sequence: 1,
        };
        fence.advance(id);
        let submitter = std::thread::current().id();
        let receiver = queue
            .submit(PresentationCommand {
                id,
                kind: PresentationKind::Hide,
            })
            .unwrap();
        let acked = Arc::new(AtomicBool::new(false));
        let seen = acked.clone();
        let (release, blocked) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            blocked.recv().unwrap();
            let job = queue.try_next().unwrap();
            consume_job(
                &fence,
                job,
                &move |_| {
                    seen.store(true, Ordering::SeqCst);
                },
                |_| {
                    assert_ne!(std::thread::current().id(), submitter);
                    Ok(PresentationVisibility::Hidden)
                },
            );
            queue.close(&|_| {});
        });
        assert!(!acked.load(Ordering::SeqCst));
        release.send(()).unwrap();
        assert_eq!(
            receiver.await.unwrap().result,
            Ok(PresentationVisibility::Hidden)
        );
        assert!(acked.load(Ordering::SeqCst));
        worker.join().unwrap();
    }
}

#[cfg(test)]
mod fix_round_one_tests {
    use super::*;
    #[test]
    fn hide_clears_resize_intent_so_no_native_effect_runs() {
        let mut desired = DesiredPresentation::default();
        let id = |sequence| PresentationId {
            generation: 1,
            sequence,
        };
        desired.record(&PresentationCommand {
            id: id(1),
            kind: PresentationKind::Show {
                origin: Point { x: 0, y: 0 },
                size_logical: (400., 200.),
                model: PopupViewModel::loading(
                    lingopop_core::operation::AIOperation::Translate,
                    "fixture",
                ),
            },
        });
        desired.record(&PresentationCommand {
            id: id(2),
            kind: PresentationKind::Hide,
        });
        desired.record(&PresentationCommand {
            id: id(3),
            kind: PresentationKind::Resize {
                origin: Point { x: 10, y: 10 },
                size_logical: (500., 300.),
            },
        });
        let mut effects = 0;
        desired
            .resize_with(|_, _, _| {
                effects += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(effects, 0);
    }
    #[tokio::test]
    async fn worker_start_failure_closes_and_resolves_accepted_work() {
        let queue = PresentationQueue::default();
        let id = PresentationId {
            generation: 1,
            sequence: 1,
        };
        let receiver = queue
            .submit(PresentationCommand {
                id,
                kind: PresentationKind::Hide,
            })
            .unwrap();
        finish_worker_start(Err(std::io::Error::other("fixture")), &queue, &|_| {});
        assert_eq!(
            receiver.await.unwrap().result,
            Ok(PresentationVisibility::Superseded)
        );
        assert_eq!(
            queue
                .submit(PresentationCommand {
                    id,
                    kind: PresentationKind::Hide
                })
                .unwrap_err(),
            PresentationQueueError::Closed
        );
    }
}
