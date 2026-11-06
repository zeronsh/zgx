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

unsafe extern "C" {
    static NSRunLoopCommonModes: *const AnyObject;
}

struct State {
    /// What finds the zgx scroll view at a window point.
    at: Box<dyn Fn(f32, f32) -> Option<ScrollDriver>>,
    /// Told how far past an end it shows (`ui.window.overscroll`).
    stretched: Box<dyn Fn(f32)>,
    /// The zgx scroll view the scroller stands for.
    driver: RefCell<Option<ScrollDriver>>,
    /// Finds what a tap on the status bar scrolls.
    top: RefCell<Box<dyn Fn() -> Option<ScrollDriver>>>,
    /// Its positions are ours being set, not UIKit's motion.
    configuring: std::cell::Cell<bool>,
    window: Arc<winit::window::Window>,
    /// Whether a finger is down, and whether a refresh ended under it.
    refresh_ending: Rc<(std::cell::Cell<bool>, std::cell::Cell<bool>)>,
    /// A finger is dragging: UIKit's `isDragging` stays false, its pan elsewhere.
    dragging: std::cell::Cell<bool>,
    /// A finger began dragging (`take_drag_began`).
    drag_began: std::cell::Cell<bool>,
    /// Finds the pull to refresh at a window point, and the one in play.
    pull_at: Box<dyn Fn(f32, f32) -> Option<zgx::native_control::PullRefresh>>,
    pull: RefCell<Option<zgx::native_control::PullRefresh>>,
    /// The delegate, which a `UIRefreshControl` tells.
    delegate: RefCell<Option<Retained<Delegate>>>,
    /// The scroller's control, kept whether or not it is on the scroller now.
    control: RefCell<Option<Retained<AnyObject>>>,
}

/// Take the page back from a refresh.
fn end_refresh(control: &AnyObject) {
    // SAFETY: UIRefreshControl's method, on the main thread.
    let _: () = unsafe { msg_send![control, endRefreshing] };
}

/// A pull let go past where it refreshes: the app's, which says when it's done.
fn run_refresh(state: &State, control: &AnyObject) {
    crate::ios_gestures::log("refresh began");
    // SAFETY: an object UIKit passes, kept while the refresh runs.
    let control: Retained<AnyObject> =
        unsafe { Retained::retain((control as *const AnyObject).cast_mut()) }.expect("not null");
    let window = state.window.clone();
    let ending = state.refresh_ending.clone();
    // The app's timers run as the host's loop turns, and it needn't through a
    // finger's tracking: kept turning, or what the app waits on never comes.
    let wake = window.clone();
    let tick = block2::RcBlock::new(move |_: *mut AnyObject| {
        zgx::native_view::wake();
        wake.request_redraw();
    });
    // SAFETY: NSTimer's constructor and NSRunLoop's method, on the main thread;
    // the run loop keeps the timer until `invalidate`.
    let timer: Retained<AnyObject> = unsafe {
        let timer: Retained<AnyObject> = msg_send![class!(NSTimer),
            timerWithTimeInterval: 0.1f64,
            repeats: true,
            block: &*tick];
        let run_loop: Retained<AnyObject> = msg_send![class!(NSRunLoop), mainRunLoop];
        let _: () = msg_send![&*run_loop, addTimer: &*timer, forMode: NSRunLoopCommonModes];
        timer
    };
    let done: Rc<dyn Fn()> = Rc::new(move || {
        crate::ios_gestures::log("refresh ended");
        // SAFETY: NSTimer's method, on the main thread.
        let _: () = unsafe { msg_send![&*timer, invalidate] };
        // With a finger still down UIKit can't take the page back: then, as it lifts.
        if ending.0.get() {
            ending.1.set(true);
        } else {
            end_refresh(&control);
        }
        window.request_redraw();
    });
    let pull = state.pull.borrow().clone();
    match pull {
        Some(pull) => (pull.refresh)(done),
        None => done(),
    }
    state.window.request_redraw();
}

