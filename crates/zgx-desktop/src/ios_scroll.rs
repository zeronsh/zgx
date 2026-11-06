//! Scrolling with UIKit's own physics: a `UIScrollView` nobody sees, its pan
//! gesture on the window's view, says where zgx's scroll views should be.
//! Tracking, deceleration, rubber-banding and a tap on the status bar are
//! UIKit's, not an imitation; zgx draws. When the pan begins, UIKit cancels the
//! finger's touches, so a press under it ends as it does in a native list.
//!
//! When a finger lands the scroller takes on the zgx scroll view under it.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, NSObject, NSObjectProtocol, Sel},
    sel,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use std::{cell::RefCell, rc::Rc, sync::Arc};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use zgx::elastic::ScrollDriver;

thread_local! {
    /// The window's scroller (iOS's one window), for its class's methods.
    static CURRENT: std::cell::RefCell<Option<Rc<State>>> = const { std::cell::RefCell::new(None) };
}

struct State {
    /// What finds the zgx scroll view at a window point.
    at: Box<dyn Fn(f32, f32) -> Option<ScrollDriver>>,
    /// The zgx scroll view the scroller stands for.
    driver: RefCell<Option<ScrollDriver>>,
    /// Finds what a tap on the status bar scrolls.
    top: RefCell<Box<dyn Fn() -> Option<ScrollDriver>>>,
    /// Its positions are ours being set, not UIKit's motion.
    configuring: std::cell::Cell<bool>,
    window: Arc<winit::window::Window>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<State>]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    impl Delegate {
        #[unsafe(method(scrollViewDidScroll:))]
        fn did_scroll(&self, view: &AnyObject) {
            let state = self.ivars();
            if state.configuring.get() {
                return;
            }
            let Some(driver) = state.driver.borrow().clone() else {
                return;
            };
            // SAFETY: UIScrollView's getter.
            let at: CGPoint = unsafe { msg_send![view, contentOffset] };
            let horizontal = driver.metrics().is_some_and(|m| m.horizontal);
            driver.drive(if horizontal { at.x } else { at.y } as f32);
            state.window.request_redraw();
        }
        #[unsafe(method(scrollViewShouldScrollToTop:))]
        fn should_scroll_to_top(&self, view: &AnyObject) -> Bool {
            let state = self.ivars();
            let driver = (state.top.borrow())();
            let Some(driver) = driver else {
                return Bool::NO;
            };
            configure(state, view, driver);
            Bool::YES
        }
    }
);

/// A `UIScrollView` no touch lands on: its pan is on the window's view, and a
/// tap on the status bar only finds a scroll view that takes interaction.
fn scroller_class() -> &'static AnyClass {
    static CLASS: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    extern "C-unwind" fn hit_test(
        _: &AnyObject,
        _: Sel,
        _: CGPoint,
        _: *mut AnyObject,
    ) -> *mut AnyObject {
        std::ptr::null_mut()
    }
    /// A pan may start from a finger zgx never heard of, one on a hosted
    /// control: what lies under it is decided as UIKit asks.
    extern "C-unwind" fn should_begin(this: &AnyObject, _: Sel, pan: &AnyObject) -> Bool {
        // SAFETY: UIScrollView's and UIGestureRecognizer's own methods.
        unsafe {
            let moving: bool = msg_send![this, isDecelerating];
            if !moving {
                let at: CGPoint = msg_send![pan, locationInView: std::ptr::null::<AnyObject>()];
                let state = CURRENT.with(|current| current.borrow().clone());
                if let Some(state) = state {
                    match (state.at)(at.x as f32, at.y as f32) {
                        Some(driver) => configure(&state, this, driver),
                        None => {
                            *state.driver.borrow_mut() = None;
                            let empty = CGSize {
                                width: 0.,
                                height: 0.,
                            };
                            let _: () = msg_send![this, setContentSize: empty];
                        }
                    }
                }
            }
            msg_send![super(this, class!(UIScrollView)), gestureRecognizerShouldBegin: pan]
        }
    }
    CLASS.get_or_init(|| {
        let mut builder = ClassBuilder::new(c"ZgxScroller", class!(UIScrollView))
            .expect("one class of this name");
        // SAFETY: `should_begin` has `gestureRecognizerShouldBegin:`'s signature.
        unsafe {
            builder.add_method(
                sel!(gestureRecognizerShouldBegin:),
                should_begin as extern "C-unwind" fn(_, _, _) -> _,
            );
        }
        // SAFETY: `hit_test` has `hitTest:withEvent:`'s signature.
        unsafe {
            builder.add_method(
                sel!(hitTest:withEvent:),
                hit_test as extern "C-unwind" fn(_, _, _, _) -> _,
            );
        }
        builder.register()
    })
}

