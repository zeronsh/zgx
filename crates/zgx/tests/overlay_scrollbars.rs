use std::time::{Duration, Instant};
use zgx::{
    compose::prelude::*,
    input::InputEvent,
    reactive::Signal,
    scene::{NodeId, NodeKind},
    semantics::Role,
    widgets::Ui,
};

fn bar(ui: &Ui) -> NodeId {
    ui.semantics
        .borrow()
        .iter()
        .find(|(_, n)| n.role == Role::ScrollBar)
        .unwrap()
        .0
}

/// The thumb's drawn size and opacity (0 to 255).
fn thumb(ui: &Ui) -> (f32, f32, u8) {
    let bar = bar(ui);
    let scene = ui.scene.borrow();
    let thumb = *scene.children(bar).last().unwrap();
    let bounds = scene.bounds(thumb);
    let NodeKind::Quad(quad) = scene.kind(thumb) else {
        panic!("a quad thumb");
    };
    (bounds.width, bounds.height, quad.fill.3)
}

fn mount(ui: &mut Ui) -> Signal<f32> {
    let offset = ui.signal(0.);
    ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .scrollbar(true)
            .child(div().h(400.)),
    );
    ui.prepare_frame();
    offset
}

fn wheel(ui: &mut Ui) {
    ui.dispatch(InputEvent::Scroll {
        x: 50.,
        y: 50.,
        delta_x: 0.,
        delta_y: 30.,
    });
    ui.prepare_frame();
}

#[test]
fn a_scrollbar_shows_at_rest_unless_scrollbars_are_overlays() {
    let mut ui = Ui::new(300., 200.);
    let offset = mount(&mut ui);
    let (width, height, alpha) = thumb(&ui);
    assert!(width > 0. && height > 0. && alpha > 0, "shown at rest");
    wheel(&mut ui);
    assert!(offset.get() > 0.);
    assert_eq!(ui.next_interaction_deadline(), None, "nothing to fade");

    ui.set_overlay_scrollbars(true);
    ui.prepare_frame();
    assert_eq!(thumb(&ui).0, 0., "an overlay hides at rest");
    assert_eq!(
        ui.scene.borrow().bounds(bar(&ui)).width,
        0.,
        "and the pointer reaches the content under it"
    );
    assert!(
        ui.semantics.borrow().get(bar(&ui)).is_some(),
        "assistive technology still has it"
    );
    // Off again, it shows as before.
    ui.set_overlay_scrollbars(false);
    ui.prepare_frame();
    assert!(thumb(&ui).0 > 0.);
}

// zgx's own timing; Windows' is WinUI's (below).
#[cfg(not(windows))]
#[test]
fn an_overlay_scrollbar_shows_as_the_user_scrolls_then_fades() {
    let mut ui = Ui::new(300., 200.);
    ui.set_overlay_scrollbars(true);
    let offset = mount(&mut ui);
    assert_eq!(thumb(&ui).0, 0.);
    // The app scrolling it does not show it, as a page's script does not.
    offset.set(20.);
    ui.prepare_frame();
    assert_eq!(thumb(&ui).0, 0.);
    assert_eq!(ui.next_interaction_deadline(), None);

    let start = Instant::now();
    wheel(&mut ui);
    let (width, _, alpha) = thumb(&ui);
    assert!(width > 0. && alpha > 0, "the user's scroll shows it");
    let deadline = ui.next_interaction_deadline().expect("it will fade");
    assert!(deadline >= start + Duration::from_millis(900));
    // It stays a second, then fades over a fifth of one.
    ui.advance_interactions(deadline - Duration::from_millis(10))
        .unwrap();
    assert_eq!(thumb(&ui).2, alpha, "it stays");
    ui.advance_interactions(deadline + Duration::from_millis(100))
        .unwrap();
    let half = thumb(&ui).2;
    assert!(0 < half && half < alpha, "fading: {half} of {alpha}");
    ui.advance_interactions(deadline + Duration::from_millis(250))
        .unwrap();
    assert_eq!(thumb(&ui).0, 0., "hidden");
    assert_eq!(
        ui.next_interaction_deadline(),
        None,
        "and nothing scheduled"
    );
}

// zgx's own timing; Windows' is WinUI's (below).
#[cfg(not(windows))]
#[test]
fn the_pointer_on_an_overlay_scrollbar_keeps_it() {
    let mut ui = Ui::new(300., 200.);
    ui.set_overlay_scrollbars(true);
    mount(&mut ui);
    wheel(&mut ui);
    let bounds = ui.scene.borrow().bounds(bar(&ui));
    let (x, y) = (bounds.x + bounds.width / 2., bounds.y + 5.);
    ui.dispatch(InputEvent::PointerMove { x, y });
    ui.prepare_frame();
    assert_eq!(ui.next_interaction_deadline(), None, "held");
    ui.advance_interactions(Instant::now() + Duration::from_secs(5))
        .unwrap();
    assert!(thumb(&ui).0 > 0., "still shown");
    // Off it, it lingers, then goes.
    ui.dispatch(InputEvent::PointerMove { x: 50., y: 50. });
    ui.prepare_frame();
    let deadline = ui.next_interaction_deadline().expect("it will fade");
    ui.advance_interactions(deadline + Duration::from_millis(250))
        .unwrap();
    assert_eq!(thumb(&ui).0, 0.);
}

