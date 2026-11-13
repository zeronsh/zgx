//! `Ui::acted_at`: when input that acts (a press or its release, a key,
//! text, an action) was last dispatched, which a host draws at once and
//! shows without keeping the cadence of the frames before. A pointer passing
//! over, a scroll (paced by the refresh, live on its own) and focus moving
//! leave it as it was.
use zgx::input::{InputEvent, Key, Modifiers, PointerButton};
use zgx::widgets::Ui;

#[test]
fn acting_input_marks_when_it_was_dispatched() {
    let mut ui = Ui::new(300., 200.);
    assert_eq!(ui.acted_at(), None, "nothing dispatched");
    ui.dispatch(InputEvent::PointerMove { x: 10., y: 10. });
    assert_eq!(ui.acted_at(), None, "a pointer passing over doesn't act");
    ui.dispatch(InputEvent::Scroll {
        x: 10.,
        y: 10.,
        delta_x: 0.,
        delta_y: -4.,
    });
    assert_eq!(ui.acted_at(), None, "a scroll has a liveness of its own");
    let before = std::time::Instant::now();
    ui.dispatch(InputEvent::PointerDown {
        x: 10.,
        y: 10.,
        button: PointerButton::Primary,
    });
    let pressed = ui.acted_at().expect("a press acts");
    assert!(pressed >= before);
    let released_before = std::time::Instant::now();
    ui.dispatch(InputEvent::PointerUp {
        x: 10.,
        y: 10.,
        button: PointerButton::Primary,
    });
    let released = ui.acted_at().expect("a release acts: a click lands there");
    assert!(released >= released_before && released >= pressed);
    ui.dispatch(InputEvent::PointerMove { x: 20., y: 20. });
    assert_eq!(ui.acted_at(), Some(released), "moving on leaves the mark");
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Character("a".into()),
        modifiers: Modifiers::default(),
        repeat: false,
    });
    let keyed = ui.acted_at().expect("a key acts");
    assert!(keyed >= released);
    ui.dispatch(InputEvent::Text("a".into()));
    assert!(ui.acted_at().expect("text acts") >= keyed);
}
