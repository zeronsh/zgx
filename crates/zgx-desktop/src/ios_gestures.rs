//! Pans iOS recognizes and zgx only draws (`native_control::add_pan`): the edge
//! swipe a navigation stack goes back by, and the drag of a sheet. Every ending
//! a recognizer has reaches the script, so none leaves a page stopped part way:
//! a touch zgx tracked itself could be taken by the scroller with no release.
//! Their states go to `Documents/zgx-gesture.log`, to see what a stuck one did.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, NSObject, NSObjectProtocol},
    sel,
};
use objc2_core_foundation::CGPoint;
use std::{cell::RefCell, io::Write, rc::Rc, sync::Arc, time::Instant};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use zgx::{
    native_control::{PanEvent, PanPhase, PanTarget, pans},
    scene::Scene,
};

thread_local! {
    static LOG: RefCell<Option<(std::fs::File, Instant)>> = const { RefCell::new(None) };
}
/// One line of what the gestures did (`ui.gesture_log` too).
pub(crate) fn log(line: &str) {
    LOG.with(|log| {
        if let Some((file, began)) = log.borrow_mut().as_mut() {
            let _ = writeln!(file, "{:.3} {line}", began.elapsed().as_secs_f64());
        }
    });
    if zgx::env::var_os("ZGX_IOS_GESTURE_LOG").is_some() {
        println!("GESTURE {line}");
    }
}

/// A drawing plane shown, moved or hidden (`ZGX_IOS_PLANE_DEBUG` only: there are
/// many).
pub(crate) fn log_plane(level: u8, shown: Option<zgx::scene::Rect>) {
    if zgx::env::var_os("ZGX_IOS_PLANE_DEBUG").is_some() {
        log(&format!("plane {level} shows {shown:?}"));
    }
}

/// A frame of a page moving with the keyboard, for comparing it with UIKit's own
/// (`ZGX_IOS_GESTURE_LOG` only: it comes each frame).
pub(crate) fn log_follow(y: f64) {
    if zgx::env::var_os("ZGX_IOS_GESTURE_LOG").is_some() {
        println!("GESTURE follow {y:.1}");
    }
}

/// A frame of the page locked to the keyboard's edge (`ZGX_IOS_GESTURE_LOG`).
pub(crate) fn log_follow_edge(edge: f64, predicted: f64, offset: f64) {
    if zgx::env::var_os("ZGX_IOS_GESTURE_LOG").is_some() {
        let _ = predicted;
        // Core Animation's clock, for comparing with a native app's on it.
        unsafe extern "C" {
            fn CACurrentMediaTime() -> f64;
        }
        // SAFETY: a plain clock read.
        let now = unsafe { CACurrentMediaTime() };
        println!("GLIDE {now:.4} {edge:.2} {offset:.2}");
    }
}

