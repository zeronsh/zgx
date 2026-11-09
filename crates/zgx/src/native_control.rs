//! Controls that are the system's own where exactness needs them: iOS's
//! `UISwitch`, whose translucent, morphing track nothing drawn matches.
//!
//! On iOS the control is a native view between the planes, clipped by what
//! clips its node, and touches go to it. Its node is the one accessibility
//! element, so assistive technology hears zgx's tree alone, and activating
//! that element toggles the control. Elsewhere it is drawn.
use crate::compose::prelude::*;
use crate::reactive::Signal;
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
    /// Who hears its height (the system's, as the host reads it).
    pub height: Option<Rc<TabBarHeight>>,
}

/// Provided around a [`native_tabs`] (`compose::provide`): hears how much of
/// the window's bottom the bar takes while it is mounted, 0 once it is gone,
/// as a page's content ends clear of it. The system's height on iOS, as its
/// host reads it from the bar; the bar's box elsewhere.
pub struct TabBarHeight {
    report: Box<dyn Fn(f32)>,
    last: std::cell::Cell<f32>,
}
impl TabBarHeight {
    /// `report` hears each height.
    pub fn new(report: impl Fn(f32) + 'static) -> Self {
        Self {
            report: Box::new(report),
            last: std::cell::Cell::new(0.),
        }
    }
    /// The bar's height now: told only when it changes.
    pub fn report(&self, height: f32) {
        if self.last.replace(height) != height {
            (self.report)(height);
        }
    }
}
/// Reports 0 as the bar unmounts.
struct TabBarGone(Rc<TabBarHeight>);
impl Drop for TabBarGone {
    fn drop(&mut self) {
        self.0.report(0.);
    }
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
        let height = cx.try_service::<TabBarHeight>();
        if let Some(height) = &height {
            cx.retain(TabBarGone(height.clone()));
        }
        let state = Rc::new(TabsState {
            selected: std::cell::Cell::new(selected()),
            changed: Box::new(set),
            items,
            height,
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
            match state.height.clone() {
                Some(height) => {
                    let runtime = cx.runtime();
                    bar.on_mount(move |handle| {
                        if let Some(size) = handle.observe_size() {
                            handle.retain(runtime.effect(move || height.report(size.get().1)));
                        }
                    })
                }
                None => bar,
            }
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

/// What a hosted slider shows and whom it tells.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub struct SliderState {
    /// Its value, as last read.
    pub value: std::cell::Cell<f32>,
    /// The least it goes to.
    pub min: f32,
    /// The most it goes to.
    pub max: f32,
    /// What a drag lands on, from `min`.
    pub step: Option<f32>,
    /// The person moved it.
    pub changed: Box<dyn Fn(f32)>,
}
impl SliderState {
    /// Where a drag to `raw` lands: in the range, on a step if it has one.
    pub fn landed(&self, raw: f32) -> f32 {
        let raw = if raw.is_nan() { self.min } else { raw };
        let stepped = match self.step {
            Some(step) => self.min + ((raw - self.min) / step).round() * step,
            None => raw,
        };
        stepped.clamp(self.min, self.max)
    }
}

/// A slider over `range`, bound both ways to `value`: on iOS the system's
/// `UISlider`, whose glass lens, glide after a release and haptics at its ends
/// are UIKit's own; elsewhere the drawn [`slider`]. `step` is what a drag
/// lands on and what assistive technology moves.
pub fn native_slider(
    label: impl Into<String>,
    value: Signal<f32>,
    range: std::ops::RangeInclusive<f32>,
    step: Option<f32>,
) -> View {
    #[cfg(target_os = "ios")]
    {
        slider_stand_in(label, value, range, step)
    }
    #[cfg(not(target_os = "ios"))]
    {
        let view = slider(label, value, range);
        match step {
            Some(step) => view.step(step),
            None => view,
        }
    }
}

/// The node a hosted slider stands on (iOS's [`native_slider`]): one slider to
/// assistive technology, its value and range followed, moved a step at a time
/// (a hundredth of the range without one), its value read as UISlider's is, a
/// whole percent of the range. It paints nothing; the host's
/// `UISlider` fills its box. Give it the system's height, 31 points.
pub fn slider_stand_in(
    label: impl Into<String>,
    value: Signal<f32>,
    range: std::ops::RangeInclusive<f32>,
    step: Option<f32>,
) -> View {
    let (min, max) = (*range.start(), *range.end());
    assert!(
        min.is_finite() && max.is_finite() && min < max,
        "slider range must be finite and increasing"
    );
    let label = label.into();
    component(move |cx| {
        let write = value.clone();
        let state = Rc::new(SliderState {
            value: std::cell::Cell::new(value.with_untracked(|value| *value)),
            min,
            max,
            step,
            changed: Box::new(move |next| {
                write.set(next);
            }),
        });
        {
            let (state, value) = (state.clone(), value.clone());
            let effect = cx.runtime().effect(move || {
                state.value.set(value.get());
                crate::native_view::wake();
            });
            cx.retain(effect);
        }
        let (read, adjust, percent) = (value.clone(), state.clone(), value.clone());
        let view = div()
            .w(240.)
            .h(31.)
            .flex_shrink(0.)
            .role(Role::Slider)
            .accessible_label(label)
            .numeric_when(f64::from(min), f64::from(max), move || {
                f64::from(read.get())
            })
            // As UISlider's reads: the share of its range, in whole percent.
            .value_when(move || {
                let share = (percent.get().clamp(min, max) - min) / (max - min);
                format!("{}%", (share * 100.).round())
            })
            .adjustable(f64::from(step.unwrap_or((max - min) / 100.)), move |next| {
                (adjust.changed)(adjust.landed(next as f32))
            });
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
                    crate::native_view::NativeContent::Slider(state),
                ));
            })
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = &state;
            view
        }
    })
}

