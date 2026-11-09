//! Style transitions (`zgx::transition`): a view's opacity or colours
//! easing from what is drawn to each new value, over its duration on its
//! curve; ending exactly on the target with nothing scheduled after;
//! turning from where it is when the value changes again, and coming back
//! as fast as it went; landing at once under reduced motion; and a view
//! without one drawing its new value at once, as ever.
use std::time::{Duration, Instant};
use zgx::{
    compose::prelude::*,
    reactive::Signal,
    scene::{Color, NodeId, NodeKind},
    theme::{TokenInput, tok},
    transition::{Ease, Transition, TransitionProp},
    widgets::Ui,
};

/// A box whose opacity follows `opacity`, and its background `fill`, easing
/// as `transition` says.
fn mount(transition: Option<Transition>) -> (Ui, NodeId, Signal<f32>, Signal<u32>) {
    let mut ui = Ui::new(200., 200.);
    let opacity = ui.signal(1f32);
    let fill = ui.signal(0u32);
    let (o, f) = (opacity.clone(), fill.clone());
    let boxed = div().id("box").size(50., 50.).reactive_style(move || {
        let color = if f.get() == 0 {
            Color(0, 0, 0, 255)
        } else {
            Color(200, 100, 0, 255)
        };
        Styles::new().opacity(o.get()).bg(color)
    });
    let boxed = match transition {
        Some(transition) => boxed.transition(transition),
        None => boxed,
    };
    let view = ui.mount(column().child(boxed));
    ui.prepare_frame();
    let node = view.find("box").unwrap();
    (ui, node, opacity, fill)
}

fn drawn_opacity(ui: &Ui, node: NodeId) -> f32 {
    ui.scene.borrow().effects(node).opacity
}

fn drawn_fill(ui: &Ui, node: NodeId) -> Color {
    match ui.scene.borrow().kind(node) {
        NodeKind::Panel { quad, .. } | NodeKind::Quad(quad) => quad.fill,
        other => panic!("{other:?}"),
    }
}

fn linear(ms: f32) -> Transition {
    Transition::of([TransitionProp::Opacity, TransitionProp::Background])
        .duration(ms)
        .ease(Ease::Linear)
}

#[test]
fn a_change_eases_to_its_target_exactly_and_leaves_nothing_scheduled() {
    let (mut ui, node, opacity, fill) = mount(Some(linear(200.)));
    assert_eq!(ui.next_interaction_deadline(), None, "nothing at rest");
    let start = Instant::now();
    opacity.set(0.);
    fill.set(1);
    ui.prepare_frame();
    assert!(drawn_opacity(&ui, node) > 0.99, "still what was drawn");
    assert!(ui.next_interaction_deadline().is_some(), "it will step");
    ui.advance_interactions(start + Duration::from_millis(100))
        .unwrap();
    let half = drawn_opacity(&ui, node);
    assert!(0.3 < half && half < 0.7, "halfway: {half}");
    let Color(r, ..) = drawn_fill(&ui, node);
    assert!(0 < r && r < 200, "the fill halfway: {r}");
    ui.advance_interactions(start + Duration::from_millis(400))
        .unwrap();
    assert_eq!(drawn_opacity(&ui, node), 0., "the target exactly");
    assert_eq!(drawn_fill(&ui, node), Color(200, 100, 0, 255));
    assert_eq!(
        ui.next_interaction_deadline(),
        None,
        "nothing scheduled after"
    );
}

#[test]
fn a_change_midway_turns_from_what_is_drawn() {
    let (mut ui, node, opacity, _) = mount(Some(linear(200.)));
    let start = Instant::now();
    opacity.set(0.);
    ui.prepare_frame();
    ui.advance_interactions(start + Duration::from_millis(100))
        .unwrap();
    let midway = drawn_opacity(&ui, node);
    // Turned to a third value: from where it is, not where it started.
    opacity.set(0.8);
    ui.prepare_frame();
    let turned = drawn_opacity(&ui, node);
    assert!(
        (turned - midway).abs() < 0.1,
        "from {midway}, drawn {turned}"
    );
    ui.advance_interactions(Instant::now() + Duration::from_millis(400))
        .unwrap();
    assert_eq!(drawn_opacity(&ui, node), 0.8);
}

#[test]
fn a_change_reversed_comes_back_as_fast_as_it_went() {
    let (mut ui, node, opacity, _) = mount(Some(linear(400.)));
    let start = Instant::now();
    opacity.set(0.);
    ui.prepare_frame();
    ui.advance_interactions(start + Duration::from_millis(100))
        .unwrap();
    // Back to where it came from, a quarter of the way in: a quarter of
    // the duration back (CSS's reversing shortening factor).
    let back = Instant::now();
    opacity.set(1.);
    ui.prepare_frame();
    ui.advance_interactions(back + Duration::from_millis(150))
        .unwrap();
    assert_eq!(
        drawn_opacity(&ui, node),
        1.,
        "back within a quarter of 400 ms"
    );
    assert_eq!(ui.next_interaction_deadline(), None);
}

#[test]
fn under_reduced_motion_a_token_duration_lands_at_once() {
    // As the hosts set them under reduced motion: motion durations of 0.
    let (ui, node, opacity, _) = mount(Some(
        Transition::of([TransitionProp::Opacity]).duration(tok::MOTION_DURATION_NORMAL),
    ));
    ui.theme().set_system_tokens([(
        tok::MOTION_DURATION_NORMAL.0,
        TokenInput::parse("0").unwrap(),
    )]);
    ui.prepare_frame();
    opacity.set(0.);
    ui.prepare_frame();
    assert_eq!(drawn_opacity(&ui, node), 0., "at once");
    assert_eq!(ui.next_interaction_deadline(), None);
}

#[test]
fn a_view_without_a_transition_draws_its_change_at_once() {
    let (ui, node, opacity, fill) = mount(None);
    opacity.set(0.);
    fill.set(1);
    ui.prepare_frame();
    assert_eq!(drawn_opacity(&ui, node), 0.);
    assert_eq!(drawn_fill(&ui, node), Color(200, 100, 0, 255));
    assert_eq!(ui.next_interaction_deadline(), None);
}
