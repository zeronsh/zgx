//! A view's own scrollbar (`View::scrollbar_style`): an app's floating rail
//! drawn and timed as the app's, on every platform: 3 across (5 under the
//! pointer) 2 from the edge in a 10 strip, its track 4 short of each end
//! with the thumb whole within it, at least 24 long, fading in and out over
//! 260 ms on an ease-out quad 1.4 s after the last scroll, the theme's
//! active thumb under the pointer, no track, a press beside the thumb
//! bringing it under the pointer, never focused.
use std::{
    rc::Rc,
    time::{Duration, Instant},
};
use zgx::{
    compose::{ScrollbarEase, ScrollbarStyle, prelude::*},
    input::{InputEvent, PointerButton},
    reactive::Signal,
    scene::{Color, NodeId, NodeKind, Rect},
    semantics::Role,
    theme::tok,
    widgets::Ui,
};

fn rail() -> ScrollbarStyle {
    ScrollbarStyle {
        platform: Some(false),
        thickness: Some(3.),
        active_thickness: Some(5.),
        strip: Some(10.),
        side: Some(2.),
        ends: Some(4.),
        inset: Some(0.),
        min_thumb: Some(24.),
        linger: Some(Duration::from_millis(1400)),
        fade: Some(Duration::from_millis(260)),
        fade_in: Some(true),
        ease: Some(ScrollbarEase::OutQuad),
        active_color: Some(true),
        track_fill: Some(false),
        press_jumps: Some(true),
        linger_from_scroll: Some(true),
        focusable: Some(false),
        always_overlay: Some(true),
        ..Default::default()
    }
}

fn bar(ui: &Ui) -> NodeId {
    ui.semantics
        .borrow()
        .iter()
        .find(|(_, n)| n.role == Role::ScrollBar)
        .unwrap()
        .0
}

/// The thumb's box and colour as drawn.
fn thumb(ui: &Ui) -> (Rect, Color) {
    let bar = bar(ui);
    let scene = ui.scene.borrow();
    let thumb = *scene.children(bar).last().unwrap();
    let NodeKind::Quad(quad) = scene.kind(thumb) else {
        panic!("a quad thumb");
    };
    (scene.bounds(thumb), quad.fill)
}

/// A 200 by 100 view of 400 at the window's corner, with `style`.
fn mount(ui: &mut Ui, style: ScrollbarStyle) -> Signal<f32> {
    let offset = ui.signal(0.);
    ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .scrollbar_style(style)
            .child(div().h(400.)),
    );
    ui.prepare_frame();
    offset
}

fn wheel(ui: &mut Ui, delta_y: f32) {
    ui.dispatch(InputEvent::Scroll {
        x: 50.,
        y: 50.,
        delta_x: 0.,
        delta_y,
    });
    ui.prepare_frame();
}

/// Run its fades to `at`.
fn at(ui: &mut Ui, at: Instant) {
    ui.advance_interactions(at).unwrap();
    ui.prepare_frame();
}

fn press(ui: &mut Ui, x: f32, y: f32) {
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
}

#[test]
fn a_rail_is_thin_off_the_edge_and_whole_inside_its_shortened_track() {
    let mut ui = Ui::new(300., 200.);
    let offset = mount(&mut ui, rail());
    // Hidden until the user scrolls, whatever the system shows.
    assert_eq!(thumb(&ui).0.height, 0.);
    wheel(&mut ui, 0.001);
    at(&mut ui, Instant::now() + Duration::from_millis(300));
    let (thumb_box, _) = thumb(&ui);
    // 3 across, 2 from the view's right edge; its track 100 − 2 × 4 = 92,
    // the thumb 92 × 100 / 400 = 23, at least 24, 4 from the top.
    assert_eq!(thumb_box.width, 3.);
    assert_eq!(thumb_box.x + thumb_box.width, 200. - 2.);
    assert_eq!(thumb_box.height, 24.);
    assert!((thumb_box.y - 4.).abs() < 0.01, "{thumb_box:?}");
    // At the end: its foot 4 from the view's.
    offset.set(300.);
    ui.prepare_frame();
    let (thumb_box, _) = thumb(&ui);
    assert!(
        (thumb_box.y + thumb_box.height - 96.).abs() < 0.01,
        "{thumb_box:?}"
    );
    // The strip that takes the pointer: 10 at the edge.
    let strip = ui.scene.borrow().bounds(bar(&ui));
    assert_eq!((strip.x, strip.width), (190., 10.));
}

