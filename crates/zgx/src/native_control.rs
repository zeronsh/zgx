//! Controls that are the system's own where exactness needs them: iOS's
//! `UISwitch`, whose translucent, morphing track nothing drawn matches.
//!
//! On iOS the control is a native view between the planes, clipped by what
//! clips its node, and touches go to it. Its node is the one accessibility
//! element, so assistive technology hears zgx's tree alone, and activating
//! that element toggles the control. Elsewhere it is drawn.
use crate::compose::prelude::*;
use crate::semantics::Role;
use std::rc::Rc;

/// What a hosted toggle shows and whom it tells.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub struct ToggleState {
    /// Whether it is on, as last read.
    pub on: std::cell::Cell<bool>,
    /// The person changed it.
    pub changed: Box<dyn Fn(bool)>,
}

/// What a hosted tab bar shows and whom it tells.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub struct TabsState {
    /// Each tab's title and system symbol.
    pub items: Vec<(String, String)>,
    /// The chosen tab, as last read.
    pub selected: std::cell::Cell<usize>,
    /// The person chose a tab.
    pub changed: Box<dyn Fn(usize)>,
}

/// The system's tab bar: its glass, and the lens that morphs between tabs,
/// which nothing drawn matches. `items` are each tab's title and SF Symbol.
/// `selected` runs again when state it reads changes; `set` hears of each
/// choice. Each tab is one element to assistive technology, the bar's own hidden.
/// Elsewhere, a row of buttons. Give it the bar's height, 83 points.
pub fn native_tabs(
    items: Vec<(String, String)>,
    selected: impl FnMut() -> usize + 'static,
    set: impl Fn(usize) + 'static,
) -> View {
    component(move |cx| {
        let mut selected = selected;
        let state = Rc::new(TabsState {
            selected: std::cell::Cell::new(selected()),
            changed: Box::new(set),
            items,
        });
        let current = cx.state(state.selected.get());
        {
            let (state, current) = (state.clone(), current.clone());
            let effect = cx.runtime().effect(move || {
                let next = selected();
                state.selected.set(next);
                if current.with_untracked(|now| *now != next) {
                    current.set(next);
                }
                crate::native_view::wake();
            });
            cx.retain(effect);
        }
        let mut bar = row().w_full().h_full().role(Role::TabList);
        for (index, (title, _)) in state.items.iter().enumerate() {
            let (chosen, choose, state) = (current.clone(), state.clone(), state.clone());
            let _ = choose;
            bar = bar.child(
                div()
                    .flex_grow(1.)
                    .h_full()
                    .role(Role::Tab)
                    .accessible_label(title.clone())
                    .selected_when(move || chosen.get() == index)
                    .on_click(move || (state.changed)(index)),
            );
        }
        #[cfg(target_os = "ios")]
        {
            let natives = cx.try_service::<crate::native_view::NativeViews>();
            bar.on_mount(move |handle| {
                let Some(natives) = natives else { return };
                if natives.supported()
                    && let Some(scene) = handle.scene()
                    && let Ok(mut scene) = scene.try_borrow_mut()
                {
                    scene.set_platform_painted(handle.node(), true);
                }
                handle.retain(natives.register(
                    handle.node(),
                    crate::native_view::NativeContent::Tabs(state),
                ));
            })
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = &state;
            bar
        }
    })
}

/// An on and off control named `label`: `is_on` runs again when state it
/// reads changes, and `set` hears of each change. Put it in a virtualized
/// list when there are many: each costs about 75 KB while mounted.
pub fn native_toggle(
    label: impl Into<String>,
    is_on: impl FnMut() -> bool + 'static,
    set: impl Fn(bool) + 'static,
) -> View {
    let label = label.into();
    component(move |cx| {
        let mut is_on = is_on;
        let state = Rc::new(ToggleState {
            on: std::cell::Cell::new(is_on()),
            changed: Box::new(set),
        });
        let on = cx.state(state.on.get());
        {
            let (state, on) = (state.clone(), on.clone());
            let effect = cx.runtime().effect(move || {
                let next = is_on();
                state.on.set(next);
                if on.with_untracked(|now| *now != next) {
                    on.set(next);
                }
                crate::native_view::wake();
            });
            cx.retain(effect);
        }
        let toggle = {
            let state = state.clone();
            move || (state.changed)(!state.on.get())
        };
        let checked = on.clone();
        let view = div()
            .w(63.)
            .h(28.)
            .flex_shrink(0.)
            .rounded(14.)
            .role(Role::Switch)
            .accessible_label(label)
            .checked_when(move || checked.get())
            .on_click(toggle);
        #[cfg(target_os = "ios")]
        {
            let natives = cx.try_service::<crate::native_view::NativeViews>();
            view.on_mount(move |handle| {
                let Some(natives) = natives else { return };
                if natives.supported()
                    && let Some(scene) = handle.scene()
                    && let Ok(mut scene) = scene.try_borrow_mut()
                {
                    scene.set_platform_painted(handle.node(), true);
                }
                handle.retain(natives.register(
                    handle.node(),
                    crate::native_view::NativeContent::Toggle(state),
                ));
            })
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = &state;
            let (track, thumb) = (on.clone(), on);
            view.reactive_style(move || {
                Styles::new().bg(if track.get() {
                    crate::style::rgb(0x34c759)
                } else {
                    crate::style::rgb(0xe5e5ea)
                })
            })
            .child(
                div()
                    .absolute()
                    .top(2.)
                    .w(39.)
                    .h(24.)
                    .rounded(12.)
                    .bg(crate::style::rgb(0xffffff))
                    .reactive_style(move || Styles::new().left(if thumb.get() { 22. } else { 2. })),
            )
        }
    })
}

