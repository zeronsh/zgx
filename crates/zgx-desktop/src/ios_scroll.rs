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
    /// What moves with the keyboard now, its display link, and the scroller.
    following: RefCell<Option<ScrollDriver>>,
    follow_link: RefCell<Option<Retained<AnyObject>>>,
    scroller: RefCell<Option<Retained<AnyObject>>>,
    /// What tracks the keyboard's edge: a view on `keyboardLayoutGuide`, which
    /// UIKit moves with the keyboard, a finger's drag of it too.
    tracker: RefCell<Option<Retained<AnyObject>>>,
    keyboard_link: RefCell<Option<Retained<AnyObject>>>,
    /// The page's relation to the keyboard's edge, while there is one.
    lock: RefCell<Option<KeyboardLock>>,
    /// The page moving with the keyboard in the render server, no redraws.
    glue: RefCell<Option<Glue>>,
    /// What waits for the frame being drawn, to change in its transaction.
    after_frame: RefCell<Option<AfterFrame>>,
    /// The bar going with the keyboard, if the app has one.
    bar: RefCell<Option<Bar>>,
    /// The bar moving in the render server, as `glue` for the page.
    bar_glue: RefCell<Option<(f32, f32)>>,
    bar_after_frame: RefCell<Option<AfterFrame>>,
    /// What must show above the keyboard: the app's latest word.
    field: RefCell<Option<(ScrollDriver, f32)>>,
    /// The edge last frame: where, when, and for how many frames it hasn't moved.
    edge_seen: std::cell::Cell<(f64, f64, u32)>,
    /// The room below the content while the keyboard is up.
    bottom_inset: std::cell::Cell<f64>,
    /// The scroll view last moved for the keyboard, while it holds room for it.
    inset_on: RefCell<Option<ScrollDriver>>,
    /// Whether a finger is down, and whether a refresh ended under it.
    refresh_ending: Rc<(std::cell::Cell<bool>, std::cell::Cell<bool>)>,
    /// A finger is dragging: UIKit's `isDragging` stays false, its pan elsewhere.
    dragging: std::cell::Cell<bool>,
    /// Finds the pull to refresh at a window point, and the one in play.
    pull_at: Box<dyn Fn(f32, f32) -> Option<zgx::native_control::PullRefresh>>,
    pull: RefCell<Option<zgx::native_control::PullRefresh>>,
    /// The delegate, which a `UIRefreshControl` tells.
    delegate: RefCell<Option<Retained<Delegate>>>,
    /// The scroller's control, kept whether or not it is on the scroller now.
    control: RefCell<Option<Retained<AnyObject>>>,
}

/// How a page moves with the keyboard's edge.
enum KeyboardLock {
    /// A field's bottom, at `bottom` with the page at `offset`, stays this far
    /// above the edge once the edge reaches it; the page is at `offset` before.
    Show {
        driver: ScrollDriver,
        offset: f32,
        bottom: f32,
    },
    /// The keyboard goes from `top` to `end`, the page in step from `from` to `to`.
    Hide {
        driver: ScrollDriver,
        from: f32,
        to: f32,
        top: f64,
        end: f64,
    },
}

/// A field's distance above the keyboard, as UIKit's own leaves about.
const KEYBOARD_ROOM: f32 = 28.;
/// Frames the edge must rest before the display link stops.
const EDGE_REST_FRAMES: u32 = 8;

/// The keyboard is about to move (`UIKeyboardWillChangeFrame`): `covered` is what
/// it will cover. Lock the page to its edge, from this very frame.
pub(crate) fn keyboard_changing(covered: f32) {
    let Some(state) = CURRENT.with(|current| current.borrow().clone()) else {
        return;
    };
    let shows_lock = state.lock.borrow().is_some();
    if covered > 0. {
        state.bottom_inset.set(f64::from(covered));
        if !shows_lock {
            let field = state.field.borrow().clone();
            if let Some((driver, bottom)) = field
                && let Some(metrics) = driver.metrics()
            {
                driver.set_end_room(covered);
                *state.inset_on.borrow_mut() = Some(driver.clone());
                *state.following.borrow_mut() = Some(driver.clone());
                *state.lock.borrow_mut() = Some(KeyboardLock::Show {
                    driver,
                    offset: metrics.offset + metrics.stretch,
                    bottom,
                });
            }
        }
    } else if !shows_lock {
        // A finger moved the page while it was up: back within its content, in
        // step, from wherever it is.
        let driver = state.inset_on.borrow().clone();
        if let Some(driver) = driver
            && let Some(metrics) = driver.metrics()
        {
            let from = metrics.offset + metrics.stretch;
            let top = keyboard_edge(&state, false).unwrap_or(0.);
            let end = root_height(&state);
            *state.following.borrow_mut() = Some(driver.clone());
            *state.lock.borrow_mut() = Some(KeyboardLock::Hide {
                driver,
                from,
                to: from.min((metrics.extent.1 - metrics.size.1).max(0.)),
                top,
                end,
            });
        }
    }
    glue_to_keyboard(&state);
    start_keyboard_link(&state);
    crate::ios_gestures::log(&format!("keyboard will cover {covered:.0}"));
}

