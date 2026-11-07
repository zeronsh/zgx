//! The pointer locked to a node (`InputDispatcher::lock_pointer`): only just
//! after a person's own press or key; every pointer event to it while it
//! holds it; Escape, focus leaving the window and the node leaving end it.
use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::{EventPhase, InputEvent, Key, Modifiers, PointerButton},
    scene::NodeId,
    widgets::Ui,
};

type Heard = Rc<RefCell<Vec<String>>>;

/// A game's box at the left (0..200), another focusable box at the right
/// (200..400), each writing down what reaches it as its target.
fn world() -> (Ui, NodeId, NodeId, Heard, Heard) {
    let mut ui = Ui::new(400., 100.);
    let (game, other): (Heard, Heard) = Default::default();
    let listen = |heard: &Heard| {
        let heard = heard.clone();
        move |cx: &mut zgx::input::EventContext| {
            if cx.phase != EventPhase::Target {
                return;
            }
            let said = match &cx.event {
                InputEvent::PointerDown { .. } => "down".to_owned(),
                InputEvent::PointerUp { .. } => "up".to_owned(),
                InputEvent::PointerMove { .. } => "move".to_owned(),
                InputEvent::PointerMotion { dx, dy } => format!("motion {dx} {dy}"),
                InputEvent::PointerLockChange(locked) => format!("locked {locked}"),
                InputEvent::Scroll { .. } => "wheel".to_owned(),
                InputEvent::Activate => "click".to_owned(),
                InputEvent::KeyDown { key, repeat, .. } => {
                    format!("key {key:?}{}", if *repeat { " again" } else { "" })
                }
                InputEvent::KeyUp { key, .. } => format!("key up {key:?}"),
                _ => return,
            };
            heard.borrow_mut().push(said);
        }
    };
    let mounted = ui.mount(
        row()
            .child(div().id("game").size(200., 100.).on_event(listen(&game)))
            .child(
                div()
                    .id("other")
                    .size(200., 100.)
                    .focusable(true)
                    .on_event(listen(&other)),
            ),
    );
    ui.prepare_frame();
    let (g, o) = (
        mounted.find("game").expect("game"),
        mounted.find("other").expect("other"),
    );
    (ui, g, o, game, other)
}

/// A press and release, as a host hands them on (`note_gesture` first).
fn press(ui: &mut Ui, x: f32, y: f32) {
    let button = PointerButton::Primary;
    for event in [
        InputEvent::PointerDown { x, y, button },
        InputEvent::PointerUp { x, y, button },
    ] {
        ui.input.note_gesture();
        ui.dispatch(event);
    }
}
/// A key, as a host hands it on.
fn typed(ui: &mut Ui, event: InputEvent) {
    if !matches!(event, InputEvent::KeyDown { repeat: true, .. }) {
        ui.input.note_gesture();
    }
    ui.dispatch(event);
}

fn key(key: Key) -> InputEvent {
    InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    }
}

/// `game` locked just after a click on it, what it heard so far forgotten.
fn locked() -> (Ui, NodeId, NodeId, Heard, Heard) {
    let (mut ui, g, o, game, other) = world();
    press(&mut ui, 50., 50.);
    assert!(ui.input.lock_pointer(&ui.scene, g));
    game.borrow_mut().clear();
    other.borrow_mut().clear();
    (ui, g, o, game, other)
}

#[test]
fn the_pointer_locks_only_just_after_a_press_or_a_key() {
    let (mut ui, g, _, game, _) = world();
    assert!(!ui.input.lock_pointer(&ui.scene, g), "nothing done yet");
    assert_eq!(ui.input.pointer_lock(&ui.scene.borrow()), None);
    // Moving and wheeling are no gesture.
    ui.dispatch(InputEvent::PointerMove { x: 50., y: 50. });
    ui.dispatch(InputEvent::Scroll {
        x: 50.,
        y: 50.,
        delta_x: 0.,
        delta_y: 10.,
    });
    assert!(!ui.input.lock_pointer(&ui.scene, g));
    typed(&mut ui, key(Key::Character("e".into())));
    assert!(ui.input.lock_pointer(&ui.scene, g));
    assert_eq!(ui.input.pointer_lock(&ui.scene.borrow()), Some(g));
    assert!(game.borrow().contains(&"locked true".to_owned()));
    assert!(ui.input.lock_pointer(&ui.scene, g), "already holding it");
}

#[test]
fn a_lock_comes_within_a_second_of_the_gesture_as_the_host_settles_it() {
    use std::time::{Duration, Instant};
    let (mut ui, g, _, _, _) = world();
    // The host settles each turn's presses and keys at its turn's time: a
    // lock a second and more after one (a timer's) is refused.
    press(&mut ui, 50., 50.);
    ui.input
        .settle_gestures(Instant::now() - Duration::from_millis(1500));
    assert!(
        !ui.input.lock_pointer(&ui.scene, g),
        "a second and a half on"
    );
    // Settled just now, it may.
    press(&mut ui, 50., 50.);
    ui.input.settle_gestures(Instant::now());
    assert!(ui.input.lock_pointer(&ui.scene, g));
    // Unsettled, a gesture is happening now (its handlers, its turn).
    ui.input.unlock_pointer(&ui.scene);
    ui.input
        .settle_gestures(Instant::now() - Duration::from_secs(5));
    press(&mut ui, 50., 50.);
    assert!(ui.input.lock_pointer(&ui.scene, g));
}

