//! Base UI's tooltip groups and close delay, initial and final focus, and an inline icon on the
//! text's baseline.
use std::{cell::RefCell, rc::Rc, time::Duration};
use zgx::{
    clock::VirtualClock,
    compose::{
        FocusTarget, IconAlign, SpanIcon, TaskRunner, Tooltip, TooltipGroup, ViewHandle, prelude::*,
    },
    input::InputEvent,
    reactive::Signal,
    scene::NodeKind,
    svg::SvgData,
    task::LocalExecutor,
    widgets::Ui,
};

const MS: Duration = Duration::from_millis(1);
const STEP: Duration = Duration::from_millis(8);

/// Whether text `label` shows (laid out, on screen, not faded out).
fn showing(ui: &Ui, label: &str) -> bool {
    let scene = ui.scene.borrow();
    scene.paint_items().any(|item| {
        matches!(scene.kind(item.id), NodeKind::Text { text, .. } if &**text == label)
            && item.effects.opacity > 0.
            && scene.visible_bounds(item.id).is_some_and(|b| b.width > 0.)
    })
}
struct Tips {
    ui: Ui,
    executor: Rc<RefCell<LocalExecutor>>,
    clock: VirtualClock,
}
impl Tips {
    fn new(view: View) -> Self {
        let clock = VirtualClock::new();
        let mut ui = Ui::new(300., 300.);
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        ui.mount(provide(TaskRunner::from_executor(executor.clone()), view));
        ui.prepare_frame();
        Self {
            ui,
            executor,
            clock,
        }
    }
    fn at(&mut self, x: f32, y: f32) {
        self.ui.dispatch(InputEvent::PointerMove { x, y });
        self.ui.prepare_frame();
    }
    fn wait(&mut self, by: Duration) {
        let (ui, executor) = (&mut self.ui, &self.executor);
        let mut settle = || {
            while executor.borrow_mut().tick() > 0 {}
            ui.prepare_frame();
        };
        settle();
        self.clock.advance(self.clock.elapsed() + by, &mut settle);
        settle();
    }
}

fn group(delay: u64, close: Option<u64>) -> TooltipGroup {
    TooltipGroup {
        delay: Some(Duration::from_millis(delay)),
        close_delay: close.map(Duration::from_millis),
        reshow_within: Some(Duration::from_secs(1)),
        reshow_delay: Some(Duration::ZERO),
    }
}
/// Two tooltips in a group that waits 100 ms, and one in another that waits 400.
fn groups() -> Tips {
    Tips::new(
        column()
            .p(60.)
            .gap(20.)
            .child(
                column()
                    .gap(20.)
                    .child(div().size(60., 24.).tooltip("A1"))
                    .child(div().size(60., 24.).tooltip("A2"))
                    .tooltip_group(group(100, None)),
            )
            .child(
                column()
                    .child(div().size(60., 24.).tooltip("B1"))
                    .tooltip_group(group(400, None)),
            ),
    )
}

#[test]
fn a_group_waits_its_own_delay() {
    let mut tips = groups();
    tips.at(70., 70.);
    tips.wait(Duration::from_millis(99));
    assert!(!showing(&tips.ui, "A1"), "its group's 100, not 300");
    tips.wait(MS + STEP);
    assert!(showing(&tips.ui, "A1"));
}

#[test]
fn within_a_group_the_next_tooltip_shows_at_once() {
    let mut tips = groups();
    tips.at(70., 70.);
    tips.wait(Duration::from_millis(100) + STEP);
    assert!(showing(&tips.ui, "A1"));
    tips.at(70., 110.);
    tips.wait(STEP);
    assert!(showing(&tips.ui, "A2"), "a reshow within the group");
}

#[test]
fn another_group_is_not_a_reshow_and_waits_its_own() {
    let mut tips = groups();
    tips.at(70., 70.);
    tips.wait(Duration::from_millis(100) + STEP);
    assert!(showing(&tips.ui, "A1"));
    // B1 sits below A2: in another group, a tooltip showing there is none of its.
    tips.at(70., 160.);
    tips.wait(Duration::from_millis(399));
    assert!(!showing(&tips.ui, "B1"), "waits its own 400");
    tips.wait(MS + STEP);
    assert!(showing(&tips.ui, "B1"));
}