/// The page's way to the keyboard's new place, done by the render server: an
/// animation of the view holding the content, from the keyboard's own spring and
/// on its clock, so the two move as one with nothing drawn. What the app paints
/// above the edge effect is not in that view and stays. Without an edge effect
/// all the app is one layer, its bars too: the page follows by frames instead.
struct Glue {
    driver: ScrollDriver,
    /// The offset drawn while this runs.
    drawn: f32,
    /// The offset to draw once it has.
    target: f32,
}
/// Told how far up a keyboard bar is to be drawn.
pub(crate) type BarLift = Rc<dyn Fn(f32)>;
/// A bar going with the keyboard: where it rests, and how far up it is drawn.
struct Bar {
    lift: BarLift,
    bottom: f32,
    lifted: f32,
}
/// How far above the keyboard a bar stays.
pub(crate) const BAR_GAP: f32 = 8.;
impl Bar {
    fn up_for(&self, edge: f64) -> f32 {
        (f64::from(self.bottom + BAR_GAP) - edge).max(0.) as f32
    }
    fn draw_at(&mut self, up: f32) {
        self.lifted = up;
        (self.lift)(up);
    }
}
enum AfterFrame {
    /// The animation goes: the frame shows what it showed.
    Remove,
    /// The animation starts: the frame was drawn for it.
    Start(Vec<f64>),
}
const GLUE_KEY: &str = "zgxKeyboard";
/// Steps of the sampled spring, per second: the display's fastest.
const GLUE_RATE: f64 = 120.;

/// `layer`'s one animation, if it is a spring on a position: the keyboard's.
///
/// # Safety
/// On the main thread.
unsafe fn keyboard_spring(layer: &AnyObject) -> Option<KeyboardSpring> {
    // SAFETY: CALayer's and CASpringAnimation's getters, as the caller promises.
    unsafe {
        let keys: Option<Retained<AnyObject>> = msg_send![layer, animationKeys];
        let keys = keys?;
        let count: usize = msg_send![&*keys, count];
        if count != 1 {
            return None;
        }
        let key: Retained<AnyObject> = msg_send![&*keys, objectAtIndex: 0usize];
        let animation: Option<Retained<AnyObject>> = msg_send![layer, animationForKey: &*key];
        let animation = animation?;
        let spring: bool = msg_send![&*animation, isKindOfClass: class!(CASpringAnimation)];
        if !spring {
            return None;
        }
        let from: Option<Retained<AnyObject>> = msg_send![&*animation, fromValue];
        let from: CGPoint = msg_send![&*from?, CGPointValue];
        Some(KeyboardSpring {
            from: from.y,
            mass: msg_send![&*animation, mass],
            stiffness: msg_send![&*animation, stiffness],
            damping: msg_send![&*animation, damping],
            velocity: msg_send![&*animation, initialVelocity],
            duration: msg_send![&*animation, duration],
        })
    }
}
#[derive(Clone, Copy)]
struct KeyboardSpring {
    /// How far from its end it starts: it is added to where the layer rests.
    from: f64,
    mass: f64,
    stiffness: f64,
    damping: f64,
    velocity: f64,
    duration: f64,
}
impl KeyboardSpring {
    /// The share of `from` left, every `1 / GLUE_RATE` of a second to its end.
    fn remaining(&self) -> Vec<f64> {
        const SUBSTEPS: usize = 16;
        let steps = (self.duration * GLUE_RATE).ceil().max(1.) as usize;
        let dt = self.duration / steps as f64 / SUBSTEPS as f64;
        let (mut x, mut v) = (1f64, -self.velocity);
        let mut out = Vec::with_capacity(steps + 1);
        out.push(x);
        for _ in 0..steps {
            for _ in 0..SUBSTEPS {
                v += (-self.damping * v - self.stiffness * x) / self.mass * dt;
                x += v * dt;
            }
            out.push(x);
        }
        out
    }
}