/// What a hosted segmented control shows and whom it tells.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub struct SegmentedState {
    /// What each segment says.
    pub items: Vec<String>,
    /// The chosen segment, as last read.
    pub selected: std::cell::Cell<usize>,
    /// The person chose a segment.
    pub changed: Box<dyn Fn(usize)>,
}

/// One of `items`, side by side, the chosen one `selected` both ways: the
/// system's `UISegmentedControl`, whose glass lens slides to the choice
/// (UIKit's own). Elsewhere a drawn one is `zgx_components::segmented`.
#[cfg(target_os = "ios")]
pub fn native_segmented(
    label: impl Into<String>,
    items: Vec<String>,
    selected: Signal<usize>,
) -> View {
    segmented_stand_in(label, items, selected)
}

/// The node a hosted segmented control stands on (iOS's [`native_segmented`]):
/// as UIKit's reads, each segment a button named by what it says, the chosen
/// one selected, choosing on activation. It paints nothing; the host's
/// `UISegmentedControl` fills its box. Give it the system's height, 32 points.
pub fn segmented_stand_in(
    label: impl Into<String>,
    items: Vec<String>,
    selected: Signal<usize>,
) -> View {
    assert!(!items.is_empty(), "a segmented control has a segment");
    let label = label.into();
    component(move |cx| {
        let write = selected.clone();
        let state = Rc::new(SegmentedState {
            selected: std::cell::Cell::new(selected.with_untracked(|chosen| *chosen)),
            changed: Box::new(move |index| {
                write.set(index);
            }),
            items,
        });
        {
            let (state, selected) = (state.clone(), selected.clone());
            let effect = cx.runtime().effect(move || {
                state.selected.set(selected.get());
                crate::native_view::wake();
            });
            cx.retain(effect);
        }
        let mut bar = row()
            .w(240.)
            .h(32.)
            .flex_shrink(0.)
            .role(Role::Group)
            .accessible_label(label);
        for (index, title) in state.items.iter().enumerate() {
            let (chosen, choose) = (selected.clone(), state.clone());
            bar = bar.child(
                div()
                    .flex_grow(1.)
                    .h_full()
                    .role(Role::Button)
                    .accessible_label(title.clone())
                    .selected_when(move || chosen.get() == index)
                    .on_click(move || (choose.changed)(index)),
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
                    crate::native_view::NativeContent::Segmented(state),
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

/// What a hosted stepper shows and whom it tells.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub struct StepperState {
    /// Its value, as last read.
    pub value: std::cell::Cell<f64>,
    /// The least it goes to.
    pub min: f64,
    /// The most it goes to.
    pub max: f64,
    /// What a press adds or takes away.
    pub step: f64,
    /// The person stepped it.
    pub changed: Box<dyn Fn(f64)>,
}

/// The system's `UIStepper`, `value` both ways between `min` and `max` (`min`
/// below `max`), `step` a press: its glass, and its repeat while held (first
/// step 0.5 s after the press, then every 0.5 s, read in UIKit), are UIKit's.
/// It shows no number, as UIKit's doesn't: show the value beside it. Elsewhere
/// a drawn stepper is `zgx_components::stepper`.
#[cfg(target_os = "ios")]
pub fn native_stepper(
    label: impl Into<String>,
    value: Signal<f64>,
    range: std::ops::RangeInclusive<f64>,
    step: f64,
) -> View {
    stepper_stand_in(label, value, range, step)
}

/// The node a hosted stepper stands on (iOS's [`native_stepper`]): a spin
/// button to assistive technology, its value and range followed, moved a step
/// at a time. It paints nothing; the host's `UIStepper` fills its box, the
/// system's 94 by 32 points.
pub fn stepper_stand_in(
    label: impl Into<String>,
    value: Signal<f64>,
    range: std::ops::RangeInclusive<f64>,
    step: f64,
) -> View {
    let (min, max) = (*range.start(), *range.end());
    assert!(min < max, "a stepper's min is below its max");
    assert!(
        step.is_finite() && step > 0.,
        "a stepper's step is positive"
    );
    let label = label.into();
    component(move |cx| {
        let write = value.clone();
        let state = Rc::new(StepperState {
            value: std::cell::Cell::new(value.with_untracked(|value| *value)),
            min,
            max,
            step,
            changed: Box::new(move |next| {
                write.set(next);
            }),
        });
        {
            let (state, value) = (state.clone(), value.clone());
            let effect = cx.runtime().effect(move || {
                state.value.set(value.get());
                crate::native_view::wake();
            });
            cx.retain(effect);
        }
        let (read, adjust) = (value.clone(), state.clone());
        let view = div()
            .w(94.)
            .h(32.)
            .flex_shrink(0.)
            .role(Role::SpinButton)
            .accessible_label(label)
            .numeric_when(min, max, move || read.get())
            .adjustable(step, move |next| (adjust.changed)(next.clamp(min, max)));
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
                    crate::native_view::NativeContent::Stepper(state),
                ));
            })
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = &state;
            view
        }
    })
}

