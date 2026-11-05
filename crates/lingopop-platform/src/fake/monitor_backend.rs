//! `FakeMonitorBackend`: a recording `MonitorBackend` for the portable macOS dismiss tests.
use crate::macos_dismiss::{DismissSink, MonitorBackend, MonitorSet, ObservedEvent};
use crate::PlatformError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Default)]
struct State {
    installed: Option<(MonitorSet, Arc<DismissSink>)>,
    live: usize,
    max_live: usize,
    off_main_calls: usize,
    fail_after: Option<usize>,
}

pub struct FakeMonitorBackend {
    trusted: AtomicBool,
    state: Mutex<State>,
}

impl FakeMonitorBackend {
    pub fn new(trusted: bool) -> Self {
        Self {
            trusted: AtomicBool::new(trusted),
            state: Mutex::default(),
        }
    }
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn set_trusted(&self, trusted: bool) {
        self.trusted.store(trusted, Ordering::SeqCst);
    }
    /// Native monitors alive right now.
    pub fn native_monitors(&self) -> usize {
        self.state().live
    }
    pub fn max_native_monitors(&self) -> usize {
        self.state().max_live
    }
    pub fn installed(&self) -> Option<MonitorSet> {
        self.state().installed.as_ref().map(|(set, _)| *set)
    }
    /// True when every install / remove call ran on the (fake) main thread.
    pub fn installed_on_main_only(&self) -> bool {
        self.state().off_main_calls == 0
    }
    /// The next install creates `n` monitors, then fails.
    pub fn fail_install_after(&self, n: usize) {
        self.state().fail_after = Some(n);
    }
    /// Delivers `event` to the installed callbacks, if a monitor of its kind is installed.
    pub fn fire(&self, event: ObservedEvent) {
        let sink = {
            let s = self.state();
            match &s.installed {
                Some((set, sink)) => {
                    let wanted = match event {
                        ObservedEvent::MouseDown { .. } => set.global_mouse || set.local_mouse,
                        ObservedEvent::KeyDown { .. } => set.global_key || set.local_key,
                    };
                    wanted.then(|| sink.clone())
                }
                None => None,
            }
        };
        if let Some(sink) = sink {
            sink.observe(event);
        }
    }
    fn note_thread(&self, s: &mut State) {
        if !super::main_thread::on_fake_main() {
            s.off_main_calls += 1;
        }
    }
}

impl MonitorBackend for FakeMonitorBackend {
    fn trusted(&self) -> bool {
        self.trusted.load(Ordering::SeqCst)
    }
    fn install(&self, set: MonitorSet, sink: Arc<DismissSink>) -> Result<(), PlatformError> {
        let mut s = self.state();
        self.note_thread(&mut s);
        if let Some(n) = s.fail_after.take() {
            s.live += n;
            s.max_live = s.max_live.max(s.live);
            return Err(PlatformError::Os {
                api: "addGlobalMonitorForEventsMatchingMask",
                code: 0,
            });
        }
        s.live += set.count();
        s.max_live = s.max_live.max(s.live);
        s.installed = Some((set, sink));
        Ok(())
    }
    fn remove_all(&self) {
        let mut s = self.state();
        self.note_thread(&mut s);
        s.live = 0;
        s.installed = None;
    }
}
