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
        let mtm = MainThreadMarker::new().ok_or(PlatformError::Os {
            api: "display topology requires main thread",
            code: 0,
        })?;
        if self.initialized.load(Ordering::Acquire) {
            return Ok(());
        }
        refresh(&self.cache, mtm)?;
        let cache = self.cache.clone();
        let block = block2::RcBlock::new(move |_| {
            if let Some(mtm) = MainThreadMarker::new() {
                if refresh(&cache, mtm).is_err() {
                    // Invalidate old plans even if the new topology could not be enumerated.
                    let _ = cache.publish(0.0, vec![]);
                    crate::error::platform_log("display topology refresh failed");
                }
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
        self.initialized.store(true, Ordering::Release);
        Ok(())
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
fn refresh(cache: &TopologyCache, mtm: MainThreadMarker) -> Result<(), PlatformError> {
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
        let b = cg.bounds();
        let visible = screen.visibleFrame();
        displays.push(MacDisplay {
            id,
            desktop_bounds: NativeRect::new(b.origin.x, b.origin.y, b.size.width, b.size.height),
            work_area: NativeRect::new(
                visible.origin.x,
                height - visible.origin.y - visible.size.height,
                visible.size.width,
                visible.size.height,
            ),
            backing_scale: screen.backingScaleFactor(),
            primary: cg.is_main(),
        });
    }
    cache.publish(height, displays)
}
impl Monitors for MacMonitors {
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