/// How far past the top a pull refreshes, in points.
const PULL_REFRESHES: f64 = 114.;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<State>]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    impl Delegate {
        #[unsafe(method(refreshed:))]
        fn refreshed(&self, control: &AnyObject) {
            run_refresh(self.ivars(), control);
        }
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
            // A pull past where a table view's refreshes (114 points, measured).
            // The control draws from the scroller but doesn't fire with its pan on
            // another view.
            if at.y < -PULL_REFRESHES && state.pull.borrow().is_some() {
                let control = state.control.borrow().clone();
                if let Some(control) = control {
                    // SAFETY: UIRefreshControl's and UIScrollView's methods, on the
                    // main thread.
                    let fire = unsafe {
                        let refreshing: bool = msg_send![&*control, isRefreshing];
                        !refreshing && state.dragging.get()
                    };
                    if fire {
                        // SAFETY: as above.
                        let _: () = unsafe { msg_send![&*control, beginRefreshing] };
                        run_refresh(state, &control);
                    }
                }
            }
            // What grew or shrank under the finger: left as it was when the finger
            // landed, UIKit would stop it at an end that has moved.
            if let Some(metrics) = driver.metrics() {
                // SAFETY: UIScrollView's getter and setter, on the main thread.
                unsafe {
                    let had: CGSize = msg_send![view, contentSize];
                    let extent = if metrics.horizontal {
                        CGSize {
                            width: metrics.extent.0 as f64,
                            height: had.height,
                        }
                    } else {
                        CGSize {
                            width: had.width,
                            height: metrics.extent.1 as f64,
                        }
                    };
                    if (extent.width - had.width).abs() > 0.5
                        || (extent.height - had.height).abs() > 0.5
                    {
                        crate::ios_gestures::log(&format!(
                            "scroll content changed {:.0} -> {:.0}",
                            had.height, extent.height
                        ));
                        let _: () = msg_send![view, setContentSize: extent];
                    }
                }
            }
            let horizontal = driver.metrics().is_some_and(|m| m.horizontal);
            driver.drive(if horizontal { at.x } else { at.y } as f32);
            if let Some(metrics) = driver.metrics() {
                (state.stretched)(metrics.stretch);
            }
            state.window.request_redraw();
        }
        #[unsafe(method(scrollViewDidEndDragging:willDecelerate:))]
        fn did_end_dragging(&self, view: &AnyObject, coasts: bool) {
            // SAFETY: UIScrollView's getter, UIKit calling on the main thread.
            let at: CGPoint = unsafe { msg_send![view, contentOffset] };
            crate::ios_gestures::log(&format!("scroll ended y={:.0} coasts={coasts}", at.y));
            let state = self.ivars();
            state.dragging.set(false);
            state.refresh_ending.0.set(false);
            zgx::native_view::wake();
            state.window.request_redraw();
            if state.refresh_ending.1.replace(false)
                && let Some(control) = &*state.control.borrow()
            {
                end_refresh(control);
            }
        }
        #[unsafe(method(scrollViewWillBeginDragging:))]
        fn will_begin_dragging(&self, view: &AnyObject) {
            crate::ios_gestures::log("scroll dragging");
            let state = self.ivars();
            state.dragging.set(true);
            state.refresh_ending.0.set(true);
            state.drag_began.set(true);
            // At once, under the finger: the host hears of the drag only as it
            // ends, its loop not turning through a scroller's motion.
            if crate::ios_system::keyboard_height() > 0. {
                // SAFETY: UIView's getter and method, UIKit calling on the main
                // thread.
                unsafe {
                    let parent: Option<Retained<AnyObject>> = msg_send![view, superview];
                    if let Some(parent) = parent {
                        let _: Bool = msg_send![&*parent, endEditing: true];
                    }
                }
            }
            state.window.request_redraw();
        }
        #[unsafe(method(scrollViewShouldScrollToTop:))]
        fn should_scroll_to_top(&self, view: &AnyObject) -> Bool {
            let state = self.ivars();
            let driver = (state.top.borrow())();
            let Some(driver) = driver else {
                return Bool::NO;
            };
            configure(state, view, driver, None);
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
            // A page scrolls itself: its finger isn't zgx's to scroll with.
            let root: Option<Retained<AnyObject>> = msg_send![pan, view];
            if let Some(root) = root {
                let at: CGPoint = msg_send![pan, locationInView: &*root];
                let hit: Option<Retained<AnyObject>> =
                    msg_send![&*root, hitTest: at, withEvent: std::ptr::null::<AnyObject>()];
                let mut on = hit;
                while let Some(view) = on {
                    if std::ptr::eq(&*view, &*root) {
                        break;
                    }
                    let page: bool = msg_send![&*view, isKindOfClass: class!(WKWebView)];
                    if page {
                        return Bool::NO;
                    }
                    on = msg_send![&*view, superview];
                }
            }
            let moving: bool = msg_send![this, isDecelerating];
            let at: CGPoint = msg_send![pan, locationInView: std::ptr::null::<AnyObject>()];
            let state = CURRENT.with(|current| current.borrow().clone());
            let mut found = "kept";
            if let Some(state) = state {
                let driver = (state.at)(at.x as f32, at.y as f32);
                let same = {
                    let current = state.driver.borrow();
                    match (&*current, &driver) {
                        (Some(current), Some(driver)) => current.same(driver),
                        _ => false,
                    }
                };
                // Coasting, the same view keeps its speed for the finger to catch.
                // Anything else is another view's, or a page's now gone: left as it
                // was, this finger would move nothing until it lifted.
                if !moving || !same {
                    match driver {
                        Some(driver) => {
                            found = "scrolls";
                            configure(&state, this, driver, Some(at));
                        }
                        None => {
                            found = "nothing";
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
            let begins: Bool =
                msg_send![super(this, class!(UIScrollView)), gestureRecognizerShouldBegin: pan];
            crate::ios_gestures::log(&format!(
                "scroll should_begin at={:.0},{:.0} coasting={moving} {found} -> {}",
                at.x,
                at.y,
                begins.as_bool()
            ));
            begins
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

/// Give the scroller the system's refresh control where what's under the finger
/// has a pull to refresh, hung from where the app says.
fn configure_pull(state: &State, view: &AnyObject, at: Option<CGPoint>, frame: &mut CGRect) {
    let pull = at.and_then(|at| (state.pull_at)(at.x as f32, at.y as f32));
    crate::ios_gestures::log(&format!("scroll pull here: {}", pull.is_some()));
    // SAFETY: UIKit's constructors and setters, on the main thread.
    unsafe {
        match &pull {
            Some(pull) => {
                let mut control = state.control.borrow_mut();
                let made = control.get_or_insert_with(|| {
                    let made: Retained<AnyObject> = msg_send![class!(UIRefreshControl), new];
                    if let Some(delegate) = &*state.delegate.borrow() {
                        // UIControlEventValueChanged.
                        let _: () = msg_send![&*made,
                            addTarget: &**delegate,
                            action: sel!(refreshed:),
                            forControlEvents: 4096usize];
                    }
                    made
                });
                let on: Option<Retained<AnyObject>> = msg_send![view, refreshControl];
                if on.is_none() {
                    let _: () = msg_send![view, setRefreshControl: &**made];
                }
                frame.origin.y = f64::from((pull.top)());
            }
            None => {
                let refreshing = state.control.borrow().as_ref().is_some_and(|control| {
                    let refreshing: bool = msg_send![&**control, isRefreshing];
                    refreshing
                });
                if !refreshing {
                    let none: *const AnyObject = std::ptr::null();
                    let _: () = msg_send![view, setRefreshControl: none];
                }
            }
        }
    }
    *state.pull.borrow_mut() = pull;
}

fn configure(state: &State, view: &AnyObject, driver: ScrollDriver, at: Option<CGPoint>) {
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
    let mut frame = CGRect {
        origin: CGPoint { x: 0., y: 0. },
        size,
    };
    configure_pull(state, view, at, &mut frame);
    // SAFETY: UIScrollView's setters, on the main thread.
    unsafe {
        let _: () = msg_send![view, setFrame: frame];
        // A short list gives under a finger, as a table view does.
        let _: () = msg_send![view, setAlwaysBounceVertical: !metrics.horizontal];
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
        stretched: impl Fn(f32) + 'static,
        pull_at: impl Fn(f32, f32) -> Option<zgx::native_control::PullRefresh> + 'static,
    ) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
            return None;
        };
        let state = Rc::new(State {
            at: Box::new(at),
            stretched: Box::new(stretched),
            driver: RefCell::default(),
            top: RefCell::new(Box::new(top)),
            configuring: Default::default(),
            drag_began: std::cell::Cell::new(false),
            refresh_ending: Rc::default(),
            dragging: std::cell::Cell::new(false),
            pull_at: Box::new(pull_at),
            pull: RefCell::default(),
            delegate: RefCell::default(),
            control: RefCell::default(),
            window: window.clone(),
        });
        CURRENT.with(|current| *current.borrow_mut() = Some(state.clone()));
        let delegate = Delegate::alloc(mtm).set_ivars(state.clone());
        // SAFETY: NSObject's `init`, on a fresh allocation with its ivars set.
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        *state.delegate.borrow_mut() = Some(delegate.clone());
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
            // Above the base plane's layer, where a refresh control shows; it draws
            // nothing else, and no touch lands on it.
            let root_layer: Retained<AnyObject> = msg_send![&*root, layer];
            let own: Retained<AnyObject> = msg_send![&*view, layer];
            let _: () = msg_send![&*root_layer, addSublayer: &*own];
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
                    configure(&state, &view, driver, None);
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
    pub(crate) fn finger_down(&self, driver: Option<ScrollDriver>, at: (f32, f32)) {
        // SAFETY: UIScrollView's getters and setters, on the main thread.
        unsafe {
            let moving: bool = msg_send![&*self.view, isDecelerating];
            let tracking: bool = msg_send![&*self.view, isTracking];
            let same = match (&*self.state.driver.borrow(), &driver) {
                (Some(current), Some(driver)) => current.same(driver),
                _ => false,
            };
            // A finger on what is coasting catches it. What else is coasting has
            // no claim on this finger (`should_begin`).
            if tracking || (moving && same) {
                return;
            }
            match driver {
                Some(driver) => {
                    let at = CGPoint {
                        x: f64::from(at.0),
                        y: f64::from(at.1),
                    };
                    configure(&self.state, &self.view, driver, Some(at));
                }
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
    /// Its pan, for what must go before it.
    pub(crate) fn pan(&self) -> Retained<AnyObject> {
        // SAFETY: UIScrollView's getter, on the main thread.
        unsafe { msg_send![&*self.view, panGestureRecognizer] }
    }
    /// Whether a finger began dragging this scroller since it last was asked.
    pub(crate) fn take_drag_began(&self) -> bool {
        self.state.drag_began.take()
    }
    /// Whether UIKit tracks scrolling for this finger: zgx offers it no pan.
    pub(crate) fn tracks(&self) -> bool {
        self.state.driver.borrow().is_some()
    }
}