/// How far a hosted control may draw past its box where nothing clips it: a
/// slider's lens and its shadow, a switch's thumb stretching while held.
#[cfg(any(target_os = "ios", test))]
pub const CONTROL_REACH: f32 = 32.;

/// What a hosted control at `frame`, showing `visible` of itself, is cut to:
/// what clips its node (`clip`), with `CONTROL_REACH` of room around its box
/// inside that; uncut (`visible`) when nothing clips it.
#[cfg(any(target_os = "ios", test))]
pub fn control_cut(
    frame: crate::scene::Rect,
    visible: crate::scene::Rect,
    clip: Option<crate::scene::Rect>,
) -> crate::scene::Rect {
    let reach = crate::scene::Rect::new(
        frame.x - CONTROL_REACH,
        frame.y - CONTROL_REACH,
        frame.width + 2. * CONTROL_REACH,
        frame.height + 2. * CONTROL_REACH,
    );
    clip.and_then(|cut| cut.intersection(visible.union(reach)))
        .unwrap_or(visible)
}

#[cfg(test)]
mod control_cut_tests {
    use super::{CONTROL_REACH, control_cut};
    use crate::scene::Rect;

    #[test]
    fn a_control_draws_past_its_box_up_to_what_clips_it() {
        let frame = Rect::new(16., 255., 370., 30.);
        // The window clips it: its lens and shadow show around its box.
        let window = Rect::new(0., 0., 402., 874.);
        assert_eq!(
            control_cut(frame, frame, Some(window)),
            Rect::new(0., 255. - CONTROL_REACH, 402., 30. + 2. * CONTROL_REACH)
        );
        // A scroll view's edge through it: cut there.
        let viewport = Rect::new(0., 265., 402., 600.);
        let shown = Rect::new(16., 265., 370., 20.);
        assert_eq!(control_cut(frame, shown, Some(viewport)).y, 265.);
        // Nothing clips it: uncut.
        assert_eq!(control_cut(frame, frame, None), frame);
    }
}

