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