/// The begin time the keyboard's animation has, once its transaction committed.
///
/// # Safety
/// On the main thread.
unsafe fn keyboard_began(state: &State) -> f64 {
    let tracker = state.tracker.borrow();
    let Some(tracker) = &*tracker else {
        return 0.;
    };
    // SAFETY: CALayer's getters, as the caller promises.
    unsafe {
        let layer: Retained<AnyObject> = msg_send![&**tracker, layer];
        let keys: Option<Retained<AnyObject>> = msg_send![&*layer, animationKeys];
        let Some(keys) = keys else { return 0. };
        let first: Option<Retained<AnyObject>> = msg_send![&*keys, firstObject];
        let Some(first) = first else { return 0. };
        let animation: Option<Retained<AnyObject>> = msg_send![&*layer, animationForKey: &*first];
        animation.map_or(0., |animation| msg_send![&*animation, beginTime])
    }
}

fn page_views() -> Vec<Retained<AnyObject>> {
    crate::native_host::ios::held_view().into_iter().collect()
}

/// Run `values` on the layers of `views`, over the keyboard's duration.
fn start_glue(views: &[Retained<AnyObject>], values: &[f64], duration: f64, begin: f64) {
    // SAFETY: Core Animation's constructors and setters, on the main thread.
    unsafe {
        let path = crate::ios_system::objc2_foundation_string("transform.translation.y");
        let animation: Retained<AnyObject> =
            msg_send![class!(CAKeyframeAnimation), animationWithKeyPath: &*path];
        let numbers: Retained<AnyObject> = msg_send![class!(NSMutableArray), array];
        for value in values {
            let number: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithDouble: *value];
            let _: () = msg_send![&*numbers, addObject: &*number];
        }
        let _: () = msg_send![&*animation, setValues: &*numbers];
        let _: () = msg_send![&*animation, setDuration: duration];
        // Held after its end, and before its start if it starts late: it is on
        // the keyboard's clock.
        let both = crate::ios_system::objc2_foundation_string("both");
        let _: () = msg_send![&*animation, setFillMode: &*both];
        let _: () = msg_send![&*animation, setRemovedOnCompletion: false];
        let _: () = msg_send![&*animation, setBeginTime: begin];
        let key = crate::ios_system::objc2_foundation_string(GLUE_KEY);
        for view in views {
            let layer: Retained<AnyObject> = msg_send![&**view, layer];
            let _: () = msg_send![&*layer, addAnimation: &*animation, forKey: &*key];
        }
    }
}

fn remove_glue(views: &[Retained<AnyObject>]) {
    // SAFETY: CALayer's method, on the main thread.
    unsafe {
        let key = crate::ios_system::objc2_foundation_string(GLUE_KEY);
        for view in views {
            let layer: Retained<AnyObject> = msg_send![&**view, layer];
            let _: () = msg_send![&*layer, removeAnimationForKey: &*key];
        }
    }
}

/// How far up `views`' first layer shows, as its animation has it now.
fn shown_translation(views: &[Retained<AnyObject>]) -> f64 {
    let Some(view) = views.first() else {
        return 0.;
    };
    // SAFETY: CALayer's getters and key-value coding, on the main thread.
    unsafe {
        let layer: Retained<AnyObject> = msg_send![&**view, layer];
        let shown: Option<Retained<AnyObject>> = msg_send![&*layer, presentationLayer];
        let Some(shown) = shown else { return 0. };
        let path = crate::ios_system::objc2_foundation_string("transform.translation.y");
        let value: Option<Retained<AnyObject>> = msg_send![&*shown, valueForKeyPath: &*path];
        value.map_or(0., |value| {
            let value: f64 = msg_send![&*value, doubleValue];
            value
        })
    }
}

/// The bar where it shows now, its animation stopped.
fn unglue_bar(state: &State) {
    let Some((drawn, _)) = state.bar_glue.borrow_mut().take() else {
        return;
    };
    let shown = shown_translation(&crate::native_host::ios::bar_views());
    if let Some(bar) = &mut *state.bar.borrow_mut() {
        bar.draw_at(drawn - shown as f32);
    }
    *state.bar_after_frame.borrow_mut() = Some(AfterFrame::Remove);
    state.window.request_redraw();
}

/// The bar's way to the keyboard's new place: as the page's, on layers of its own.
fn glue_bar(state: &State, spring: KeyboardSpring, end: f64) {
    let mut bar = state.bar.borrow_mut();
    let Some(bar) = &mut *bar else {
        return;
    };
    let (first, last) = (bar.up_for(end + spring.from), bar.up_for(end));
    if (first - last).abs() < 0.5 {
        return;
    }
    // Lifted the less of the two: it is the layer that makes up the rest.
    let drawn = first.min(last);
    let values: Vec<f64> = spring
        .remaining()
        .into_iter()
        .map(|left| f64::from(drawn - bar.up_for(end + spring.from * left)))
        .chain(std::iter::once(f64::from(drawn - last)))
        .collect();
    let duration = spring.duration * (values.len() as f64 - 1.) / (values.len() as f64 - 2.);
    crate::ios_gestures::log(&format!("keyboard bar {first:.0} to {last:.0}"));
    *state.bar_glue.borrow_mut() = Some((drawn, last));
    if (bar.lifted - drawn).abs() < 0.5 {
        start_glue(&crate::native_host::ios::bar_views(), &values, duration, 0.);
    } else {
        bar.draw_at(drawn);
        *state.bar_after_frame.borrow_mut() = Some(AfterFrame::Start(values));
        state.window.request_redraw();
    }
}