/// Where a hosted control goes whose drawing keeps clear of the safe area, inside
/// what holds it, and how far that holder's `sublayerTransform` moves it: its
/// geometry stays in the middle of the window, clear of the safe area whatever
/// the orientation, and it shows at `frame`. (`cut`: where the holder lies.)
#[cfg(any(target_os = "ios", test))]
pub fn control_held_clear(
    frame: crate::scene::Rect,
    cut: crate::scene::Rect,
    window: (f32, f32),
) -> ((f32, f32), (f32, f32)) {
    let middle = (
        (window.0 - frame.width) / 2.,
        (window.1 - frame.height) / 2.,
    );
    (
        (middle.0 - cut.x, middle.1 - cut.y),
        (frame.x - middle.0, frame.y - middle.1),
    )
}

#[cfg(test)]
mod control_held_clear_tests {
    use super::control_held_clear;
    use crate::scene::Rect;

    #[test]
    fn it_shows_at_its_frame_with_its_geometry_in_the_middle_of_the_window() {
        let window = (402., 874.);
        for frame in [
            Rect::new(292., -16., 94., 32.),
            Rect::new(16., 850., 94., 32.),
            Rect::new(100., 400., 94., 32.),
        ] {
            let cut = Rect::new(frame.x - 32., (frame.y - 32.).max(0.), 158., 96.);
            let (at, shift) = control_held_clear(frame, cut, window);
            // Its geometry, in the window: clear of a 62 point top and a 34 point bottom.
            let geometry = (at.0 + cut.x, at.1 + cut.y);
            assert!(geometry.1 >= 62. && geometry.1 + frame.height <= window.1 - 34.);
            assert_eq!(
                (geometry.0 + shift.0, geometry.1 + shift.1),
                (frame.x, frame.y)
            );
        }
    }
}

/// Whether a finger on a hosted control waits a moment before the control hears
/// of it, to see if it starts a scroll (`delaysContentTouches`): on a control,
/// not in a web page, which scrolls itself, and only where something scrolls.
#[cfg(any(target_os = "ios", test))]
pub fn control_press_waits(control: bool, page: bool, scrolls: impl FnOnce() -> bool) -> bool {
    control && !page && scrolls()
}

#[cfg(test)]
mod control_press_tests {
    use super::control_press_waits;

    #[test]
    fn only_a_control_where_something_scrolls_waits() {
        assert!(control_press_waits(true, false, || true));
        assert!(
            !control_press_waits(true, false, || false),
            "nothing scrolls"
        );
        assert!(
            !control_press_waits(false, false, || true),
            "zgx's own view"
        );
        assert!(!control_press_waits(true, true, || true), "a page's own");
    }
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

/// How far past its top a scroll view `height` points tall is pulled before
/// its pull to refresh fires, as `UIRefreshControl` decides (UIKitCore,
/// iOS 26.5, `-_updateSnappingHeight`): half the height, rounded, held under
/// the modern spinner's `maximumSnappingHeight`, 100 × 1.125 scaled by
/// max(height, 372) / 568. 173.1 points on an 874-point screen.
pub fn pull_refresh_threshold(height: f64) -> f64 {
    let half = (height * 0.5).round();
    let most = 100. * 1.125 * height.max(372.) / 568.;
    half.min(most).max(0.)
}
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
    /// Where the finger is, in points from the window's top left.
    pub at: (f32, f32),
}
/// A pan the platform recognizes, which an app only draws (`add_pan`).
#[derive(Clone)]
pub struct PanTarget {
    /// The node it starts on, or `None` for a swipe from the leading edge.
    pub node: Option<crate::scene::NodeId>,
    /// Whether it may start now.
    pub can_begin: Option<Rc<dyn Fn() -> bool>>,
    /// A row's swipe: it begins only sideways, as UIKit's swipe actions do
    /// (`SwipePan`), if this says so of a move that way (points, right
    /// positive). Elsewhere a pan begins whichever way it goes.
    pub across: Option<Rc<dyn Fn(f32) -> bool>>,
    /// Told of each of its states, always to an ending.
    pub on_pan: Rc<dyn Fn(PanEvent)>,
}