#[test]
fn a_group_s_close_delay_keeps_its_tooltips_after_the_pointer_leaves() {
    let mut tips = Tips::new(
        column()
            .p(60.)
            .child(div().size(60., 24.).tooltip("Held"))
            .tooltip_group(group(100, Some(200))),
    );
    tips.at(70., 70.);
    tips.wait(Duration::from_millis(100) + STEP);
    assert!(showing(&tips.ui, "Held"));
    tips.at(250., 250.);
    tips.wait(Duration::from_millis(199));
    assert!(showing(&tips.ui, "Held"), "through the close delay");
    tips.wait(MS + Duration::from_millis(120));
    assert!(!showing(&tips.ui, "Held"), "then gone");
}

#[test]
fn a_tooltip_s_own_close_delay_holds_it_after_the_pointer_leaves() {
    let mut tips = Tips::new(
        column().p(60.).child(
            div()
                .size(60., 24.)
                .tooltip(Tooltip::from("Held").close_delay(Duration::from_millis(250))),
        ),
    );
    tips.at(70., 70.);
    tips.wait(Duration::from_millis(300) + STEP);
    assert!(showing(&tips.ui, "Held"));
    tips.at(250., 250.);
    tips.wait(Duration::from_millis(249));
    assert!(showing(&tips.ui, "Held"));
    tips.wait(MS + Duration::from_millis(120));
    assert!(!showing(&tips.ui, "Held"));
}

struct Dialog {
    ui: Ui,
    view: ViewHandle,
    open: Signal<bool>,
}
impl Dialog {
    fn new(initial: FocusTarget, last: FocusTarget, anchored: bool) -> Self {
        let mut ui = Ui::new(400., 300.);
        let open = ui.signal(false);
        let body = |view: View| {
            view.initial_focus(initial.clone())
                .final_focus(last.clone())
                .w(200.)
                .h(120.)
                .child(button().id("first").size(80., 24.).child(text("First")))
                .child(button().id("second").size(80., 24.).child(text("Second")))
        };
        let content = if anchored {
            popover(
                "P",
                open.clone(),
                button().id("trigger").size(60., 24.).child(text("Open")),
            )
            .lazy()
        } else {
            modal("D", open.clone()).lazy()
        };
        let view = ui.mount(
            column()
                .child(button().id("trigger").size(60., 24.).child(text("Open")))
                .child(button().id("other").size(60., 24.).child(text("Other")))
                .child(body(content)),
        );
        ui.prepare_frame();
        Self { ui, view, open }
    }
    fn focused(&self) -> Option<zgx::scene::NodeId> {
        self.ui.input.focused()
    }
    fn node(&self, id: &str) -> Option<zgx::scene::NodeId> {
        self.view.find(id)
    }
    /// Opened from the trigger, which had focus.
    fn open(&mut self) {
        let trigger = self.node("trigger");
        self.ui.input.focus(&self.ui.scene, trigger);
        self.open.set(true);
        self.ui.prepare_frame();
    }
    fn close(&mut self) {
        self.open.set(false);
        self.ui.prepare_frame();
    }
}

#[test]
fn a_dialog_focuses_its_first_control_unless_told_otherwise() {
    let mut d = Dialog::new(FocusTarget::Default, FocusTarget::Default, false);
    d.open();
    assert_eq!(d.focused(), d.node("first"));
    d.close();
    assert_eq!(d.focused(), d.node("trigger"), "back where it was");
}

#[test]
fn initial_focus_names_the_node_focus_lands_on() {
    for anchored in [false, true] {
        let mut d = Dialog::new(
            FocusTarget::Id("second".into()),
            FocusTarget::Default,
            anchored,
        );
        d.open();
        assert_eq!(d.focused(), d.node("second"), "anchored {anchored}");
    }
}

#[test]
fn initial_focus_none_leaves_the_panel_focused_not_a_control() {
    let mut d = Dialog::new(FocusTarget::None, FocusTarget::Default, false);
    d.open();
    assert!(d.focused().is_some(), "the panel holds focus");
    assert_ne!(d.focused(), d.node("first"));
    assert_ne!(d.focused(), d.node("second"));
}

#[test]
fn an_initial_focus_that_is_not_in_it_falls_back_to_the_first_control() {
    let mut d = Dialog::new(FocusTarget::Id("other".into()), FocusTarget::Default, false);
    d.open();
    assert_eq!(
        d.focused(),
        d.node("first"),
        "`other` is outside the dialog"
    );
}