/// The page where it shows now, the animation stopped: a finger took it, or the
/// keyboard changed its mind.
fn unglue(state: &State) {
    let Some(glue) = state.glue.borrow_mut().take() else {
        return;
    };
    let shown = shown_translation(&page_views());
    glue.driver.drive(glue.drawn - shown as f32);
    *state.after_frame.borrow_mut() = Some(AfterFrame::Remove);
    state.window.request_redraw();
}

/// Called as the keyboard's animation is made, in its transaction.
fn glue_to_keyboard(state: &State) {
    unglue(state);
    unglue_bar(state);
    let (spring, end) = {
        let tracker = state.tracker.borrow();
        let Some(tracker) = &*tracker else {
            return;
        };
        // SAFETY: UIView's and CALayer's methods, on the main thread: the layout
        // guide's change, with the keyboard's animation, comes out of this.
        let spring = unsafe {
            let parent: Option<Retained<AnyObject>> = msg_send![&**tracker, superview];
            if let Some(parent) = parent {
                let _: () = msg_send![&*parent, layoutIfNeeded];
            }
            let layer: Retained<AnyObject> = msg_send![&**tracker, layer];
            keyboard_spring(&layer)
        };
        (spring, keyboard_edge(state, true))
    };
    let (Some(spring), Some(end)) = (spring, end) else {
        crate::ios_gestures::log("keyboard: no spring to follow, by frames");
        return;
    };
    glue_bar(state, spring, end);
    if crate::native_host::ios::held_view().is_none() {
        return;
    }
    let start = end + spring.from;
    let lock = state.lock.borrow();
    // Where the page belongs for each place of the keyboard's edge.
    let (driver, page): (ScrollDriver, Box<dyn Fn(f64) -> f32>) = match &*lock {
        Some(KeyboardLock::Show {
            driver,
            offset,
            bottom,
        }) => {
            let (offset, bottom) = (*offset, *bottom);
            (
                driver.clone(),
                Box::new(move |edge| {
                    offset + (f64::from(bottom + KEYBOARD_ROOM) - edge).max(0.) as f32
                }),
            )
        }
        Some(KeyboardLock::Hide {
            driver, from, to, ..
        }) => {
            let (from, to) = (*from, *to);
            (
                driver.clone(),
                Box::new(move |edge| (from - (edge - start) as f32).max(to)),
            )
        }
        None => return,
    };
    let (first, last) = (page(start), page(end));
    if (first - last).abs() < 0.5 {
        return;
    }
    // Drawn with the page the lower of the two: what the move uncovers is then
    // below the content drawn, behind the keyboard.
    let drawn = first.min(last);
    let values: Vec<f64> = spring
        .remaining()
        .into_iter()
        .map(|left| f64::from(drawn - page(end + spring.from * left)))
        .chain(std::iter::once(f64::from(drawn - last)))
        .collect();
    let duration = spring.duration * (values.len() as f64 - 1.) / (values.len() as f64 - 2.);
    crate::ios_gestures::log(&format!(
        "keyboard glue {first:.0} to {last:.0}, drawn {drawn:.0}, spring {:.0} over {:.3}s",
        spring.from, spring.duration
    ));
    *state.glue.borrow_mut() = Some(Glue {
        driver: driver.clone(),
        drawn,
        target: last,
    });
    if (drawn - first).abs() < 0.5 {
        // In this transaction, so on the keyboard's clock with no begin time.
        start_glue(&page_views(), &values, duration, 0.);
    } else {
        driver.drive(drawn);
        *state.after_frame.borrow_mut() = Some(AfterFrame::Start(values));
        state.window.request_redraw();
    }
}

/// The frame just presented shows in this transaction: what waited for it, too.
pub(crate) fn frame_done() {
    let Some(state) = CURRENT.with(|current| current.borrow().clone()) else {
        return;
    };
    let waiting = state.after_frame.borrow_mut().take();
    run_after_frame(&state, waiting, &page_views());
    let waiting = state.bar_after_frame.borrow_mut().take();
    run_after_frame(&state, waiting, &crate::native_host::ios::bar_views());
}