/// Windows' scrollbar (WinUI's): a thin indicator that widens 0.4 s after
/// the pointer arrives and narrows 0.5 s after it leaves, each over 167 ms,
/// and goes 2 s after scrolling stops; each wait a single wake, nothing
/// scheduled at rest.
#[cfg(windows)]
#[test]
fn a_windows_scrollbar_widens_under_the_pointer_and_narrows_after_it() {
    let mut ui = Ui::new(300., 200.);
    ui.set_overlay_scrollbars(true);
    mount(&mut ui);
    let start = Instant::now();
    wheel(&mut ui);
    let (thin, _, _) = thumb(&ui);
    assert_eq!(thin, 2., "a thin indicator");
    let bounds = ui.scene.borrow().bounds(bar(&ui));
    let (x, y) = (bounds.x + bounds.width / 2., bounds.y + 5.);
    ui.dispatch(InputEvent::PointerMove { x, y });
    ui.prepare_frame();
    let widen = ui.next_interaction_deadline().expect("it will widen");
    assert!(widen >= start + Duration::from_millis(390), "after 0.4 s");
    ui.advance_interactions(widen - Duration::from_millis(5))
        .unwrap();
    assert_eq!(thumb(&ui).0, 2., "not yet");
    ui.advance_interactions(widen).unwrap();
    ui.advance_interactions(widen + Duration::from_millis(80))
        .unwrap();
    let midway = thumb(&ui).0;
    assert!(2. < midway && midway < 6., "widening: {midway}");
    ui.advance_interactions(widen + Duration::from_millis(200))
        .unwrap();
    assert_eq!(thumb(&ui).0, 6., "wide");
    assert_eq!(ui.next_interaction_deadline(), None, "held, at rest");
    // Off it: narrows after 0.5 s, then goes after 2 s.
    ui.dispatch(InputEvent::PointerMove { x: 50., y: 50. });
    ui.prepare_frame();
    let left = Instant::now();
    let narrow = ui.next_interaction_deadline().expect("it will narrow");
    assert!(narrow >= left + Duration::from_millis(490) - Duration::from_millis(20));
    ui.advance_interactions(narrow).unwrap();
    ui.advance_interactions(narrow + Duration::from_millis(200))
        .unwrap();
    assert_eq!(thumb(&ui).0, 2., "narrow again");
    let hide = ui.next_interaction_deadline().expect("it will go");
    ui.advance_interactions(hide + Duration::from_millis(100))
        .unwrap();
    assert_eq!(thumb(&ui).0, 0., "gone");
    assert_eq!(ui.next_interaction_deadline(), None, "nothing scheduled");
}

/// Windows' arrow buttons: where scrollbars always show, a press on the
/// bar's end steps it, down at the bottom, up at the top, as an arrow key.
#[cfg(windows)]
#[test]
fn a_windows_scrollbars_arrow_buttons_step_it() {
    use zgx::input::PointerButton;
    let mut ui = Ui::new(300., 200.);
    let offset = mount(&mut ui);
    let bounds = ui.scene.borrow().bounds(bar(&ui));
    let press = |ui: &mut Ui, y: f32| {
        let x = bounds.x + bounds.width / 2.;
        for event in [
            InputEvent::PointerDown {
                x,
                y,
                button: PointerButton::Primary,
            },
            InputEvent::PointerUp {
                x,
                y,
                button: PointerButton::Primary,
            },
        ] {
            ui.dispatch(event);
        }
        ui.prepare_frame();
    };
    press(&mut ui, bounds.y + bounds.height - 4.);
    assert_eq!(offset.get(), 40., "down a step");
    press(&mut ui, bounds.y + 4.);
    assert_eq!(offset.get(), 0., "and up");
    assert_eq!(ui.next_interaction_deadline(), None, "nothing scheduled");
}

/// Windows' arrow buttons held (WinUI's RepeatButton): a step at once, the
/// next after 500 ms, then one each 50 ms, each a single wake, until let go.
#[cfg(windows)]
#[test]
fn a_windows_scrollbars_arrow_button_held_repeats() {
    use zgx::input::PointerButton;
    let mut ui = Ui::new(300., 200.);
    let offset = mount(&mut ui);
    let bounds = ui.scene.borrow().bounds(bar(&ui));
    let (x, y) = (bounds.x + bounds.width / 2., bounds.y + bounds.height - 4.);
    let pressed = Instant::now();
    ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Primary,
    });
    ui.prepare_frame();
    assert_eq!(offset.get(), 40., "a step at once");
    let first = ui.next_interaction_deadline().expect("it will repeat");
    assert!(
        first >= pressed + Duration::from_millis(490),
        "after 500 ms"
    );
    ui.advance_interactions(first).unwrap();
    ui.prepare_frame();
    assert_eq!(offset.get(), 80., "again");
    let next = ui.next_interaction_deadline().expect("and again");
    assert!(next <= first + Duration::from_millis(60), "each 50 ms");
    ui.advance_interactions(next).unwrap();
    ui.prepare_frame();
    assert_eq!(offset.get(), 120.);
    ui.dispatch(InputEvent::PointerUp {
        x,
        y,
        button: PointerButton::Primary,
    });
    ui.prepare_frame();
    assert_eq!(
        ui.next_interaction_deadline(),
        None,
        "let go: nothing scheduled"
    );
}