#[test]
fn under_the_pointer_it_widens_to_five_and_takes_the_active_colour() {
    let mut ui = Ui::new(300., 200.);
    mount(&mut ui, rail());
    wheel(&mut ui, 0.001);
    at(&mut ui, Instant::now() + Duration::from_millis(300));
    let (rest, color) = thumb(&ui);
    let theme = ui.theme_scope();
    assert_eq!(color, theme.color_untracked(tok::SCROLLBAR_THUMB));
    ui.dispatch(InputEvent::PointerMove { x: 195., y: 50. });
    ui.prepare_frame();
    let (active, color) = thumb(&ui);
    assert_eq!(active.width, 5.);
    assert_eq!(active.x + active.width, rest.x + rest.width, "still 2 in");
    assert_eq!(color, theme.color_untracked(tok::SCROLLBAR_THUMB_ACTIVE));
    // No track drawn under it.
    let scene = ui.scene.borrow();
    let track = scene.children(bar(&ui))[0];
    let NodeKind::Rect(fill) = scene.kind(track) else {
        panic!("a track");
    };
    assert_eq!(fill.3, 0);
}

#[test]
fn it_fades_in_and_out_over_its_own_times() {
    let mut ui = Ui::new(300., 200.);
    mount(&mut ui, rail());
    let start = Instant::now();
    wheel(&mut ui, 0.001);
    let full = ui.theme_scope().color_untracked(tok::SCROLLBAR_THUMB).3;
    // Fading in: on an ease-out quad, three quarters there halfway.
    at(&mut ui, start + Duration::from_millis(130));
    let rising = thumb(&ui).1.3;
    assert!(rising > 0 && rising < full, "fading in: {rising} of {full}");
    at(&mut ui, start + Duration::from_millis(300));
    assert_eq!(thumb(&ui).1.3, full, "in");
    // It stays 1.4 s from the scroll, then fades over 260 ms.
    let deadline = ui.next_interaction_deadline().expect("it will fade");
    assert!(
        deadline >= start + Duration::from_millis(1390),
        "{:?}",
        deadline - start
    );
    at(&mut ui, deadline - Duration::from_millis(10));
    assert_eq!(thumb(&ui).1.3, full, "it stays");
    at(&mut ui, deadline + Duration::from_millis(130));
    let falling = thumb(&ui).1.3;
    assert!(falling > 0 && falling < full, "fading: {falling}");
    at(&mut ui, deadline + Duration::from_millis(300));
    assert_eq!(thumb(&ui).0.height, 0., "gone");
    assert_eq!(ui.next_interaction_deadline(), None, "nothing scheduled");
}

#[test]
fn let_go_long_after_the_scroll_it_fades_at_once() {
    // This thread's time, moved by the test alone.
    let clock = zgx::clock::VirtualClock::new();
    let mut ui = Ui::new(300., 200.);
    mount(&mut ui, rail());
    wheel(&mut ui, 0.001);
    ui.dispatch(InputEvent::PointerMove { x: 195., y: 50. });
    ui.prepare_frame();
    clock.advance(Duration::from_secs(5), || {});
    at(&mut ui, clock.now());
    assert!(thumb(&ui).0.height > 0., "held");
    // Off it, 5 s after the scroll: it fades now, not 1.4 s from now.
    ui.dispatch(InputEvent::PointerMove { x: 50., y: 50. });
    ui.prepare_frame();
    let deadline = ui.next_interaction_deadline().expect("it will fade");
    assert!(deadline <= clock.now(), "{:?}", deadline - clock.now());
}

#[test]
fn a_press_beside_the_thumb_brings_it_under_the_pointer_and_keeps_focus() {
    let mut ui = Ui::new(300., 200.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        column()
            .child(
                scroll(offset.clone())
                    .w(200.)
                    .h(100.)
                    .scrollbar_style(rail())
                    .child(div().h(400.)),
            )
            .child(button().id("composer").size(50., 20.).child(text("Send"))),
    );
    ui.prepare_frame();
    let composer = view.find("composer").unwrap();
    assert!(ui.input.focus(&ui.scene, Some(composer)));
    wheel(&mut ui, 0.001);
    at(&mut ui, Instant::now() + Duration::from_millis(300));
    // At 64: the thumb's middle there, 12 above it from the track's start
    // at 4, over a travel of 92 − 24 = 68 for 300 of offset.
    press(&mut ui, 195., 64.);
    let expected = (64. - 12. - 4.) / 68. * 300.;
    assert!(
        (offset.get() - expected).abs() < 0.5,
        "{} for {expected}",
        offset.get()
    );
    let (thumb_box, _) = thumb(&ui);
    assert!((thumb_box.y + thumb_box.height / 2. - 64.).abs() < 0.5);
    assert_eq!(ui.input.focused(), Some(composer), "focus stays put");
}

