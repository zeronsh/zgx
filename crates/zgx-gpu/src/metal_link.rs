//! Experiment (`ZGX_METAL_LINK=1`): scroll drawables paced by a
//! `CAMetalDisplayLink` on their layer (macOS 14+), as Zed's GPUI paces its
//! windows. The link hands each update a drawable early enough for the frame
//! drawn into it to reach the display at the update's target; a frame never
//! asks the layer for one (`nextDrawable`, which waits for the compositor to
//! release one). Updates come on the main run loop; each wakes the host
//! (`PlaneWaker`), which draws the frame then (`GpuRenderer::take_link_tick`).
//! A drawable no frame took goes back to the pool at the next update.
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, ProtocolObject},
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::{
    CAMetalDisplayLink, CAMetalDisplayLinkDelegate, CAMetalDisplayLinkUpdate, CAMetalDrawable,
    CAMetalLayer,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

type Drawable = Retained<ProtocolObject<dyn CAMetalDrawable>>;

/// `ZGX_METAL_LINK=1`: scroll drawables come from a `CAMetalDisplayLink`.
pub(crate) static METAL_LINK: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| zgx::env::var_os("ZGX_METAL_LINK").is_some_and(|v| v != "0"));

/// What the link's last update handed over, until a frame takes it.
#[derive(Default)]
pub(crate) struct Slot {
    drawable: RefCell<Option<Drawable>>,
    /// The update's `targetPresentationTimestamp` and `targetTimestamp`, in
    /// host seconds (CACurrentMediaTime's), not yet delivered to the host.
    tick: Cell<Option<(f64, f64)>>,
    /// Updates so far, and those whose drawable no frame took.
    pub(crate) updates: Cell<u64>,
    pub(crate) unused: Cell<u64>,
}

struct Ivars {
    slot: Rc<Slot>,
    wake: Option<crate::PlaneWaker>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the link calls its
    // delegate on the run loop it is added to, the main one.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CAMetalDisplayLinkDelegate for Delegate {
        #[unsafe(method(metalDisplayLink:needsUpdate:))]
        fn needs_update(&self, _link: &CAMetalDisplayLink, update: &CAMetalDisplayLinkUpdate) {
            let this = self.retain();
            let ivars = this.ivars();
            let slot = &ivars.slot;
            slot.updates.set(slot.updates.get() + 1);
            // The last one's, untaken, goes back to the pool.
            if slot.drawable.replace(Some(update.drawable())).is_some() {
                slot.unused.set(slot.unused.get() + 1);
            }
            slot.tick.set(Some((
                update.targetPresentationTimestamp(),
                update.targetTimestamp(),
            )));
            if let Some(wake) = &ivars.wake {
                wake();
            }
        }
    }
);

/// A `CAMetalDisplayLink` on a scroll drawables layer.
pub(crate) struct MetalLink {
    link: Retained<CAMetalDisplayLink>,
    // The link holds its delegate weakly.
    _delegate: Retained<Delegate>,
    pub(crate) slot: Rc<Slot>,
}

impl MetalLink {
    /// Paused until `set_paused(false)`. `None` before macOS 14, or off the
    /// main thread.
    pub(crate) fn new(layer: &CAMetalLayer, wake: Option<crate::PlaneWaker>) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        AnyClass::get(c"CAMetalDisplayLink")?;
        let slot = Rc::new(Slot::default());
        let delegate = Delegate::alloc(mtm).set_ivars(Ivars {
            slot: slot.clone(),
            wake,
        });
        // SAFETY: NSObject's designated initializer, on a fresh allocation
        // with its ivars set.
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        let link = CAMetalDisplayLink::initWithMetalLayer(CAMetalDisplayLink::alloc(), layer);
        link.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        link.setPaused(true);
        // SAFETY: on the main thread, with the main run loop, which outlives
        // the link (invalidated on drop).
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
        Some(Self {
            link,
            _delegate: delegate,
            slot,
        })
    }
    /// A paused link hands out no drawables; pausing returns the held one.
    pub(crate) fn set_paused(&self, paused: bool) {
        if self.link.isPaused() != paused {
            self.link.setPaused(paused);
        }
        if paused {
            self.slot.drawable.take();
            self.slot.tick.take();
        }
    }
    pub(crate) fn paused(&self) -> bool {
        self.link.isPaused()
    }
    /// The drawable the last update handed over, if no frame took it yet.
    pub(crate) fn take_drawable(&self) -> Option<Drawable> {
        self.slot.drawable.take()
    }
    /// The last update's targets (presentation, then the refresh), once.
    pub(crate) fn take_tick(&self) -> Option<(f64, f64)> {
        self.slot.tick.take()
    }
}

impl Drop for MetalLink {
    fn drop(&mut self) {
        self.link.invalidate();
    }
}