fn run_after_frame(state: &State, waiting: Option<AfterFrame>, views: &[Retained<AnyObject>]) {
    match waiting {
        Some(AfterFrame::Remove) => remove_glue(views),
        Some(AfterFrame::Start(values)) => {
            // Its begin time once committed; not yet, then this joins its commit
            // and takes the same one.
            // SAFETY: Core Animation's getters, on the main thread.
            let begin = unsafe { keyboard_began(state) };
            let duration = state
                .tracker
                .borrow()
                .as_ref()
                .and_then(|tracker| {
                    // SAFETY: as above.
                    unsafe {
                        let layer: Retained<AnyObject> = msg_send![&**tracker, layer];
                        keyboard_spring(&layer)
                    }
                })
                .map_or(0.4, |spring| spring.duration);
            start_glue(
                views,
                &values,
                duration * (values.len() as f64 - 1.) / (values.len() as f64 - 2.),
                begin,
            );
        }
        None => {}
    }
}

fn root_height(state: &State) -> f64 {
    let tracker = state.tracker.borrow();
    let Some(tracker) = &*tracker else {
        return 0.;
    };
    // SAFETY: UIView's getters, on the main thread.
    unsafe {
        let parent: Option<Retained<AnyObject>> = msg_send![&**tracker, superview];
        let Some(parent) = parent else { return 0. };
        let bounds: CGRect = msg_send![&*parent, bounds];
        bounds.size.height
    }
}

/// Where the keyboard's top is now, in the window's points: as drawn, or (`model`)
/// where it is going.
fn keyboard_edge(state: &State, model: bool) -> Option<f64> {
    let tracker = state.tracker.borrow();
    let tracker = tracker.as_ref()?;
    // SAFETY: CALayer's getters, on the main thread.
    unsafe {
        let layer: Retained<AnyObject> = msg_send![&**tracker, layer];
        let shown: Option<Retained<AnyObject>> = if model {
            None
        } else {
            msg_send![&*layer, presentationLayer]
        };
        let frame: CGRect = match shown {
            Some(shown) => msg_send![&*shown, frame],
            None => msg_send![&*layer, frame],
        };
        Some(frame.origin.y + frame.size.height)
    }
}

fn start_keyboard_link(state: &State) {
    if state.keyboard_link.borrow().is_some() {
        state.edge_seen.set((state.edge_seen.get().0, 0., 0));
        return;
    }
    let delegate = state.delegate.borrow();
    let Some(delegate) = &*delegate else { return };
    state.edge_seen.set((-1., 0., 0));
    // SAFETY: Core Animation's constructor and methods, on the main thread; the
    // run loop keeps the link until it is invalidated.
    unsafe {
        let link: Retained<AnyObject> = msg_send![class!(CADisplayLink),
            displayLinkWithTarget: &**delegate,
            selector: sel!(keyboardTick:)];
        let _: () = msg_send![&*link, setPreferredFrameRateRange: crate::ios_refresh::FAST];
        let run_loop: Retained<AnyObject> = msg_send![class!(NSRunLoop), mainRunLoop];
        let _: () = msg_send![&*link, addToRunLoop: &*run_loop, forMode: NSRunLoopCommonModes];
        *state.keyboard_link.borrow_mut() = Some(link);
    }
}

/// A frame of the page locked to the keyboard's edge.
fn keyboard_frame(state: &State, link: &AnyObject) {
    let Some(edge) = keyboard_edge(state, false) else {
        return;
    };
    // As drawn now, unpredicted: measured from video, leading it by the display
    // link's next frame changed nothing but the risk of overshooting.
    let _ = link;
    let (last_edge, _, rested) = state.edge_seen.get();
    let moving = last_edge >= 0. && (edge - last_edge).abs() > 0.05;
    let at_target = edge;
    state
        .edge_seen
        .set((edge, 0., if moving { 0 } else { rested + 1 }));
    // A bar not on the render server's clock (a finger has the keyboard): by frames.
    if state.bar_glue.borrow().is_none()
        && let Some(bar) = &mut *state.bar.borrow_mut()
    {
        let up = bar.up_for(edge);
        if (up - bar.lifted).abs() >= 0.25 {
            bar.draw_at(up);
            state.window.request_redraw();
        }
    }
    if let Some((drawn, _)) = *state.bar_glue.borrow()
        && let Some(bar) = &*state.bar.borrow()
    {
        let moved = shown_translation(&crate::native_host::ios::bar_views());
        let bottom = f64::from(bar.bottom - drawn) + moved;
        crate::ios_gestures::log(&format!("bar gap {:.1} edge {edge:.1}", edge - bottom));
    }
    let glued = state.glue.borrow().is_some();
    if glued {
        crate::ios_gestures::log_follow_edge(edge, edge, 0.);
    }
    let lock = state.lock.borrow();
    match &*lock {
        // The render server has it, until it lands.
        _ if glued => {}
        Some(KeyboardLock::Show {
            driver,
            offset,
            bottom,
        }) => {
            let need = (f64::from(*bottom + KEYBOARD_ROOM) - at_target).max(0.);
            driver.drive(*offset + need as f32);
            crate::ios_gestures::log_follow_edge(edge, at_target, *offset as f64 + need);
            state.window.request_redraw();
        }
        Some(KeyboardLock::Hide {
            driver,
            from,
            to,
            top,
            end,
        }) => {
            let gone = if end > top {
                ((at_target - top) / (end - top)).clamp(0., 1.)
            } else {
                1.
            };
            driver.drive(*from + (*to - *from) * gone as f32);
            state.window.request_redraw();
        }
        None => {}
    }
    drop(lock);
    // At rest, and no finger that could move it: nothing awake until it moves.
    if rested + 1 >= EDGE_REST_FRAMES && !state.dragging.get() {
        finish_keyboard_move(state, edge);
    }
}