struct State {
    scene: Rc<RefCell<Scene>>,
    window: Arc<winit::window::Window>,
    root: Retained<AnyObject>,
    edge: Retained<AnyObject>,
    /// Which pan this gesture is for.
    current: RefCell<Option<PanTarget>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<State>]
    struct Handler;
    unsafe impl NSObjectProtocol for Handler {}
    impl Handler {
        #[unsafe(method(gestureRecognizerShouldBegin:))]
        fn should_begin(&self, recognizer: &AnyObject) -> Bool {
            let state = self.ivars();
            let is_edge = std::ptr::eq(recognizer, &*state.edge);
            // SAFETY: UIGestureRecognizer's method, UIKit calling on the main thread.
            let at: CGPoint = unsafe { msg_send![recognizer, locationInView: &*state.root] };
            let target = {
                let scene = state.scene.borrow();
                let under = (!is_edge).then(|| scene.hit_test_all(at.x as f32, at.y as f32));
                pans().into_iter().rev().find(|target| {
                    if target.node.is_some_and(|node| !scene.contains(node)) {
                        return false;
                    }
                    let here = match (target.node, &under) {
                        (None, None) => true,
                        (Some(node), Some(hits)) => hits.iter().any(|hit| {
                            *hit == node || scene.ancestors(*hit).any(|above| above == node)
                        }),
                        _ => false,
                    };
                    here && target.can_begin.as_ref().is_none_or(|can| can())
                })
            };
            let begins = target.is_some();
            *state.current.borrow_mut() = target;
            log(&format!("should_begin edge={is_edge} at={:.0},{:.0} -> {begins}", at.x, at.y));
            Bool::new(begins)
        }
        // What a touch driver calls, which can't start the system's edge swipe
        // (`ZGX_IOS_GESTURE_TEST`): the script hears what a recognizer would say.
        #[unsafe(method(zgxTestPhase:x:vx:))]
        fn test_pan(&self, phase: isize, x: f64, vx: f64) {
            if zgx::env::var_os("ZGX_IOS_GESTURE_TEST").is_none() {
                return;
            }
            let state = self.ivars();
            if phase == 1 {
                let target = pans()
                    .into_iter()
                    .rev()
                    .find(|target| target.node.is_none() && target.can_begin.as_ref().is_none_or(|can| can()));
                *state.current.borrow_mut() = target;
            }
            let phase = match phase {
                1 => PanPhase::Began,
                2 => PanPhase::Changed,
                3 => PanPhase::Ended,
                _ => PanPhase::Cancelled,
            };
            log(&format!("test {phase:?} x={x:.1} vx={vx:.0}"));
            let target = state.current.borrow().clone();
            if let Some(target) = target {
                (target.on_pan)(PanEvent {
                    phase,
                    x: x as f32,
                    y: 0.,
                    vx: vx as f32,
                    vy: 0.,
                });
            }
            state.window.request_redraw();
        }
        #[unsafe(method(panned:))]
        fn panned(&self, recognizer: &AnyObject) {
            let state = self.ivars();
            // SAFETY: UIPanGestureRecognizer's getters, on the main thread.
            let (phase, moved, speed): (isize, CGPoint, CGPoint) = unsafe {
                (
                    msg_send![recognizer, state],
                    msg_send![recognizer, translationInView: &*state.root],
                    msg_send![recognizer, velocityInView: &*state.root],
                )
            };
            let phase = match phase {
                1 => PanPhase::Began,
                2 => PanPhase::Changed,
                3 => PanPhase::Ended,
                _ => PanPhase::Cancelled,
            };
            if phase != PanPhase::Changed {
                log(&format!(
                    "pan {phase:?} x={:.1} y={:.1} vx={:.0} vy={:.0}",
                    moved.x, moved.y, speed.x, speed.y
                ));
            }
            let target = state.current.borrow().clone();
            if let Some(target) = target {
                (target.on_pan)(PanEvent {
                    phase,
                    x: moved.x as f32,
                    y: moved.y as f32,
                    vx: speed.x as f32,
                    vy: speed.y as f32,
                });
            }
            // What the app does next (a spring's timer) runs as the host's loop
            // turns: told now, not left to whatever comes next.
            zgx::native_view::wake();
            state.window.request_redraw();
        }
    }
);

/// Recognizers over `window`, which `scroller_pan` waits for: a finger at the
/// edge is the swipe's first, as a navigation controller's is.
pub(crate) fn install(
    window: &Arc<winit::window::Window>,
    scene: Rc<RefCell<Scene>>,
    scroller_pan: Option<Retained<AnyObject>>,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit's view, alive with its window.
    let Some(root) = (unsafe { Retained::<AnyObject>::retain(handle.ui_view.as_ptr().cast()) })
    else {
        return;
    };
    LOG.with(|log| {
        *log.borrow_mut() = std::env::var_os("HOME").and_then(|home| {
            let path = std::path::Path::new(&home).join("Documents/zgx-gesture.log");
            // The last run's stays, for what a person saw before relaunching.
            let _ = std::fs::rename(
                &path,
                std::path::Path::new(&home).join("Documents/zgx-gesture.previous.log"),
            );
            Some((std::fs::File::create(path).ok()?, Instant::now()))
        });
    });
    // SAFETY: UIKit's constructors and setters, on the main thread. The handler is
    // forgotten: the window's recognizers call it for the app's life.
    unsafe {
        let edge: Retained<AnyObject> = msg_send![class!(UIScreenEdgePanGestureRecognizer), new];
        // UIRectEdgeLeft.
        let _: () = msg_send![&*edge, setEdges: 2usize];
        let pan: Retained<AnyObject> = msg_send![class!(UIPanGestureRecognizer), new];
        let handler = Handler::alloc(mtm).set_ivars(Rc::new(State {
            scene,
            window: window.clone(),
            root: root.clone(),
            edge: edge.clone(),
            current: RefCell::new(None),
        }));
        let handler: Retained<Handler> = msg_send![super(handler), init];
        for recognizer in [&edge, &pan] {
            let _: () = msg_send![&**recognizer, addTarget: &*handler, action: sel!(panned:)];
            let _: () = msg_send![&**recognizer, setDelegate: &*handler];
            let _: () = msg_send![&*root, addGestureRecognizer: &**recognizer];
            if let Some(scroller) = &scroller_pan {
                let _: () = msg_send![&**scroller, requireGestureRecognizerToFail: &**recognizer];
            }
        }
        std::mem::forget(handler);
    }
}