#[test]
fn a_hint_shows_it_and_its_own_track_stops_short_of_the_view() {
    let mut ui = Ui::new(300., 200.);
    let hinted = ui.signal(false);
    let offset = ui.signal(0.);
    let read = hinted.clone();
    ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .scrollbar_style(ScrollbarStyle {
                hint: Some(Rc::new(move || read.get())),
                track: Some(Rc::new(|| 60.)),
                ..rail()
            })
            .child(div().h(400.)),
    );
    ui.prepare_frame();
    assert_eq!(thumb(&ui).0.height, 0.);
    hinted.set(true);
    ui.prepare_frame();
    at(&mut ui, Instant::now() + Duration::from_millis(300));
    let (thumb_box, _) = thumb(&ui);
    assert!(thumb_box.height > 0., "the hint shows it");
    // Along a 60 track: at the end, its foot 4 from the track's.
    offset.set(300.);
    ui.prepare_frame();
    let (thumb_box, _) = thumb(&ui);
    assert!(
        (thumb_box.y + thumb_box.height - 56.).abs() < 0.01,
        "{thumb_box:?}"
    );
    assert_eq!(ui.scene.borrow().bounds(bar(&ui)).height, 60.);
    // The hint gone and never scrolled: it fades at once.
    hinted.set(false);
    ui.prepare_frame();
    let deadline = ui.next_interaction_deadline().expect("it will fade");
    assert!(deadline <= Instant::now() + Duration::from_millis(20));
}

#[test]
fn the_app_placing_its_view_does_not_show_it() {
    let mut ui = Ui::new(300., 200.);
    let offset = mount(&mut ui, rail());
    // A kept place put back, rows measured, a reload: no one scrolled.
    offset.set(120.);
    ui.prepare_frame();
    assert_eq!(thumb(&ui).0.height, 0.);
    assert_eq!(ui.next_interaction_deadline(), None);
}

#[test]
fn a_rail_left_out_of_assistive_technology_has_no_node_there() {
    let mut ui = Ui::new(300., 200.);
    let offset = mount(
        &mut ui,
        ScrollbarStyle {
            accessible: Some(false),
            ..rail()
        },
    );
    wheel(&mut ui, 30.);
    assert!(offset.get() > 0., "the view scrolls as before");
    assert!(
        !ui.semantics
            .borrow()
            .iter()
            .any(|(_, n)| n.role == Role::ScrollBar),
        "no scrollbar for a reader"
    );
}

#[test]
fn fading_in_it_is_there_from_the_frame_the_user_scrolls() {
    let mut ui = Ui::new(300., 200.);
    mount(&mut ui, rail());
    wheel(&mut ui, 0.001);
    // Before any step of its fade: laid out, and the pointer reaches it.
    assert_eq!(thumb(&ui).0.height, 24.);
    assert_eq!(ui.scene.borrow().bounds(bar(&ui)).width, 10.);
    assert!(
        ui.next_interaction_deadline()
            .is_some_and(|at| at <= Instant::now()),
        "its first step is due at once"
    );
}

#[test]
fn at_sixty_hertz_its_fade_steps_every_frame() {
    let clock = zgx::clock::VirtualClock::new();
    let mut ui = Ui::new(300., 200.);
    let frame = Duration::from_secs_f64(1. / 60.);
    // The refresh as the display says it (16,666,667 ns), the frames as
    // they come (16,666,666 ns apart): each step falls due a hair after
    // its frame.
    ui.set_refresh_interval(frame + Duration::from_nanos(1));
    mount(&mut ui, rail());
    wheel(&mut ui, 0.001);
    // Frames a 60th of a second apart, as a window's (and a test app's):
    // the fade in and, 1.4 s on, the fade out change it each frame.
    let mut alphas = Vec::new();
    for n in 1..=110 {
        clock.advance(frame * n, || {});
        at(&mut ui, clock.now());
        alphas.push(thumb(&ui).1.3);
    }
    let changes =
        |range: std::ops::Range<usize>| alphas[range].windows(2).filter(|w| w[0] != w[1]).count();
    // In: 260 ms, 15 frames; out from 1.4 s (frame 84), as long.
    assert!(changes(0..16) >= 14, "in: {:?}", &alphas[0..16]);
    assert!(changes(83..100) >= 14, "out: {:?}", &alphas[83..100]);
}