fn configure(state: &State, view: &AnyObject, driver: ScrollDriver) {
    let Some(metrics) = driver.metrics() else {
        return;
    };
    state.configuring.set(true);
    let (offset_x, offset_y) = if metrics.horizontal {
        (metrics.offset as f64, 0.)
    } else {
        (0., metrics.offset as f64)
    };
    let size = CGSize {
        width: metrics.size.0 as f64,
        height: metrics.size.1 as f64,
    };
    let content = CGSize {
        width: if metrics.horizontal {
            metrics.extent.0 as f64
        } else {
            size.width
        },
        height: if metrics.horizontal {
            size.height
        } else {
            metrics.extent.1 as f64
        },
    };
    let frame = CGRect {
        origin: CGPoint { x: 0., y: 0. },
        size,
    };
    // SAFETY: UIScrollView's setters, on the main thread.
    unsafe {
        let _: () = msg_send![view, setFrame: frame];
        let _: () = msg_send![view, setContentSize: content];
        let _: () = msg_send![view, setContentOffset: CGPoint { x: offset_x, y: offset_y }];
    }
    *state.driver.borrow_mut() = Some(driver);
    state.configuring.set(false);
}

pub(crate) struct NativeScroller {
    view: Retained<AnyObject>,
    _delegate: Retained<Delegate>,
    state: Rc<State>,
}

impl NativeScroller {
    pub(crate) fn new(
        window: Arc<winit::window::Window>,
        top: impl Fn() -> Option<ScrollDriver> + 'static,
        at: impl Fn(f32, f32) -> Option<ScrollDriver> + 'static,
    ) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
            return None;
        };
        let state = Rc::new(State {
            at: Box::new(at),
            driver: RefCell::default(),
            top: RefCell::new(Box::new(top)),
            configuring: Default::default(),
            window: window.clone(),
        });
        CURRENT.with(|current| *current.borrow_mut() = Some(state.clone()));
        let delegate = Delegate::alloc(mtm).set_ivars(state.clone());
        // SAFETY: NSObject's `init`, on a fresh allocation with its ivars set.
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        // SAFETY: winit's view, alive with its window; UIKit's constructor and
        // setters, on the main thread checked above. The view keeps the pan.
        let view = unsafe {
            let root: *const AnyObject = handle.ui_view.as_ptr().cast();
            let view: Retained<AnyObject> = msg_send![scroller_class(), new];
            let _: () = msg_send![&*view, setDelegate: &*delegate];
            // Not hidden or transparent, which a tap on the status bar skips:
            // it has nothing to draw, and lies under the base plane.
            let _: () = msg_send![&*view, setAccessibilityElementsHidden: true];
            let _: () = msg_send![&*view, setShowsVerticalScrollIndicator: false];
            let _: () = msg_send![&*view, setShowsHorizontalScrollIndicator: false];
            // UIScrollViewContentInsetAdjustmentNever: zgx lays out by the
            // safe area itself.
            let _: () = msg_send![&*view, setContentInsetAdjustmentBehavior: 2isize];
            let _: () = msg_send![&*view, setScrollsToTop: true];
            let _: () = msg_send![&*root, insertSubview: &*view, atIndex: 0isize];
            let pan: Retained<AnyObject> = msg_send![&*view, panGestureRecognizer];
            let _: () = msg_send![&*root, addGestureRecognizer: &*pan];
            view
        };
        // `ZGX_IOS_SCROLL_PROBE=y`: three seconds in, UIKit animates the main
        // scroll view to `y`, for a check of this path with no finger on it.
        if let Some(y) = zgx::env::var("ZGX_IOS_SCROLL_PROBE")
            .ok()
            .and_then(|y| y.parse::<f64>().ok())
        {
            let (view, state) = (view.clone(), state.clone());
            let fire = block2::RcBlock::new(move |_: *mut AnyObject| {
                let driver = (state.top.borrow())();
                if let Some(driver) = driver {
                    configure(&state, &view, driver);
                    // SAFETY: UIScrollView's method, on the main thread.
                    let _: () = unsafe {
                        msg_send![&*view, setContentOffset: CGPoint { x: 0., y }, animated: true]
                    };
                }
            });
            // SAFETY: NSTimer's constructor; the run loop keeps it.
            let _: Retained<AnyObject> = unsafe {
                msg_send![class!(NSTimer),
                    scheduledTimerWithTimeInterval: 3f64,
                    repeats: false,
                    block: &*fire]
            };
        }
        Some(Self {
            view,
            _delegate: delegate,
            state,
        })
    }
    /// A finger landed over `driver` (none: nothing there scrolls, and the
    /// finger is zgx's alone). A scroller in motion keeps its scroll view:
    /// the finger catches it, as it does in a native list.
    pub(crate) fn finger_down(&self, driver: Option<ScrollDriver>) {
        // SAFETY: UIScrollView's getters and setters, on the main thread.
        unsafe {
            let moving: bool = msg_send![&*self.view, isDecelerating];
            let tracking: bool = msg_send![&*self.view, isTracking];
            if moving || tracking {
                return;
            }
            match driver {
                Some(driver) => configure(&self.state, &self.view, driver),
                // Left enabled: UIKit gathers a touch's recognizers before the
                // view hears of the touch, so a disabled pan would miss it for
                // good. With nothing to scroll it never begins.
                None => {
                    *self.state.driver.borrow_mut() = None;
                    let empty = CGSize {
                        width: 0.,
                        height: 0.,
                    };
                    let _: () = msg_send![&*self.view, setContentSize: empty];
                }
            }
        }
    }
    /// Whether UIKit tracks scrolling for this finger: zgx offers it no pan.
    pub(crate) fn tracks(&self) -> bool {
        self.state.driver.borrow().is_some()
    }
}
