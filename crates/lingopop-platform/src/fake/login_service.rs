use std::sync::{Arc, Mutex};

use crate::macos_login::{MainAppService, ServiceError, ServiceStatus};

/// One call that reached the fake `SMAppService.mainApp`, in order. `status` reads are counted
/// separately (`FakeMainAppService::status_reads`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCall {
    Register,
    Unregister,
    OpenLoginItemsSettings,
}

struct State {
    bundled: bool,
    status: ServiceStatus,
    after_register: ServiceStatus,
    after_unregister: ServiceStatus,
    register_failure: Option<(i64, ServiceStatus)>,
    unregister_failure: Option<(i64, ServiceStatus)>,
    calls: Vec<ServiceCall>,
    status_reads: usize,
}

/// Fake `SMAppService.mainApp`. Clones share state, so a test keeps a handle after handing one to
/// a `ServiceLoginItem`. It can only act on the main app: there is no way to name another login
/// item (the frozen Swift app's included).
#[derive(Clone)]
pub struct FakeMainAppService(Arc<Mutex<State>>);

impl FakeMainAppService {
    pub fn new(bundled: bool, status: ServiceStatus) -> Self {
        Self(Arc::new(Mutex::new(State {
            bundled,
            status,
            after_register: ServiceStatus::Enabled,
            after_unregister: ServiceStatus::NotRegistered,
            register_failure: None,
            unregister_failure: None,
            calls: Vec::new(),
            status_reads: 0,
        })))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The status after a successful `register` (default `Enabled`).
    pub fn set_status_after_register(&self, status: ServiceStatus) {
        self.lock().after_register = status;
    }

    /// The next `register` fails with `code` and leaves the service in `status`.
    pub fn fail_register(&self, code: i64, status: ServiceStatus) {
        self.lock().register_failure = Some((code, status));
    }

    /// The next `unregister` fails with `code` and leaves the service in `status`.
    pub fn fail_unregister(&self, code: i64, status: ServiceStatus) {
        self.lock().unregister_failure = Some((code, status));
    }

    pub fn calls(&self) -> Vec<ServiceCall> {
        self.lock().calls.clone()
    }

    /// The current status without counting as a read.
    pub fn status(&self) -> ServiceStatus {
        self.lock().status
    }

    pub fn status_reads(&self) -> usize {
        self.lock().status_reads
    }
}

impl MainAppService for FakeMainAppService {
    fn is_installed_bundle(&self) -> bool {
        self.lock().bundled
    }

    fn status(&self) -> ServiceStatus {
        let mut state = self.lock();
        state.status_reads += 1;
        state.status
    }

    fn register(&self) -> Result<(), ServiceError> {
        let mut state = self.lock();
        state.calls.push(ServiceCall::Register);
        if let Some((code, status)) = state.register_failure.take() {
            state.status = status;
            return Err(ServiceError { code });
        }
        state.status = state.after_register;
        Ok(())
    }

    fn unregister(&self) -> Result<(), ServiceError> {
        let mut state = self.lock();
        state.calls.push(ServiceCall::Unregister);
        if let Some((code, status)) = state.unregister_failure.take() {
            state.status = status;
            return Err(ServiceError { code });
        }
        state.status = state.after_unregister;
        Ok(())
    }

    fn open_login_items_settings(&self) {
        self.lock().calls.push(ServiceCall::OpenLoginItemsSettings);
    }
}