#[test]
fn let_go_on_the_thumb_it_stays_lit_until_the_pointer_leaves() {
    let clock = zgx::clock::VirtualClock::new();
    let mut ui = Ui::new(300., 200.);
    mount(&mut ui, rail());
    wheel(&mut ui, 0.001);
    clock.advance(Duration::from_millis(300), || {});
    at(&mut ui, clock.now());
    // A drag on the thumb, let go there with no move after.
    let (thumb_box, _) = thumb(&ui);
    let (x, y) = (195., thumb_box.y + thumb_box.height / 2.);
    ui.dispatch(InputEvent::PointerMove { x, y });
    ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerMove { x, y: y + 20. });
    ui.dispatch(InputEvent::PointerUp {
        x,
        y: y + 20.,
        button: PointerButton::Primary,
    });
    ui.prepare_frame();
    assert_eq!(thumb(&ui).0.width, 5., "still under the pointer");
    clock.advance(Duration::from_secs(5), || {});
    at(&mut ui, clock.now());
    assert!(thumb(&ui).0.height > 0., "it stays while the pointer rests");
    assert_eq!(thumb(&ui).0.width, 5.);
    // Off it: it goes.
    ui.dispatch(InputEvent::PointerMove { x: 50., y: 50. });
    ui.prepare_frame();
    assert_eq!(thumb(&ui).0.width, 3.);
    assert!(ui.next_interaction_deadline().is_some(), "it will fade");
}

#[test]
fn let_go_off_the_bar_it_is_no_longer_lit() {
    let mut ui = Ui::new(300., 200.);
    mount(&mut ui, rail());
    wheel(&mut ui, 0.001);
    at(&mut ui, Instant::now() + Duration::from_millis(300));
    let (thumb_box, _) = thumb(&ui);
    let (x, y) = (195., thumb_box.y + 4.);
    ui.dispatch(InputEvent::PointerMove { x, y });
    ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Primary,
    });
    // Dragged out across the view, let go over the rows.
    ui.dispatch(InputEvent::PointerMove { x: 50., y: 60. });
    ui.dispatch(InputEvent::PointerUp {
        x: 50.,
        y: 60.,
        button: PointerButton::Primary,
    });
    ui.prepare_frame();
    assert_eq!(thumb(&ui).0.width, 3.);
}

#[test]
fn a_bar_that_takes_no_focus_is_not_in_the_tab_order() {
    use zgx::input::{Key, Modifiers};
    let tab = |ui: &mut Ui, shift: bool| {
        ui.dispatch(InputEvent::KeyDown {
            key: Key::Tab,
            modifiers: Modifiers {
                shift,
                ..Default::default()
            },
            repeat: false,
        });
        ui.prepare_frame();
    };
    let mut ui = Ui::new(300., 200.);
    let view = ui.mount(
        column()
            .child(
                scroll(ui.signal(0.))
                    .w(200.)
                    .h(100.)
                    .scrollbar_style(rail())
                    .child(button().id("row").h(400.).child(text("row"))),
            )
            .child(button().id("composer").size(50., 20.).child(text("Send"))),
    );
    ui.prepare_frame();
    let (row, composer) = (view.find("row").unwrap(), view.find("composer").unwrap());
    wheel(&mut ui, 0.001);
    // Shown and lit or not, the keys go from the row to the composer and
    // back, never to the bar.
    let mut seen = Vec::new();
    for _ in 0..4 {
        tab(&mut ui, false);
        seen.push(ui.input.focused());
    }
    assert!(
        seen.iter().all(|f| *f == Some(row) || *f == Some(composer)),
        "{seen:?}"
    );
    for _ in 0..4 {
        tab(&mut ui, true);
        let focused = ui.input.focused();
        assert!(
            focused == Some(row) || focused == Some(composer),
            "{focused:?}"
        );
    }
}