#[test]
fn a_locked_pointer_takes_every_press_and_its_motion_to_the_node() {
    let (mut ui, _, _, game, other) = locked();
    // Over the other box, the pointer's events still go to the game.
    ui.dispatch(InputEvent::PointerMove { x: 300., y: 50. });
    press(&mut ui, 300., 50.);
    ui.dispatch(InputEvent::PointerMotion { dx: 3., dy: -2. });
    ui.dispatch(InputEvent::Scroll {
        x: 300.,
        y: 50.,
        delta_x: 0.,
        delta_y: 10.,
    });
    assert_eq!(
        *game.borrow(),
        ["down", "up", "click", "motion 3 -2", "wheel"],
        "a move goes nowhere: the pointer stays where it locked"
    );
    assert!(other.borrow().is_empty());
    assert_eq!(ui.input.focused(), None, "a locked press focuses nothing");
}

#[test]
fn escape_ends_the_lock_and_goes_nowhere_else() {
    let (mut ui, _, o, game, other) = locked();
    ui.input.focus(&ui.scene, Some(o));
    typed(&mut ui, key(Key::Escape));
    assert_eq!(*game.borrow(), ["locked false"]);
    assert!(
        other.borrow().is_empty(),
        "the Escape that unlocks is the lock's"
    );
    assert_eq!(ui.input.pointer_lock(&ui.scene.borrow()), None);
    // Unlocked, motion goes nowhere and a press lands where it is.
    ui.dispatch(InputEvent::PointerMotion { dx: 1., dy: 1. });
    press(&mut ui, 300., 50.);
    assert_eq!(*game.borrow(), ["locked false"]);
    assert_eq!(*other.borrow(), ["down", "up", "click"]);
    // The next Escape is an ordinary key again.
    typed(&mut ui, key(Key::Escape));
    assert_eq!(
        other.borrow().last().map(String::as_str),
        Some("key Escape")
    );
}

fn escape(repeat: bool) -> InputEvent {
    InputEvent::KeyDown {
        key: Key::Escape,
        modifiers: Modifiers::default(),
        repeat,
    }
}

#[test]
fn the_escape_that_ends_a_lock_is_no_gesture_to_lock_again() {
    let (mut ui, g, o, game, other) = locked();
    ui.input.focus(&ui.scene, Some(o));
    typed(&mut ui, escape(false));
    // A handler hearing the lock end (or a timer just after) can't take
    // the pointer straight back: Escape always lets it go.
    assert!(!ui.input.lock_pointer(&ui.scene, g), "on the Escape");
    typed(&mut ui, escape(true));
    typed(
        &mut ui,
        InputEvent::KeyUp {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        },
    );
    assert!(!ui.input.lock_pointer(&ui.scene, g), "on its release");
    assert_eq!(*game.borrow(), ["locked false"]);
    assert!(
        other.borrow().is_empty(),
        "its repeats and release go nowhere either: {:?}",
        other.borrow()
    );
    // A new press is a new gesture.
    press(&mut ui, 50., 50.);
    assert!(ui.input.lock_pointer(&ui.scene, g));
}

#[test]
fn a_press_before_the_window_lost_focus_locks_nothing() {
    let (mut ui, g, _, _, _) = world();
    press(&mut ui, 50., 50.);
    ui.dispatch(InputEvent::Blur);
    assert!(!ui.input.lock_pointer(&ui.scene, g));
    assert_eq!(ui.input.pointer_lock(&ui.scene.borrow()), None);
}

#[test]
fn a_lock_the_window_cant_hold_isnt_taken_straight_back() {
    let (ui, g, _, game, _) = locked();
    // The host's refusal: the window doesn't have the keyboard.
    ui.input.refuse_pointer_lock(&ui.scene);
    assert_eq!(*game.borrow(), ["locked false"]);
    assert!(
        !ui.input.lock_pointer(&ui.scene, g),
        "the gesture is spent: no lock again each turn"
    );
}

#[test]
fn the_window_losing_focus_ends_the_lock() {
    let (mut ui, _, _, game, _) = locked();
    ui.dispatch(InputEvent::PointerCancel);
    ui.dispatch(InputEvent::Blur);
    assert_eq!(ui.input.pointer_lock(&ui.scene.borrow()), None);
    assert_eq!(
        game.borrow().last().map(String::as_str),
        Some("locked false")
    );
}

#[test]
fn a_disabled_node_loses_the_lock() {
    let (mut ui, g, _, game, _) = locked();
    ui.set_disabled(g, true);
    assert_eq!(ui.input.pointer_lock(&ui.scene.borrow()), None);
    ui.dispatch(InputEvent::PointerMotion { dx: 1., dy: 1. });
    assert!(
        !game.borrow().iter().any(|e| e.starts_with("motion")),
        "{:?}",
        game.borrow()
    );
}

#[test]
fn another_node_locking_takes_it_and_unlocking_lets_it_go() {
    let (mut ui, _, o, game, other) = locked();
    typed(&mut ui, key(Key::Character("e".into())));
    assert!(ui.input.lock_pointer(&ui.scene, o));
    assert_eq!(*game.borrow(), ["locked false"]);
    assert_eq!(
        other.borrow().last().map(String::as_str),
        Some("locked true")
    );
    assert_eq!(ui.input.pointer_lock(&ui.scene.borrow()), Some(o));
    ui.input.unlock_pointer(&ui.scene);
    assert_eq!(
        other.borrow().last().map(String::as_str),
        Some("locked false")
    );
    assert_eq!(ui.input.pointer_lock(&ui.scene.borrow()), None);
    // Unlocking with no lock says nothing.
    ui.input.unlock_pointer(&ui.scene);
    assert_eq!(game.borrow().len(), 1);
}