/// How a [`SystemUi::alert`] shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertStyle {
    /// In the middle of the screen.
    Alert,
    /// Rising from the bottom, for a choice among actions.
    ActionSheet,
}
/// What a button of a [`SystemUi::alert`] says about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertButtonStyle {
    /// An ordinary choice.
    Default,
    /// What backs out: the system places it apart.
    Cancel,
    /// What destroys something: the system marks it.
    Destructive,
}
thread_local! {
    static ESCAPES: std::cell::RefCell<Vec<Rc<dyn Fn() -> bool>>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// What "go back" does: VoiceOver's two-finger scrub (the host calls [`escape`]).
/// `handler` answers whether it went back.
pub fn on_escape(handler: Rc<dyn Fn() -> bool>) {
    ESCAPES.with(|escapes| escapes.borrow_mut().push(handler));
}
/// Go back, the latest handler first; whether one did.
pub fn escape() -> bool {
    let handlers = ESCAPES.with(|escapes| escapes.borrow().clone());
    handlers.iter().rev().any(|handler| handler())
}

/// An edge of a scroll view or the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// The top.
    Top,
    /// The bottom.
    Bottom,
    /// The left.
    Left,
    /// The right.
    Right,
}
/// The system's scroll edge effect on a node's box (iOS): what passes under the
/// box softens and fades, as under a navigation bar. Paint after the node shows
/// above it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeEffect {
    /// The window's edge it lies on.
    pub edge: Edge,
    /// A veil cut sharply at the box's end, rather than the soft ramp past it.
    pub hard: bool,
}
thread_local! {
    static PENDING_EDGES: std::cell::RefCell<Vec<(crate::scene::NodeId, EdgeEffect)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}
