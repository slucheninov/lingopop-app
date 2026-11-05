//! Portable queued presentation. Consumers execute effects without any queue or session lock.
use crate::PopupViewModel;
use lingopop_platform::{PlatformError, Point};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Condvar, Mutex,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresentationId {
    pub generation: u64,
    pub sequence: u64,
}
#[derive(Clone)]
pub enum PresentationKind {
    Show {
        origin: Point,
        size_logical: (f64, f64),
        model: PopupViewModel,
    },
    Update {
        model: PopupViewModel,
    },
    Resize {
        origin: Point,
        size_logical: (f64, f64),
    },
    Hide,
}
#[derive(Clone)]
pub struct PresentationCommand {
    pub id: PresentationId,
    pub kind: PresentationKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationVisibility {
    Visible,
    Hidden,
    Superseded,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationAck {
    /// True only when an executed effect returned Visible, including after supersession.
    pub effect_left_visible: bool,
    pub id: PresentationId,
    pub result: Result<PresentationVisibility, PlatformError>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationQueueError {
    Full,
    Closed,
}
pub type AckReceiver = tokio::sync::oneshot::Receiver<PresentationAck>;
pub type AckSink = dyn Fn(PresentationAck) + Send + Sync;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Hidden,
    Pending(PresentationId),
    Visible(PresentationId),
}

/// A sequence is globally increasing. Publishing sequence last prevents a reader accepting
/// a partially published generation/sequence pair. Only the orchestrator commit writes it.
#[derive(Default)]
pub struct PresentationFence {
    generation: AtomicU64,
    sequence: AtomicU64,
}
impl PresentationFence {
    pub fn advance(&self, id: PresentationId) {
        self.generation.store(id.generation, Ordering::SeqCst);
        self.sequence.store(id.sequence, Ordering::SeqCst);
    }
    pub fn is_current(&self, id: PresentationId) -> bool {
        self.sequence.load(Ordering::SeqCst) == id.sequence
            && self.generation.load(Ordering::SeqCst) == id.generation
            && self.sequence.load(Ordering::SeqCst) == id.sequence
    }
    pub fn apply_if_current(
        &self,
        id: PresentationId,
        effect: impl FnOnce() -> Result<PresentationVisibility, PlatformError>,
    ) -> Option<PresentationAck> {
        let mut effect_left_visible = false;
        let result = if self.is_current(id) {
            let result = effect();
            effect_left_visible = result == Ok(PresentationVisibility::Visible);
            if self.is_current(id) {
                result
            } else {
                Ok(PresentationVisibility::Superseded)
            }
        } else {
            Ok(PresentationVisibility::Superseded)
        };
        Some(PresentationAck {
            id,
            result,
            effect_left_visible,
        })
    }
}
pub struct PresentationJob {
    pub command: PresentationCommand,
    sender: tokio::sync::oneshot::Sender<PresentationAck>,
}
impl PresentationJob {
    pub fn complete(self, ack: PresentationAck, sink: &AckSink) {
        sink(ack.clone());
        let _ = self.sender.send(ack);
    }
    fn supersede(self, sink: &AckSink) {
        let id = self.command.id;
        self.complete(
            PresentationAck {
                id,
                result: Ok(PresentationVisibility::Superseded),
                effect_left_visible: false,
            },
            sink,
        );
    }
}
#[derive(Default)]
struct QueueState {
    pending: VecDeque<PresentationJob>,
    superseded: Vec<PresentationJob>,
    closed: bool,
}
#[derive(Default)]
pub struct PresentationQueue {
    state: Mutex<QueueState>,
    wake: Condvar,
}
impl PresentationQueue {
    pub fn submit(
        &self,
        command: PresentationCommand,
    ) -> Result<AckReceiver, PresentationQueueError> {
        self.submit_notified(command, || true)
    }

    /// Wake a worker before acceptance, atomically with enqueue. The notifier must only
    /// send a signal: it must never consume, dispatch native work or call the ack sink.
    pub fn submit_notified(
        &self,
        command: PresentationCommand,
        notify: impl FnOnce() -> bool,
    ) -> Result<AckReceiver, PresentationQueueError> {
        let mut s = self.state.lock().unwrap();
        if s.closed {
            return Err(PresentationQueueError::Closed);
        }
        let merges_show = s.pending.back().is_some_and(|old| {
            old.command.id.generation == command.id.generation
                && old.command.id.sequence < command.id.sequence
                && matches!(old.command.kind, PresentationKind::Show { .. })
                && matches!(
                    command.kind,
                    PresentationKind::Update { .. } | PresentationKind::Resize { .. }
                )
        });
        if !matches!(command.kind, PresentationKind::Hide)
            && !merges_show
            && s.pending
                .iter()
                .filter(|j| !matches!(j.command.kind, PresentationKind::Hide))
                .count()
                >= 32
        {
            return Err(PresentationQueueError::Full);
        }
        if !notify() {
            s.closed = true;
            self.wake.notify_all();
            return Err(PresentationQueueError::Closed);
        }
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut job = PresentationJob { command, sender };
        let generation = job.command.id.generation;
        if matches!(job.command.kind, PresentationKind::Hide) {
            let mut kept = VecDeque::new();
            while let Some(old) = s.pending.pop_front() {
                if old.command.id.generation == generation
                    && old.command.id.sequence <= job.command.id.sequence
                {
                    s.superseded.push(old);
                } else {
                    kept.push_back(old);
                }
            }
            s.pending = kept;
        } else if let Some(old) = s.pending.back_mut().filter(|old| {
            old.command.id.generation == generation
                && old.command.id.sequence < job.command.id.sequence
        }) {
            if let PresentationKind::Show {
                origin,
                size_logical,
                model,
            } = &old.command.kind
            {
                let merged = match &job.command.kind {
                    PresentationKind::Update { model } => Some(PresentationKind::Show {
                        origin: *origin,
                        size_logical: *size_logical,
                        model: model.clone(),
                    }),
                    PresentationKind::Resize {
                        origin,
                        size_logical,
                    } => Some(PresentationKind::Show {
                        origin: *origin,
                        size_logical: *size_logical,
                        model: model.clone(),
                    }),
                    _ => None,
                };
                if let Some(kind) = merged {
                    job.command.kind = kind;
                    let old = s.pending.pop_back().unwrap();
                    s.superseded.push(old);
                }
            }
        }
        s.pending.push_back(job);
        self.wake.notify_one();
        Ok(receiver)
    }
    /// Nonblocking manual consumer seam, for deterministic tests.
    pub fn seal(&self) {
        self.state.lock().unwrap().closed = true;
        self.wake.notify_all();
    }
    pub fn complete_superseded(&self, sink: &AckSink) {
        let jobs = std::mem::take(&mut self.state.lock().unwrap().superseded);
        for job in jobs {
            job.supersede(sink);
        }
    }
    pub fn try_next(&self) -> Option<PresentationJob> {
        self.state.lock().unwrap().pending.pop_front()
    }
    pub fn next(&self, sink: &AckSink) -> Option<PresentationJob> {
        loop {
            let (retired, job, closed) = {
                let mut s = self.state.lock().unwrap();
                while s.pending.is_empty() && s.superseded.is_empty() && !s.closed {
                    s = self.wake.wait(s).unwrap();
                }
                (
                    std::mem::take(&mut s.superseded),
                    s.pending.pop_front(),
                    s.closed,
                )
            };
            for job in retired {
                job.supersede(sink);
            }
            if job.is_some() || closed {
                return job;
            }
        }
    }
    /// Rejects future submissions and resolves all queued commands. The worker caller owns
    /// the final shutdown acknowledgment (its thread completion), after the active job ends.
    pub fn close(&self, sink: &AckSink) {
        let jobs = {
            let mut s = self.state.lock().unwrap();
            s.closed = true;
            let mut jobs = std::mem::take(&mut s.superseded);
            jobs.extend(s.pending.drain(..));
            self.wake.notify_all();
            jobs
        };
        for job in jobs {
            job.supersede(sink);
        }
    }
}