/// The edge came to rest: the scroller takes the page from where it was left.
fn finish_keyboard_move(state: &State, edge: f64) {
    if let Some((drawn, target)) = state.bar_glue.borrow_mut().take() {
        if (target - drawn).abs() < 0.5 {
            remove_glue(&crate::native_host::ios::bar_views());
        } else {
            if let Some(bar) = &mut *state.bar.borrow_mut() {
                bar.draw_at(target);
            }
            *state.bar_after_frame.borrow_mut() = Some(AfterFrame::Remove);
            state.window.request_redraw();
        }
    }
    if let Some(glue) = state.glue.borrow_mut().take() {
        // The new offset's frame and the animation's end, in one transaction.
        if (glue.target - glue.drawn).abs() < 0.5 {
            remove_glue(&page_views());
        } else {
            glue.driver.drive(glue.target);
            *state.after_frame.borrow_mut() = Some(AfterFrame::Remove);
            state.window.request_redraw();
        }
    }
    if let Some(link) = state.keyboard_link.borrow_mut().take() {
        // SAFETY: CADisplayLink's method, on the main thread.
        let _: () = unsafe { msg_send![&*link, invalidate] };
    }
    let gone = edge >= root_height(state) - 0.5;
    let held = matches!(&*state.lock.borrow(), Some(KeyboardLock::Show { .. }));
    if gone {
        state.lock.borrow_mut().take();
        state.following.borrow_mut().take();
        if let Some(driver) = state.inset_on.borrow_mut().take() {
            driver.set_end_room(0.);
        }
        state.bottom_inset.set(0.);
    } else if !held {
        state.lock.borrow_mut().take();
        state.following.borrow_mut().take();
    }
    if let Some(view) = &*state.scroller.borrow() {
        // SAFETY: UIScrollView's setter, on the main thread.
        let _: () = unsafe {
            msg_send![&**view, setContentInset: UiEdgeInsets(0., 0., state.bottom_inset.get(), 0.)]
        };
    }
    crate::ios_gestures::log(&format!("keyboard edge rests at {edge:.0}"));
}

/// A view UIKit keeps on the keyboard's top edge.
///
/// # Safety
/// `root` is the window's view, on the main thread.
unsafe fn install_tracker(state: &State, root: *const AnyObject) {
    // SAFETY: UIKit's constructors and Auto Layout's methods, as the caller promises.
    unsafe {
        let tracker: Retained<AnyObject> = msg_send![class!(UIView), new];
        let _: () = msg_send![&*tracker, setHidden: true];
        let _: () = msg_send![&*tracker, setUserInteractionEnabled: false];
        let _: () = msg_send![&*tracker, setAccessibilityElementsHidden: true];
        let _: () = msg_send![&*tracker, setTranslatesAutoresizingMaskIntoConstraints: false];
        let _: () = msg_send![&*root, addSubview: &*tracker];
        let guide: Retained<AnyObject> = msg_send![&*root, keyboardLayoutGuide];
        let _: () = msg_send![&*guide, setFollowsUndockedKeyboard: false];
        let _: () = msg_send![&*guide, setUsesBottomSafeArea: false];
        let tracker_bottom: Retained<AnyObject> = msg_send![&*tracker, bottomAnchor];
        let guide_top: Retained<AnyObject> = msg_send![&*guide, topAnchor];
        let on_edge: Retained<AnyObject> =
            msg_send![&*tracker_bottom, constraintEqualToAnchor: &*guide_top];
        let height: Retained<AnyObject> = msg_send![&*tracker, heightAnchor];
        let one: Retained<AnyObject> = msg_send![&*height, constraintEqualToConstant: 1f64];
        let left: Retained<AnyObject> = msg_send![&*tracker, leftAnchor];
        let root_left: Retained<AnyObject> = msg_send![&*root, leftAnchor];
        let at_left: Retained<AnyObject> = msg_send![&*left, constraintEqualToAnchor: &*root_left];
        let width: Retained<AnyObject> = msg_send![&*tracker, widthAnchor];
        let wide: Retained<AnyObject> = msg_send![&*width, constraintEqualToConstant: 1f64];
        for constraint in [&on_edge, &one, &at_left, &wide] {
            let _: () = msg_send![&**constraint, setActive: true];
        }
        *state.tracker.borrow_mut() = Some(tracker);
    }
}