#[test]
fn final_focus_names_where_focus_goes_as_it_closes() {
    for anchored in [false, true] {
        let mut d = Dialog::new(
            FocusTarget::Default,
            FocusTarget::Id("other".into()),
            anchored,
        );
        d.open();
        d.close();
        assert_eq!(d.focused(), d.node("other"), "anchored {anchored}");
    }
}

#[test]
fn final_focus_none_leaves_nothing_focused() {
    let mut d = Dialog::new(FocusTarget::Default, FocusTarget::None, false);
    d.open();
    assert!(d.focused().is_some());
    d.close();
    assert_eq!(d.focused(), None);
}

/// A paragraph of one word with a 56 point icon, in a 14 point font on a 20 point line.
fn icon_line(align: IconAlign) -> (f32, zgx::scene::Rect, zgx::scene::Rect) {
    icon_line_with(align, 0.)
}
fn icon_line_with(align: IconAlign, margin: f32) -> (f32, zgx::scene::Rect, zgx::scene::Rect) {
    let mut ui = Ui::new(400., 200.);
    let svg = std::sync::Arc::new(SvgData::new(&b"<svg viewBox='0 0 24 24'/>"[..]).unwrap());
    let view = ui.mount(
        View::from(rich_text().child(text_span("word").icon(SpanIcon {
            svg,
            size: 56.,
            gap: 4.,
            margin,
            tint: None,
            bg: None,
            radius: 0.,
            pad: 0.,
            align,
        })))
        .text_size(14.)
        .line_height(20.),
    );
    ui.prepare_frame();
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let kids = scene.children(view.node()).to_vec();
    let paragraph = kids
        .iter()
        .copied()
        .find(|id| matches!(scene.kind(*id), NodeKind::RichText { .. }))
        .expect("the paragraph");
    let icon = kids
        .iter()
        .copied()
        .find(|id| matches!(scene.kind(*id), NodeKind::Svg(..)))
        .expect("the icon");
    (
        scene.bounds(paragraph).height,
        scene.bounds(paragraph),
        scene.bounds(icon),
    )
}

#[test]
fn an_icon_centres_on_its_line_unless_it_stands_on_the_baseline() {
    let (height, paragraph, icon) = icon_line(IconAlign::Center);
    assert!(
        (height - 20.).abs() < 1.,
        "the line keeps its 20 points: {height}"
    );
    assert!(
        icon.y < paragraph.y - 10.,
        "centred, a 56 point icon reaches above a 20 point line: {icon:?} {paragraph:?}"
    );
}

#[test]
fn a_baseline_icon_stands_on_the_text_baseline_and_grows_the_line() {
    let (height, paragraph, icon) = icon_line(IconAlign::Baseline);
    // The text's baseline lies 14.2 below a 14 point font's line top (a 20 point line: 3 of leading, 11.2
    // of ascent), 5.8 above its bottom; the icon's bottom edge is on it, the line holding both.
    assert!((icon.height - 56.).abs() < 0.5, "{icon:?}");
    assert!(
        (icon.y - paragraph.y).abs() < 1.,
        "its top is the line's: {icon:?} {paragraph:?}"
    );
    assert!(
        (height - (56. + 5.8)).abs() < 1.5,
        "56 above the baseline, 5.8 below: {height}"
    );
    assert!(
        ((icon.y + icon.height) - (paragraph.y + height - 5.8)).abs() < 1.5,
        "its bottom edge is on the baseline: {icon:?} {paragraph:?} {height}"
    );
}

#[test]
fn an_icon_margin_adds_room_each_side_without_a_taller_line() {
    for align in [IconAlign::Baseline, IconAlign::Center] {
        let (h0, p0, i0) = icon_line_with(align, 0.);
        let (h2, p2, i2) = icon_line_with(align, 2.);
        assert!(
            (h2 - h0).abs() < 0.01,
            "{align:?}: the line as tall: {h0} {h2}"
        );
        assert!(
            ((i2.x - p2.x) - (i0.x - p0.x) - 2.).abs() < 0.01,
            "{align:?}: the icon 2 in from its room's start: {i0:?} {i2:?}"
        );
        assert!(
            ((p2.width - p0.width) - 4.).abs() < 0.01 || align == IconAlign::Center,
            "{align:?}: 2 each side of the box: {p0:?} {p2:?}"
        );
    }
}
