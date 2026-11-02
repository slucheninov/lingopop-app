//! The !Send service stays in the event-loop thread's TLS. Registry locks never span
//! posting a main-thread job, service calls, or emitting on_change.
use crate::settings_cmds::HotkeyStatus;
use lingopop_core::{hotkey::Hotkey, operation::AIOperation};
use lingopop_orchestrator::Orchestrator;
use lingopop_platform::{GlobalHotkeyService, HotkeyId, HotkeyService, PlatformError};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

thread_local! {
    static SERVICE: RefCell<Option<Box<dyn HotkeyService>>> = const { RefCell::new(None) };
}
pub trait MainThread: Send + Sync {
    fn run(&self, job: Box<dyn FnOnce() + Send>);
}
pub struct TauriMainThread(pub tauri::AppHandle);
impl MainThread for TauriMainThread {
    fn run(&self, job: Box<dyn FnOnce() + Send>) {
        if self.0.run_on_main_thread(job).is_err() {
            log::warn!("hotkey main-thread job could not be posted");
        }
    }
}
#[doc(hidden)]
pub fn install_service_for_tests(service: Box<dyn HotkeyService>) {
    SERVICE.with(|slot| *slot.borrow_mut() = Some(service));
}
#[derive(Default)]
struct RegistryState {
    by_id: HashMap<HotkeyId, AIOperation>,
    status: BTreeMap<AIOperation, HotkeyStatus>,
    wanted: BTreeMap<AIOperation, Hotkey>,
    suspended: bool,
}
#[derive(Clone)]
pub struct HotkeyRegistry {
    inner: Arc<Mutex<RegistryState>>,
    main: Arc<dyn MainThread>,
    on_change: Arc<dyn Fn() + Send + Sync>,
}
impl HotkeyRegistry {
    pub fn new(main: Arc<dyn MainThread>, on_change: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryState {
                status: AIOperation::ALL
                    .into_iter()
                    .map(|op| (op, HotkeyStatus::None))
                    .collect(),
                ..RegistryState::default()
            })),
            main,
            on_change,
        }
    }
    pub fn status(&self) -> BTreeMap<AIOperation, HotkeyStatus> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .status
            .clone()
    }
    pub fn lookup(&self, id: HotkeyId) -> Option<AIOperation> {
        let state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.suspended {
            None
        } else {
            state.by_id.get(&id).copied()
        }
    }
    pub fn apply(&self, hotkeys: BTreeMap<AIOperation, Hotkey>) {
        let suspended = {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.wanted = hotkeys;
            state.suspended
        };
        if !suspended {
            self.reconcile();
        }
    }
    pub fn suspend(&self, suspended: bool) {
        {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.suspended == suspended {
                return;
            }
            state.suspended = suspended;
        }
        self.reconcile();
    }
    fn reconcile(&self) {
        let registry = self.clone();
        self.main.run(Box::new(move || {
            // Read the CURRENT plan inside the serialized event-loop job. A queued apply
            // must not overwrite a newer suspend/resume with a stale captured plan.
            let (active, wanted, suspended) = {
                let mut state = registry
                    .inner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (
                    std::mem::take(&mut state.by_id),
                    state.wanted.clone(),
                    state.suspended,
                )
            };
            let result = SERVICE.with(|slot| {
                let service = slot.borrow();
                if let Some(service) = service.as_ref() {
                    unregister_all(service.as_ref(), active);
                    if suspended {
                        None
                    } else {
                        Some(register_all(service.as_ref(), &wanted))
                    }
                } else {
                    Some((
                        HashMap::new(),
                        AIOperation::ALL
                            .into_iter()
                            .map(|op| {
                                (
                                    op,
                                    if wanted.contains_key(&op) {
                                        HotkeyStatus::Invalid
                                    } else {
                                        HotkeyStatus::None
                                    },
                                )
                            })
                            .collect(),
                    ))
                }
            });
            if let Some((by_id, status)) = result {
                let mut state = registry
                    .inner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.by_id = by_id;
                state.status = status;
            }
            (registry.on_change)();
        }));
    }
}
pub(crate) fn register_all(
    service: &dyn HotkeyService,
    hotkeys: &BTreeMap<AIOperation, Hotkey>,
) -> (
    HashMap<HotkeyId, AIOperation>,
    BTreeMap<AIOperation, HotkeyStatus>,
) {
    let mut by_id = HashMap::new();
    let status = AIOperation::ALL
        .into_iter()
        .map(|op| {
            let status = match hotkeys.get(&op) {
                None => HotkeyStatus::None,
                Some(h) => match service.register(h) {
                    Ok(id) => {
                        by_id.insert(id, op);
                        HotkeyStatus::Registered
                    }
                    Err(PlatformError::HotkeyConflict) => HotkeyStatus::Conflict,
                    Err(error) => {
                        log::warn!(
                            "hotkey for {} was not registered: {error}",
                            op.english_name()
                        );
                        HotkeyStatus::Invalid
                    }
                },
            };
            (op, status)
        })
        .collect();
    (by_id, status)
}
pub(crate) fn unregister_all(service: &dyn HotkeyService, by_id: HashMap<HotkeyId, AIOperation>) {
    for id in by_id.into_keys() {
        if let Err(error) = service.unregister(id) {
            log::warn!("hotkey unregister failed: {error}");
        }
    }
}
/// Setup posts installation onto the event-loop thread, just like later transitions.
pub fn install(
    registry: &HotkeyRegistry,
    orchestrator: Orchestrator,
    hotkeys: &BTreeMap<AIOperation, Hotkey>,
) {
    let current = registry.clone();
    let hotkeys = hotkeys.clone();
    registry.main.run(Box::new(move || {
        let service = match GlobalHotkeyService::new() {
            Ok(service) => service,
            Err(error) => {
                log::error!("hotkeys are unavailable: {error}");
                return;
            }
        };
        let events = service.events();
        SERVICE.with(|slot| *slot.borrow_mut() = Some(Box::new(service)));
        current.apply(hotkeys);
        if std::thread::Builder::new()
            .name("lingopop-hotkey-forwarder".into())
            .spawn(move || {
                for event in events.iter() {
                    let Some(operation) = current.lookup(event.id) else {
                        continue;
                    };
                    let orchestrator = orchestrator.clone();
                    tauri::async_runtime::spawn(
                        async move { orchestrator.trigger(operation).await },
                    );
                }
            })
            .is_err()
        {
            log::error!("hotkey forwarder thread could not start");
        }
    }));
}
#[tauri::command]
pub async fn hotkeys_suspend(
    state: tauri::State<'_, crate::commands::ShellState>,
    suspended: bool,
) -> Result<(), String> {
    state.hotkeys.suspend(suspended);
    Ok(())
}
#[cfg(test)]
#[path = "task8_hotkey_tests.rs"]
mod task8_tests;