/// Leave a move with the keyboard where it is: a finger took over.
fn stop_following(state: &State, view: &AnyObject) {
    if state.following.borrow_mut().take().is_none() {
        return;
    }
    // SAFETY: CALayer's and CADisplayLink's methods, on the main thread.
    unsafe {
        let layer: Retained<AnyObject> = msg_send![view, layer];
        let _: () = msg_send![&*layer, removeAllAnimations];
        if let Some(link) = state.follow_link.borrow_mut().take() {
            let _: () = msg_send![&*link, invalidate];
        }
    }
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

/// `UIEdgeInsets`: top, left, bottom, right.
#[repr(C)]
#[derive(Clone, Copy)]
struct UiEdgeInsets(f64, f64, f64, f64);
// SAFETY: four CGFloats, as UIEdgeInsets is declared on a 64-bit platform.
unsafe impl objc2::Encode for UiEdgeInsets {
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
        "UIEdgeInsets",
        &[
            objc2::Encoding::Double,
            objc2::Encoding::Double,
            objc2::Encoding::Double,
            objc2::Encoding::Double,
        ],
    );
}
const NO_INSETS: UiEdgeInsets = UiEdgeInsets(0., 0., 0., 0.);

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
        // Each frame of a move with the keyboard: where UIKit's animation has it.
        #[unsafe(method(followTick:))]
        fn follow_tick(&self, _link: &AnyObject) {
            let state = self.ivars();
            let driver = state.following.borrow().clone();
            let scroller = state.scroller.borrow().clone();
            if let (Some(driver), Some(scroller)) = (driver, scroller) {
                // SAFETY: CALayer's getters, on the main thread.
                let at: CGRect = unsafe {
                    let layer: Retained<AnyObject> = msg_send![&*scroller, layer];
                    let shown: Option<Retained<AnyObject>> = msg_send![&*layer, presentationLayer];
                    match shown {
                        Some(shown) => msg_send![&*shown, bounds],
                        None => msg_send![&*layer, bounds],
                    }
                };
                driver.drive(at.origin.y as f32);
                crate::ios_gestures::log_follow(at.origin.y);
                state.window.request_redraw();
            }
        }
        // Each frame the keyboard may be moving: the page, locked to its edge.
        #[unsafe(method(keyboardTick:))]
        fn keyboard_tick(&self, link: &AnyObject) {
            keyboard_frame(self.ivars(), link);
        }
        #[unsafe(method(scrollViewDidScroll:))]
        fn did_scroll(&self, view: &AnyObject) {
            let state = self.ivars();
            if state.configuring.get() || state.following.borrow().is_some() {
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
            stop_following(state, view);
            unglue(state);
            state.lock.borrow_mut().take();
            state.following.borrow_mut().take();
            start_keyboard_link(state);
            state.dragging.set(true);
            state.refresh_ending.0.set(true);
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
    // Where it shows: past its end, too, above a keyboard.
    let shown = f64::from(metrics.offset + metrics.stretch);
    let (offset_x, offset_y) = if metrics.horizontal {
        (shown, 0.)
    } else {
        (0., shown)
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
        let inset = UiEdgeInsets(0., 0., state.bottom_inset.get(), 0.);
        let _: () = msg_send![view, setContentInset: inset];
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
            following: RefCell::default(),
            follow_link: RefCell::default(),
            scroller: RefCell::default(),
            inset_on: RefCell::default(),
            tracker: RefCell::default(),
            keyboard_link: RefCell::default(),
            lock: RefCell::default(),
            field: RefCell::default(),
            edge_seen: std::cell::Cell::new((0., 0., 0)),
            glue: RefCell::new(None),
            after_frame: RefCell::new(None),
            bar: RefCell::new(None),
            bar_glue: RefCell::new(None),
            bar_after_frame: RefCell::new(None),
            bottom_inset: std::cell::Cell::new(0.),
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
            // The keyboard leaves with a finger, as in Messages.
            let _: () = msg_send![&*view, setKeyboardDismissMode: 2isize];
            install_tracker(&state, root);
            *state.scroller.borrow_mut() = Some(view.clone());
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
            // UIKit may call it tracking before this hears of the touch: with
            // nothing configured yet, this finger is still to be given a driver.
            let held = self.state.driver.borrow().is_some();
            if (tracking && held) || (moving && same) {
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
    /// Move `driver`'s scroll view to `to` as the keyboard moves: UIKit animates
    /// the scroller with its curve, in the same frames, and each frame zgx draws
    /// where that has got to, past its content's end if it must be. `inset` is
    /// the room that keeps a finger from taking it back there.
    pub(crate) fn follow_keyboard(&self, driver: ScrollDriver, to: f32, inset: Option<f32>) {
        let state = &self.state;
        stop_following(state, &self.view);
        let (mut duration, curve) = crate::ios_system::keyboard_motion();
        if duration <= 0. {
            // The keyboard isn't moving (focus moved under it): UIKit's own default.
            duration = 0.25;
        }
        if let Some(inset) = inset {
            state.bottom_inset.set(f64::from(inset));
            driver.set_end_room(inset);
            *state.inset_on.borrow_mut() = Some(driver.clone());
        }
        configure(state, &self.view, driver.clone(), None);
        *state.following.borrow_mut() = Some(driver.clone());
        *state.scroller.borrow_mut() = Some(self.view.clone());
        let view = self.view.clone();
        let change = block2::RcBlock::new(move || {
            // SAFETY: UIScrollView's setter, in UIKit's animation block.
            let _: () =
                unsafe { msg_send![&*view, setContentOffset: CGPoint { x: 0., y: f64::from(to) }] };
        });
        let done_state = state.clone();
        let done_view = self.view.clone();
        let done_driver = driver.clone();
        let done = block2::RcBlock::new(move |_finished: Bool| {
            let still = done_state
                .following
                .borrow()
                .as_ref()
                .is_some_and(|following| following.same(&done_driver));
            if !still {
                return;
            }
            done_driver.drive(to);
            stop_following(&done_state, &done_view);
            if inset.is_none() {
                if let Some(driver) = done_state.inset_on.borrow_mut().take() {
                    driver.set_end_room(0.);
                }
                done_state.bottom_inset.set(0.);
                // SAFETY: UIScrollView's setter, on the main thread.
                let _: () = unsafe { msg_send![&*done_view, setContentInset: NO_INSETS] };
            }
            crate::ios_gestures::log(&format!("keyboard moved page to {to:.0}"));
            done_state.window.request_redraw();
        });
        // SAFETY: UIKit's and Core Animation's methods, on the main thread; the
        // run loop keeps the link until it is invalidated.
        unsafe {
            // The curve as animation options: a keyboard's isn't one UIKit names.
            let options = (curve as usize) << 16;
            let _: () = msg_send![class!(UIView),
                animateWithDuration: duration,
                delay: 0f64,
                options: options,
                animations: &*change,
                completion: &*done];
            if let Some(delegate) = &*state.delegate.borrow() {
                let link: Retained<AnyObject> = msg_send![class!(CADisplayLink),
                    displayLinkWithTarget: &**delegate,
                    selector: sel!(followTick:)];
                let _: () = msg_send![&*link, setPreferredFrameRateRange: crate::ios_refresh::FAST];
                let run_loop: Retained<AnyObject> = msg_send![class!(NSRunLoop), mainRunLoop];
                let _: () =
                    msg_send![&*link, addToRunLoop: &*run_loop, forMode: NSRunLoopCommonModes];
                *state.follow_link.borrow_mut() = Some(link);
            }
        }
        crate::ios_gestures::log(&format!(
            "keyboard moves page to {to:.0} in {duration:.3} s, curve {curve}"
        ));
    }
    /// What must show above the keyboard: `bottom`, in the window's points, in
    /// `driver`'s scroll view; `None` with no field focused.
    pub(crate) fn set_field(&self, field: Option<(ScrollDriver, f32)>) {
        // While a lock holds, it keeps the place it was given.
        *self.state.field.borrow_mut() = field;
    }
    /// Its pan, for what must go before it.
    pub(crate) fn pan(&self) -> Retained<AnyObject> {
        // SAFETY: UIScrollView's getter, on the main thread.
        unsafe { msg_send![&*self.view, panGestureRecognizer] }
    }
    /// The app's bar going with the keyboard, and where it rests.
    pub(crate) fn set_bar(&self, bar: Option<(BarLift, f32)>) {
        let mut held = self.state.bar.borrow_mut();
        match (bar, &mut *held) {
            // Its box is where it is drawn, lift and all.
            (Some((_, bottom)), Some(held)) => held.bottom = bottom + held.lifted,
            (Some((lift, bottom)), slot) => {
                *slot = Some(Bar {
                    lift,
                    bottom,
                    lifted: 0.,
                });
            }
            (None, slot) => *slot = None,
        }
    }
    /// Whether UIKit tracks scrolling for this finger: zgx offers it no pan.
    pub(crate) fn tracks(&self) -> bool {
        self.state.driver.borrow().is_some()
    }
}