/// What UIKit's swipe actions make of a finger's move from where it landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipePan {
    /// Too little yet to say.
    Waits,
    /// It is a swipe.
    Begins,
    /// It is a scroll: a table's scroll view has taken it.
    Fails,
}
/// A row's swipe begins once a finger has gone 25 points across
/// (`_UISwipeActionPanGestureRecognizer`'s `_hysteresis`, UIKit 26.5), unless
/// it has gone 10 down first, where the scroll view around it takes it
/// (`UIScrollViewPanGestureRecognizer`'s, the first past its own wins).
pub fn swipe_pan(dx: f32, dy: f32) -> SwipePan {
    if dy.abs() > 10. {
        SwipePan::Fails
    } else if dx.abs() >= 25. {
        SwipePan::Begins
    } else {
        SwipePan::Waits
    }
}
thread_local! {
    static PANS: std::cell::RefCell<Vec<PanTarget>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// Have the platform recognize this pan: the latest added wins where two could.
pub fn add_pan(target: PanTarget) {
    PANS.with(|pans| pans.borrow_mut().push(target));
}
/// A finger's part in what watches them all (`set_touch_watch`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchWatch {
    /// One landed.
    Down,
    /// It lifted with no pan of a row's having taken it.
    Up,
}
/// What hears each finger on the window, and says whether the rest of the
/// window is to hear no more of it.
pub type TouchWatcher = Rc<dyn Fn(TouchWatch, f32, f32) -> bool>;
thread_local! {
    static TOUCH_WATCH: std::cell::RefCell<Option<TouchWatcher>> = const { std::cell::RefCell::new(None) };
    static SWALLOWED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CUSTOM_ACTIONS: std::cell::RefCell<std::collections::HashMap<crate::scene::NodeId, MenuBuilder>> = std::cell::RefCell::default();
}
/// A row with its actions open watches where fingers go, as a table does to
/// close it: a touch elsewhere closes it, and is nothing else.
pub fn set_touch_watch(watch: Option<TouchWatcher>) {
    TOUCH_WATCH.with(|slot| *slot.borrow_mut() = watch);
}
/// Whether anything watches fingers now (the host).
pub fn touch_watched() -> bool {
    TOUCH_WATCH.with(|slot| slot.borrow().is_some())
}
/// Tell what watches of a finger (the host).
pub fn watch_touch(phase: TouchWatch, x: f32, y: f32) {
    let watch = TOUCH_WATCH.with(|slot| slot.borrow().clone());
    if let Some(watch) = watch
        && watch(phase, x, y)
        && phase == TouchWatch::Down
    {
        SWALLOWED.with(|swallowed| swallowed.set(true));
    }
}
/// Whether the finger just landed is nothing to the window (the host, once).
pub fn take_swallowed_touch() -> bool {
    SWALLOWED.with(|swallowed| swallowed.replace(false))
}
/// What `node` offers assistive technology as actions of its own, as a table
/// row's swipe actions are (VoiceOver's "Actions available").
pub fn set_custom_actions(node: crate::scene::NodeId, build: MenuBuilder) {
    CUSTOM_ACTIONS.with(|actions| actions.borrow_mut().insert(node, build));
}
/// `node`'s actions, for assistive technology: those it offers and its menu's.
pub fn custom_actions_of(node: crate::scene::NodeId) -> Option<Vec<MenuAction>> {
    let build = CUSTOM_ACTIONS.with(|actions| actions.borrow().get(&node).cloned());
    let own = build.map(|build| build());
    match (own, system_menu_of(node)) {
        (Some(mut own), Some(menu)) => {
            own.extend(menu);
            Some(own)
        }
        (own, menu) => own.or(menu),
    }
}
/// The actions of what assistive technology has reached at `node`: its own, or
/// those of the nearest node around it with some, as a cell's reach what's in it.
pub fn custom_actions_around(
    scene: &crate::scene::Scene,
    node: crate::scene::NodeId,
) -> Option<Vec<MenuAction>> {
    std::iter::once(node)
        .chain(scene.ancestors(node))
        .find_map(custom_actions_of)
}
/// Forget the pans of nodes gone from `scene`: rows of a list come and go.
pub fn prune_pans(scene: &crate::scene::Scene) {
    PANS.with(|pans| {
        pans.borrow_mut()
            .retain(|target| target.node.is_none_or(|node| scene.contains(node)));
    });
    CUSTOM_ACTIONS.with(|actions| actions.borrow_mut().retain(|node, _| scene.contains(*node)));
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
