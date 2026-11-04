//! Main-thread AppKit topology publication; worker reads use immutable cached geometry.
use crate::macos_geometry::{MacDisplay, MacGeometry, NativeRect, TopologyCache};
use crate::{MonitorInfo, Monitors, PlatformError, Point};
use core_graphics::display::CGDisplay;
use core_graphics::event::CGEvent;
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc2::{rc::Retained, runtime::ProtocolObject, MainThreadMarker};
use objc2_app_kit::{NSApplicationDidChangeScreenParametersNotification, NSScreen};
use objc2_foundation::{
    ns_string, NSNotificationCenter, NSNumber, NSObjectProtocol, NSOperationQueue,
};
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Debug, Clone)]
pub struct MacMonitors {
    cache: TopologyCache,
    initialized: Arc<AtomicBool>,
}
impl Default for MacMonitors {
    fn default() -> Self {
        Self::new()
    }
}
impl MacMonitors {
    /// Empty until shell setup initializes it on the main thread. Construction never dispatches.
    pub fn new() -> Self {
        Self::from_geometry(MacGeometry::new(0, 0.0, vec![]).expect("empty topology"))
    }
    pub fn from_geometry(geometry: MacGeometry) -> Self {
        Self {
            cache: TopologyCache::new(geometry),
            initialized: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn geometry(&self) -> MacGeometry {
        self.cache.snapshot()
    }
    pub fn topology_revision(&self) -> u64 {
        self.cache.topology_revision()
    }
    /// Call once from shell setup. AppKit enumeration and observer registration stay here.
    pub fn initialize_on_main_thread(&self) -> Result<(), PlatformError> {
        let Some(mtm) = MainThreadMarker::new() else {
            crate::error::platform_log("display topology requires main thread");
            return Ok(());
        };
        if self.initialized.load(Ordering::Acquire) {
            return Ok(());
        }
        self.cache.initialize_with(
            || self.register_observer(),
            || read_topology(mtm),
            &crate::error::platform_log,
        );
        self.initialized.store(true, Ordering::Release);
        Ok(())
    }
    fn register_observer(&self) {
        let cache = self.cache.clone();
        let block = block2::RcBlock::new(move |_| {
            if let Some(mtm) = MainThreadMarker::new() {
                cache.refresh_with(|| read_topology(mtm), &crate::error::platform_log);
            } else {
                crate::error::platform_log("display topology observer requires main thread");
            }
        });
        let center = NSNotificationCenter::defaultCenter();
        // SAFETY: name is AppKit's static notification; no object filter; mainQueue ensures
        // AppKit is only read on main. The block captures only a Send + Sync topology cache.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSApplicationDidChangeScreenParametersNotification),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
        };
        OBSERVERS.with(|v| v.borrow_mut().push(Observer { center, token }));
    }
}
struct Observer {
    center: Retained<NSNotificationCenter>,
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}
impl Drop for Observer {
    fn drop(&mut self) {
        // SAFETY: this is exactly the retained token returned by this center's registration.
        unsafe { self.center.removeObserver((*self.token).as_ref()) };
    }
}
thread_local! {static OBSERVERS:RefCell<Vec<Observer>>=const {RefCell::new(Vec::new())};}
fn read_topology(mtm: MainThreadMarker) -> Result<(f64, Vec<MacDisplay>), PlatformError> {
    let primary = CGDisplay::main().bounds();
    let height = primary.size.height;
    let screens = NSScreen::screens(mtm);
    let mut displays = Vec::new();
    for screen in screens {
        let description = screen.deviceDescription();
        let value = description
            .objectForKey(ns_string!("NSScreenNumber"))
            .ok_or(PlatformError::InvalidRegion)?;
        let number = value
            .downcast_ref::<NSNumber>()
            .ok_or(PlatformError::InvalidRegion)?;
        let id = u32::try_from(number.unsignedIntegerValue())
            .map_err(|_| PlatformError::InvalidRegion)?;
        let cg = CGDisplay::new(id);
        let frame_to_desktop = |r: objc2_foundation::NSRect| {
            MacGeometry::appkit_frame_to_desktop(
                height,
                NativeRect::new(r.origin.x, r.origin.y, r.size.width, r.size.height),
            )
        };
        displays.push(MacDisplay {
            id,
            desktop_bounds: frame_to_desktop(screen.frame()),
            work_area: frame_to_desktop(screen.visibleFrame()),
            backing_scale: screen.backingScaleFactor(),
            primary: cg.is_main(),
        });
    }
    Ok((height, displays))
}
impl Monitors for MacMonitors {
    fn initialize_on_main_thread(&self) -> Result<(), PlatformError> {
        MacMonitors::initialize_on_main_thread(self)
    }
    fn cursor_position(&self) -> Result<Point, PlatformError> {
        let err = PlatformError::Os {
            api: "CGEventCreate",
            code: 0,
        };
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
            .map_err(|_| err.clone())?;
        let p = CGEvent::new(source).map_err(|_| err)?.location();
        Ok(Point {
            x: p.x.floor() as i32,
            y: p.y.floor() as i32,
        })
    }
    fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError> {
        Ok(self.cache.snapshot().monitors())
    }
}