/// Give `node` the system's edge effect: the host takes it up at its next frame.
pub fn request_edge_effect(node: crate::scene::NodeId, effect: EdgeEffect) {
    PENDING_EDGES.with(|pending| pending.borrow_mut().push((node, effect)));
    crate::native_view::wake();
}
/// How far something resting `contact` above a keyboard rises when the keyboard
/// has come `travel` of its `total` way up: with it from a little before they
/// touch, at the keyboard's speed once they have. The keyboard is near its fastest
/// by then: started there at a stroke, the thing would go from still to that speed
/// in a frame. Quadratic, so its speed grows smoothly from none to the keyboard's;
/// never under the plain `travel - contact`, so the keyboard never covers it.
pub fn keyboard_rise(travel: f64, contact: f64, total: f64) -> f64 {
    /// Half the way the blend takes, at most.
    const EASE: f64 = 120.;
    let ease = EASE.min(contact).min(total - contact).max(0.);
    let past = travel - contact;
    if past <= -ease {
        0.
    } else if past >= ease {
        past
    } else {
        (past + ease) * (past + ease) / (4. * ease)
    }
}
/// UIKit's ease-in-out (`kCAMediaTimingFunctionEaseInEaseOut`, the cubic
/// Bézier through 0.42, 0 and 0.58, 1): the share of the way done at `t` of
/// the time. What a `UITableView` scrolls a field above the keyboard on.
pub fn ease_in_out(t: f64) -> f64 {
    const X1: f64 = 0.42;
    const X2: f64 = 0.58;
    let t = t.clamp(0., 1.);
    let bezier = |u: f64, a: f64, b: f64| {
        let v = 1. - u;
        3. * v * v * u * a + 3. * v * u * u * b + u * u * u
    };
    // The curve's parameter where its x is `t`, by halving: x rises with it.
    let (mut low, mut high) = (0f64, 1f64);
    for _ in 0..40 {
        let mid = (low + high) / 2.;
        if bezier(mid, X1, X2) < t {
            low = mid;
        } else {
            high = mid;
        }
    }
    bezier((low + high) / 2., 0., 1.)
}
/// A page's offset every `1 / rate` of a second as it scrolls a field clear of
/// the keyboard as UIKit's scroll views do (`KeyboardRide::Native`): from
/// `first` to `last` on `ease_in_out` over `duration`, the keyboard's, both
/// ends included. It doesn't follow the keyboard's edge: the field lands on
/// its own curve.
pub fn native_keyboard_scroll(first: f32, last: f32, duration: f64, rate: f64) -> Vec<f32> {
    let steps = (duration * rate).ceil().max(1.) as usize;
    (0..=steps)
        .map(|step| {
            let done = ease_in_out(step as f64 / steps as f64);
            first + (last - first) * done as f32
        })
        .collect()
}
/// A spring's share of its starting distance still to go every `1 / rate` of
/// a second over `duration`, both ends included: the keyboard's, from its own
/// `CASpringAnimation` (mass, stiffness, damping, initial velocity).
pub fn spring_remaining(
    mass: f64,
    stiffness: f64,
    damping: f64,
    velocity: f64,
    duration: f64,
    rate: f64,
) -> Vec<f64> {
    const SUBSTEPS: usize = 16;
    let steps = (duration * rate).ceil().max(1.) as usize;
    let dt = duration / steps as f64 / SUBSTEPS as f64;
    let (mut x, mut v) = (1f64, -velocity);
    let mut out = Vec::with_capacity(steps + 1);
    out.push(x);
    for _ in 0..steps {
        for _ in 0..SUBSTEPS {
            v += (-damping * v - stiffness * x) / mass * dt;
            x += v * dt;
        }
        out.push(x);
    }
    out
}
/// `plain`, a page's place for each place of the keyboard's edge, with the
/// start of its rise eased (`keyboard_rise`), for a keyboard going between
/// edges `start` and `end` (`KeyboardRide::Attached`): once the edge reaches
/// the field, the field goes with it.
pub fn keyboard_attached(plain: impl Fn(f64) -> f32, start: f64, end: f64) -> impl Fn(f64) -> f32 {
    let (top, bottom) = (start.min(end), start.max(end));
    let rest = plain(bottom);
    let total = bottom - top;
    let contact = total - f64::from(plain(top) - rest);
    move |edge| rest + keyboard_rise(bottom - edge, contact, total) as f32
}
/// A bar that goes with the keyboard (`ui.keyboard_bar`).
#[derive(Clone)]
pub struct KeyboardBar {
    /// What marks it, filling the bar: its box says where the bar rests.
    pub node: crate::scene::NodeId,
    /// Told how far up it is to be drawn.
    pub lift: Rc<dyn Fn(f32)>,
}
thread_local! {
    static PENDING_BARS: std::cell::RefCell<Vec<KeyboardBar>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// Have the host make `bar` go with the keyboard.
pub fn request_keyboard_bar(bar: KeyboardBar) {
    PENDING_BARS.with(|pending| pending.borrow_mut().push(bar));
    crate::native_view::wake();
}
/// Whether any wait for the host to take them: they make it active.
pub fn keyboard_bars_pending() -> bool {
    PENDING_BARS.with(|pending| !pending.borrow().is_empty())
}
/// What was asked for since the host last took them.
pub fn take_keyboard_bars() -> Vec<KeyboardBar> {
    PENDING_BARS.with(|pending| std::mem::take(&mut *pending.borrow_mut()))
}
/// Whether any wait for the host to take them: they make it active.
pub fn edge_effects_pending() -> bool {
    PENDING_EDGES.with(|pending| !pending.borrow().is_empty())
}
/// What was asked for since the host last took them.
pub fn take_edge_effects() -> Vec<(crate::scene::NodeId, EdgeEffect)> {
    PENDING_EDGES.with(|pending| std::mem::take(&mut *pending.borrow_mut()))
}

/// What a platform's pull to refresh on a scroll view does: `top` is where its
/// spinner hangs from, and `refresh` is given what ends it.
#[derive(Clone)]
pub struct PullRefresh {
    /// Points from the window's top.
    pub top: Rc<dyn Fn() -> f32>,
    /// Told when a pull lets go past where it refreshes.
    pub refresh: Rc<dyn Fn(RefreshDone)>,
}
/// Called when a refresh has finished.
pub type RefreshDone = Rc<dyn Fn()>;
type Pulls = std::collections::HashMap<crate::scene::NodeId, PullRefresh>;
thread_local! {
    static PULLS: std::cell::RefCell<Pulls> = std::cell::RefCell::default();
}
/// Give `node`, a scroll view, the platform's pull to refresh.
pub fn set_pull_refresh(node: crate::scene::NodeId, pull: PullRefresh) {
    PULLS.with(|pulls| pulls.borrow_mut().insert(node, pull));
}
/// The pull to refresh of the scroll view around the window's point `(x, y)`.
pub fn pull_refresh_at(scene: &crate::scene::Scene, x: f32, y: f32) -> Option<PullRefresh> {
    PULLS.with(|pulls| {
        let mut pulls = pulls.borrow_mut();
        pulls.retain(|node, _| scene.contains(*node));
        if pulls.is_empty() {
            return None;
        }
        let hit = scene.hit_test(x, y)?;
        std::iter::once(hit)
            .chain(scene.ancestors(hit))
            .find_map(|node| pulls.get(&node).cloned())
    })
}

/// Where a pan the platform recognizes is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanPhase {
    /// It was recognized.
    Began,
    /// The finger moved.
    Changed,
    /// The finger lifted.
    Ended,
    /// The platform took it away: what it moved goes back.
    Cancelled,
}
/// A pan's state: how far from where it began, and how fast, in points.
#[derive(Clone, Copy, Debug)]
pub struct PanEvent {
    /// Where it is.
    pub phase: PanPhase,
    /// Across, from where it began.
    pub x: f32,
    /// Down, from where it began.
    pub y: f32,
    /// Across, in points a second.
    pub vx: f32,
    /// Down, in points a second.
    pub vy: f32,
}
/// A pan the platform recognizes, which an app only draws (`add_pan`).
#[derive(Clone)]
pub struct PanTarget {
    /// The node it starts on, or `None` for a swipe from the leading edge.
    pub node: Option<crate::scene::NodeId>,
    /// Whether it may start now.
    pub can_begin: Option<Rc<dyn Fn() -> bool>>,
    /// Told of each of its states, always to an ending.
    pub on_pan: Rc<dyn Fn(PanEvent)>,
}
thread_local! {
    static PANS: std::cell::RefCell<Vec<PanTarget>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// Have the platform recognize this pan: the latest added wins where two could.
pub fn add_pan(target: PanTarget) {
    PANS.with(|pans| pans.borrow_mut().push(target));
}
/// The pans added.
pub fn pans() -> Vec<PanTarget> {
    PANS.with(|pans| pans.borrow().clone())
}

/// An entry of a node's system menu (`set_system_menu`).
pub struct MenuAction {
    /// What it says.
    pub title: String,
    /// An SF Symbol's name.
    pub symbol: Option<String>,
    /// Shown as what can't be undone.
    pub destructive: bool,
    /// What choosing it does.
    pub run: Rc<dyn Fn()>,
}
/// What gives a node's system menu when it is asked for.
pub type MenuBuilder = Rc<dyn Fn() -> Vec<MenuAction>>;
thread_local! {
    static MENUS: std::cell::RefCell<std::collections::HashMap<crate::scene::NodeId, MenuBuilder>> =
        std::cell::RefCell::default();
}
/// Give `node` a system menu: a long press on it, or what is inside it, shows
/// what `build` gives, the system's way (a host that has one: iOS).
pub fn set_system_menu(node: crate::scene::NodeId, build: MenuBuilder) {
    MENUS.with(|menus| menus.borrow_mut().insert(node, build));
}
/// `node`'s own system menu, for what assistive technology offers on it.
pub fn system_menu_of(node: crate::scene::NodeId) -> Option<Vec<MenuAction>> {
    let build = MENUS.with(|menus| menus.borrow().get(&node).cloned())?;
    Some(build())
}
/// The system menu for the window's point `(x, y)`, with the box of the node
/// that has it: its innermost around what is there.
pub fn system_menu_at(
    scene: &crate::scene::Scene,
    x: f32,
    y: f32,
) -> Option<(crate::scene::Rect, Vec<MenuAction>)> {
    MENUS.with(|menus| {
        let mut menus = menus.borrow_mut();
        // Those whose nodes went with their rows.
        menus.retain(|node, _| scene.contains(*node));
        if menus.is_empty() {
            return None;
        }
        for hit in scene.hit_test_all(x, y) {
            let mut at = Some(hit);
            while let Some(node) = at {
                if let Some(build) = menus.get(&node) {
                    let build = build.clone();
                    return Some((scene.transformed_bounds(node), build()));
                }
                at = scene.parent(node);
            }
        }
        None
    })
}

/// What only the platform does: a host that has these provides them
/// ([`set_system_ui`]), and elsewhere they do nothing.
pub trait SystemUi {
    /// A tap felt in the hand: `"light"`, `"medium"`, `"heavy"`, `"soft"`,
    /// `"rigid"`, `"success"`, `"warning"`, `"error"` or `"selection"`.
    fn haptic(&self, kind: &str);
    /// A line in the record of what gestures did, to see what a stuck one did.
    fn log_gesture(&self, line: &str);
    /// The system's share sheet, for text, an address and files (paths).
    fn share(&self, text: Option<&str>, url: Option<&str>, files: &[String]);
    /// The system's alert or action sheet. `done` hears which button, from 0.
    fn alert(
        &self,
        style: AlertStyle,
        title: &str,
        message: &str,
        buttons: &[(String, AlertButtonStyle)],
        done: Box<dyn FnOnce(usize)>,
    );
}
thread_local! {
    static SYSTEM_UI: std::cell::RefCell<Option<Rc<dyn SystemUi>>> =
        const { std::cell::RefCell::new(None) };
}
/// The host's own (the host calls this).
pub fn set_system_ui(system: Rc<dyn SystemUi>) {
    SYSTEM_UI.with(|slot| *slot.borrow_mut() = Some(system));
}
/// The host's, if it has one.
pub fn system_ui() -> Option<Rc<dyn SystemUi>> {
    SYSTEM_UI.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
mod keyboard_rise_tests {
    use super::keyboard_rise;

    #[test]
    fn the_speed_grows_with_no_step_and_ends_riding_the_keyboard() {
        let (contact, total) = (116., 335.);
        assert_eq!(keyboard_rise(0., contact, total), 0.);
        assert_eq!(keyboard_rise(total, contact, total), total - contact);
        let mut last = (0., 0.);
        for step in 1..=335 {
            let travel = f64::from(step);
            let rise = keyboard_rise(travel, contact, total);
            let speed = rise - last.0;
            assert!(speed >= last.1 - 1e-9, "never slows against the keyboard");
            assert!(speed - last.1 < 0.01, "no step in speed at {travel}");
            assert!(speed <= 1. + 1e-9, "never ahead of the keyboard");
            assert!(rise >= travel - contact - 1e-9, "never covered at {travel}");
            last = (rise, speed);
        }
    }

    #[test]
    fn what_the_keyboard_already_touches_rides_it_from_the_start() {
        assert_eq!(keyboard_rise(40., 0., 335.), 40.);
        // Little to rise: the blend fits in it, and it still ends in place.
        assert_eq!(keyboard_rise(335., 325., 335.), 10.);
    }
}

#[cfg(test)]
mod keyboard_ride_tests {
    use super::{ease_in_out, keyboard_attached, native_keyboard_scroll, spring_remaining};

    #[test]
    fn the_native_ride_is_uikits_ease_in_out_end_to_end() {
        // kCAMediaTimingFunctionEaseInEaseOut: symmetric, slow from rest.
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-6);
        assert!((ease_in_out(0.25) - 0.1291).abs() < 0.001);
        assert!((ease_in_out(0.25) + ease_in_out(0.75) - 1.).abs() < 1e-6);
        let path = native_keyboard_scroll(100., 300., 0.3833, 120.);
        assert_eq!(path.first(), Some(&100.));
        assert_eq!(path.last(), Some(&300.));
        assert!(path.windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn the_attached_ride_holds_the_gap_once_the_edge_reaches_the_field() {
        // A field bottom at 700, 28 points of room, a keyboard rising 336 to 538.
        let (bottom, room, end) = (700f32, 28f32, 538f64);
        let start = end + 336.;
        let plain = move |edge: f64| (f64::from(bottom + room) - edge).max(0.) as f32;
        let page = keyboard_attached(plain, start, end);
        for left in spring_remaining(1., 555.0265, 47.118, 0., 0.3833, 120.) {
            let edge = end + 336. * left;
            if (edge as f32) < bottom + room - 120. {
                let field = bottom - page(edge);
                assert!((edge as f32 - field - room).abs() < 0.01, "at {edge}");
            }
        }
    }
}
