//! `FakeMainThread`: a main loop the test drives. Posted jobs wait in a queue until the test
//! runs them (or a pump thread does), on the running thread, which then counts as "main".
use crate::main_thread::{dispatch_error, MainThread};
use crate::PlatformError;
use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

type Job = Box<dyn FnOnce() + Send>;

thread_local! {
    static ON_FAKE_MAIN: Cell<bool> = const { Cell::new(false) };
}

/// True while the current thread runs fake main-thread work (see [`FakeMainThread`]).
pub fn on_fake_main() -> bool {
    ON_FAKE_MAIN.with(Cell::get)
}

#[derive(Default)]
struct State {
    jobs: VecDeque<Job>,
    posted: usize,
    closed: bool,
}

#[derive(Default)]
pub struct FakeMainThread {
    state: Mutex<State>,
    wake: Condvar,
}

struct MainScope(bool);
impl MainScope {
    fn enter() -> Self {
        Self(ON_FAKE_MAIN.with(|m| m.replace(true)))
    }
}
impl Drop for MainScope {
    fn drop(&mut self) {
        let previous = self.0;
        ON_FAKE_MAIN.with(|m| m.set(previous));
    }
}

impl FakeMainThread {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn pending(&self) -> usize {
        self.state().jobs.len()
    }
    /// Jobs ever posted (inline main-thread work is not posted).
    pub fn posted(&self) -> usize {
        self.state().posted
    }
    /// Blocks until at least `n` jobs are queued: a completion barrier, never a clock.
    pub fn wait_for_pending(&self, n: usize) {
        let mut s = self.state();
        while s.jobs.len() < n && !s.closed {
            s = self.wake.wait(s).unwrap_or_else(|p| p.into_inner());
        }
    }
    /// Runs the jobs queued at the time of the call, in order, on this thread as "main".
    /// Jobs posted while they run stay queued. Returns how many ran.
    pub fn run_pending(&self) -> usize {
        let count = self.pending();
        let _main = MainScope::enter();
        let mut ran = 0;
        for _ in 0..count {
            let Some(job) = self.state().jobs.pop_front() else {
                break;
            };
            job();
            ran += 1;
        }
        ran
    }
    /// Runs `f` on this thread as if it were main (for inline-path tests).
    pub fn enter_main<T>(&self, f: impl FnOnce() -> T) -> T {
        let _main = MainScope::enter();
        f()
    }
    /// The loop went away: queued jobs are dropped unrun and later posts fail.
    pub fn close(&self) {
        let dropped = {
            let mut s = self.state();
            s.closed = true;
            std::mem::take(&mut s.jobs)
        };
        self.wake.notify_all();
        drop(dropped);
    }
    /// Runs jobs as they arrive until [`Self::close`]; for a pump thread playing the loop.
    pub fn pump(&self) {
        let _main = MainScope::enter();
        loop {
            let job = {
                let mut s = self.state();
                loop {
                    if let Some(job) = s.jobs.pop_front() {
                        break Some(job);
                    }
                    if s.closed {
                        break None;
                    }
                    s = self.wake.wait(s).unwrap_or_else(|p| p.into_inner());
                }
            };
            match job {
                Some(job) => job(),
                None => return,
            }
        }
    }
}

impl MainThread for FakeMainThread {
    fn is_main_thread(&self) -> bool {
        on_fake_main()
    }
    fn post(&self, job: Job) -> Result<(), PlatformError> {
        let mut s = self.state();
        if s.closed {
            return Err(dispatch_error());
        }
        s.jobs.push_back(job);
        s.posted += 1;
        drop(s);
        self.wake.notify_all();
        Ok(())
    }
}
