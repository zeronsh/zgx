use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    scene::NodeId,
    widgets::Ui,
};

/// Everything a focus ring could change about a node's drawing.
fn look(ui: &Ui, node: NodeId) -> String {
    let scene = ui.scene.borrow();
    format!(
        "{:?} {:?} {:?}",
        scene.style(node),
        scene.effects(node),
        scene.kind(node)
    )
}

fn settle(ui: &mut Ui) {
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
}

#[test]
fn a_clicked_button_shows_no_focus_ring_but_a_tabbed_one_does() {
    let mut ui = Ui::new(300., 200.);
    let view = ui.mount(
        column()
            .child(button().id("a").size(80., 30.).child(text("A")))
            .child(button().id("b").size(80., 30.).child(text("B"))),
    );
    let (a, b) = (view.find("a").unwrap(), view.find("b").unwrap());
    settle(&mut ui);
    let resting = look(&ui, a);

    ui.dispatch(InputEvent::PointerDown {
        x: 10.,
        y: 10.,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x: 10.,
        y: 10.,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerMove { x: 250., y: 150. });
    settle(&mut ui);
    assert_eq!(ui.input.focused(), Some(a));
    assert_eq!(
        look(&ui, a),
        resting,
        "clicking focuses the button without drawing its ring"
    );

    let tab = |ui: &mut Ui, shift: bool| {
        ui.dispatch(InputEvent::KeyDown {
            key: Key::Tab,
            modifiers: Modifiers {
                shift,
                ..Default::default()
            },
            repeat: false,
        })
    };
    tab(&mut ui, false);
    tab(&mut ui, true);
    settle(&mut ui);
    assert_eq!(ui.input.focused(), Some(a));
    assert_ne!(
        look(&ui, a),
        resting,
        "focus from the keyboard draws the ring"
    );
    assert_ne!(Some(b), ui.input.focused());
}

#[test]
fn a_key_pressed_on_a_clicked_button_shows_its_ring_but_a_shortcut_does_not() {
    let mut ui = Ui::new(300., 200.);
    let view = ui.mount(column().child(button().id("a").size(80., 30.).child(text("A"))));
    let a = view.find("a").unwrap();
    settle(&mut ui);
    let resting = look(&ui, a);
    ui.dispatch(InputEvent::PointerDown {
        x: 10.,
        y: 10.,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x: 10.,
        y: 10.,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerMove { x: 250., y: 150. });
    settle(&mut ui);
    assert_eq!(ui.input.focused(), Some(a));
    assert_eq!(look(&ui, a), resting);

    let key = |ui: &mut Ui, key: Key, modifiers: Modifiers| {
        ui.dispatch(InputEvent::KeyDown {
            key: key.clone(),
            modifiers,
            repeat: false,
        });
        ui.dispatch(InputEvent::KeyUp { key, modifiers });
        settle(ui);
    };
    let meta = Modifiers {
        meta: true,
        ..Default::default()
    };
    key(&mut ui, Key::Character("c".into()), meta);
    assert_eq!(look(&ui, a), resting, "a shortcut chord draws no ring");

    key(&mut ui, Key::Space, Modifiers::default());
    assert_eq!(ui.input.focused(), Some(a));
    assert_ne!(
        look(&ui, a),
        resting,
        "Space activates the button with its ring showing"
    );
}

#[test]
fn an_editor_shows_its_focus_style_however_it_was_focused() {
    let mut ui = Ui::new(300., 200.);
    let value = ui.signal(String::new());
    let view = ui.mount(column().child(text_input("Name", value).id("field").w(200.).h(30.)));
    let field = view.find("field").unwrap();
    settle(&mut ui);
    let resting = look(&ui, field);
    ui.dispatch(InputEvent::PointerDown {
        x: 20.,
        y: 10.,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x: 20.,
        y: 10.,
        button: PointerButton::Primary,
    });
    settle(&mut ui);
    assert_ne!(look(&ui, field), resting);
}

fn press(ui: &mut Ui, key: Key, modifiers: Modifiers) {
    ui.dispatch(InputEvent::KeyDown {
        key: key.clone(),
        modifiers,
        repeat: false,
    });
    ui.dispatch(InputEvent::KeyUp { key, modifiers });
    settle(ui);
}
fn click(ui: &mut Ui, x: f32, y: f32) {
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
    ui.dispatch(InputEvent::PointerMove { x: 290., y: 190. });
    settle(ui);
}
/// A window of two buttons ("New chat" first, as Zeron's) and a menu: its
/// trigger, `open`, and the buttons' looks at rest.
struct Window {
    ui: Ui,
    open: zgx::reactive::Signal<bool>,
    first: NodeId,
    other: NodeId,
    anchor: NodeId,
    item: NodeId,
}
fn window() -> (Window, [String; 3]) {
    let mut ui = Ui::new(300., 200.);
    let open = ui.signal(false);
    let view = ui.mount(
        column()
            .child(button().id("first").size(80., 30.).child(text("New chat")))
            .child(button().id("other").size(80., 30.).child(text("Other")))
            .child(
                menu(
                    "Actions",
                    open.clone(),
                    button().id("anchor").size(80., 30.).child("Menu"),
                )
                .children([menu_item("Alpha").id("alpha"), menu_item("Beta")]),
            ),
    );
    settle(&mut ui);
    let [first, other, anchor, item] =
        ["first", "other", "anchor", "alpha"].map(|id| view.find(id).unwrap());
    let rest = [first, other, anchor].map(|node| look(&ui, node));
    let window = Window {
        ui,
        open,
        first,
        other,
        anchor,
        item,
    };
    (window, rest)
}

#[test]
fn escape_from_a_popup_opened_with_nothing_focused_leaves_nothing_focused() {
    let (mut w, rest) = window();
    // A press on blank space (nothing focusable), then the menu opens.
    click(&mut w.ui, 250., 150.);
    assert_eq!(w.ui.input.focused(), None);
    w.open.set(true);
    settle(&mut w.ui);
    assert_eq!(w.ui.input.focused(), Some(w.item));
    press(&mut w.ui, Key::Escape, Modifiers::default());
    assert!(!w.open.get());
    assert_eq!(w.ui.input.focused(), None, "not the window's first button");
    assert_eq!(look(&w.ui, w.first), rest[0], "and no ring on it");
}

#[test]
fn escape_from_a_popup_returns_focus_to_a_clicked_button_without_a_ring() {
    let (mut w, rest) = window();
    let other = w.ui.scene.borrow().bounds(w.other);
    click(&mut w.ui, other.x + 10., other.y + 10.);
    assert_eq!(w.ui.input.focused(), Some(w.other));
    w.open.set(true);
    settle(&mut w.ui);
    press(&mut w.ui, Key::Escape, Modifiers::default());
    assert_eq!(w.ui.input.focused(), Some(w.other));
    assert_eq!(
        look(&w.ui, w.other),
        rest[1],
        "the pointer's focus: no ring"
    );
    // A shortcut chord draws none either; Tab does.
    let meta = Modifiers {
        meta: true,
        ..Default::default()
    };
    press(&mut w.ui, Key::Character("k".into()), meta);
    assert_eq!(look(&w.ui, w.other), rest[1], "a shortcut draws no ring");
    assert!(!w.ui.input.focus_visible_now());
    press(&mut w.ui, Key::Tab, Modifiers::default());
    assert_eq!(w.ui.input.focused(), Some(w.anchor));
    assert_ne!(
        look(&w.ui, w.anchor),
        rest[2],
        "Tab after Escape draws the ring"
    );
}

#[test]
fn escape_from_a_popup_used_from_the_keyboard_keeps_the_ring_on_its_trigger() {
    let (mut w, rest) = window();
    // Tab to the trigger, open it with Enter, move with the arrows.
    for _ in 0..3 {
        press(&mut w.ui, Key::Tab, Modifiers::default());
    }
    assert_eq!(w.ui.input.focused(), Some(w.anchor));
    press(&mut w.ui, Key::Enter, Modifiers::default());
    assert!(w.open.get());
    press(&mut w.ui, Key::ArrowDown, Modifiers::default());
    press(&mut w.ui, Key::Escape, Modifiers::default());
    assert!(!w.open.get());
    assert_eq!(w.ui.input.focused(), Some(w.anchor));
    assert_ne!(look(&w.ui, w.anchor), rest[2], "the keyboard's ring stays");
}

#[test]
fn a_popup_opened_from_the_keyboard_with_nothing_focused_returns_to_its_trigger() {
    let (mut w, _) = window();
    // A shortcut-like open (the keyboard in use, nothing focused): its
    // trigger takes focus as it closes, ringed.
    press(&mut w.ui, Key::ArrowDown, Modifiers::default());
    assert_eq!(w.ui.input.focused(), None);
    assert!(w.ui.input.focus_visible_now());
    w.open.set(true);
    settle(&mut w.ui);
    press(&mut w.ui, Key::Escape, Modifiers::default());
    assert_eq!(w.ui.input.focused(), Some(w.anchor));
}

#[test]
fn a_right_clicked_button_takes_focus_without_a_ring_and_gets_it_back_from_its_menu() {
    let mut ui = Ui::new(300., 200.);
    let view = ui.mount(
        column()
            .child(button().id("first").size(80., 30.).child(text("New chat")))
            .child(
                button()
                    .id("row")
                    .size(80., 30.)
                    .child(text("Row"))
                    .context_menu(|_| vec![menu_item("Rename").id("rename")]),
            ),
    );
    settle(&mut ui);
    let (row, first) = (view.find("row").unwrap(), view.find("first").unwrap());
    let rest = look(&ui, row);
    let b = ui.scene.borrow().bounds(row);
    let (x, y) = (b.x + 10., b.y + 10.);
    ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Secondary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x,
        y,
        button: PointerButton::Secondary,
    });
    settle(&mut ui);
    assert!(ui.input.focus_scope().is_some(), "its menu opened");
    press(&mut ui, Key::Escape, Modifiers::default());
    assert!(ui.input.focus_scope().is_none(), "Escape closed it");
    assert_eq!(ui.input.focused(), Some(row), "not the first button");
    assert_ne!(ui.input.focused(), Some(first));
    ui.dispatch(InputEvent::PointerMove { x: 290., y: 190. });
    settle(&mut ui);
    assert_eq!(look(&ui, row), rest, "the pointer focused it: no ring");
}

#[test]
fn a_menu_opened_by_clicking_its_trigger_gives_it_focus_back_without_a_ring() {
    // A menu opens on the press, before the press would focus its trigger:
    // it counts as focused, as a browser's press focuses a button first.
    let (mut w, rest) = window();
    let anchor = w.ui.scene.borrow().bounds(w.anchor);
    let (x, y) = (anchor.x + 10., anchor.y + 10.);
    click(&mut w.ui, x, y);
    assert!(w.open.get());
    press(&mut w.ui, Key::Escape, Modifiers::default());
    assert_eq!(w.ui.input.focused(), Some(w.anchor), "not the first button");
    // Its ring is its focus border (its fill may still show the press).
    let ringless = |look: &str| look.contains("border_width: 0.0");
    assert!(ringless(&rest[2]));
    assert!(ringless(&look(&w.ui, w.anchor)), "no ring");
    // Chosen with Return after the arrows, it goes back there too.
    click(&mut w.ui, x, y);
    press(&mut w.ui, Key::ArrowDown, Modifiers::default());
    press(&mut w.ui, Key::Enter, Modifiers::default());
    assert!(!w.open.get());
    assert_eq!(w.ui.input.focused(), Some(w.anchor));
    assert_ne!(w.ui.input.focused(), Some(w.first));
}

#[test]
fn a_trigger_that_opened_its_menu_with_a_press_shows_pressed_no_longer() {
    let (mut w, rest) = window();
    let anchor = w.ui.scene.borrow().bounds(w.anchor);
    click(&mut w.ui, anchor.x + 10., anchor.y + 10.);
    assert!(w.open.get());
    assert_eq!(
        look(&w.ui, w.anchor),
        rest[2],
        "pressed while its menu shows"
    );
    press(&mut w.ui, Key::Escape, Modifiers::default());
    assert_eq!(w.ui.input.focused(), Some(w.anchor));
    assert_eq!(look(&w.ui, w.anchor), rest[2], "pressed after it closed");
}

/// A trigger counting its clicks and releases, which opens `open`.
fn counting(
    open: &zgx::reactive::Signal<bool>,
) -> (
    View,
    std::rc::Rc<std::cell::Cell<u32>>,
    std::rc::Rc<std::cell::Cell<u32>>,
) {
    use std::{cell::Cell, rc::Rc};
    let (clicks, ups) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (counted, up, open) = (clicks.clone(), ups.clone(), open.clone());
    let trigger = button()
        .id("anchor")
        .size(80., 30.)
        .child(text("Open"))
        .on_click(move || {
            counted.set(counted.get() + 1);
            open.set(true);
        })
        .on_event(move |cx| {
            if matches!(cx.event, InputEvent::PointerUp { .. })
                && cx.phase != zgx::input::EventPhase::Capture
            {
                up.set(up.get() + 1);
            }
        });
    (trigger, clicks, ups)
}

#[test]
fn a_trigger_whose_press_opened_its_menu_fires_nothing_more_as_it_ends() {
    // A menu opens on the press and chooses on the release, which goes to
    // it: the trigger's press ends (it shows pressed no longer) without a
    // click or a release of its own, open or closed.
    let mut ui = Ui::new(300., 200.);
    let open = ui.signal(false);
    let (trigger, clicks, ups) = counting(&open);
    let view = ui.mount(
        column().child(menu("Actions", open.clone(), trigger).children([menu_item("Alpha")])),
    );
    settle(&mut ui);
    let b = ui.scene.borrow().bounds(view.find("anchor").unwrap());
    click(&mut ui, b.x + 10., b.y + 10.);
    assert!(open.get(), "its press opened it");
    assert_eq!((clicks.get(), ups.get()), (0, 0));
    press(&mut ui, Key::Escape, Modifiers::default());
    assert!(!open.get());
    assert_eq!(
        (clicks.get(), ups.get()),
        (0, 0),
        "nothing fired as it ended"
    );
}

#[test]
fn a_popover_its_triggers_click_opens_runs_that_click_once() {
    // Opened on the release (the trigger's own click): its press was never
    // cancelled, and the click runs once, closing nothing more.
    let mut ui = Ui::new(300., 200.);
    let open = ui.signal(false);
    let (trigger, clicks, ups) = counting(&open);
    let view = ui
        .mount(column().child(
            popover("Details", open.clone(), trigger).child(button().child(text("Inside"))),
        ));
    settle(&mut ui);
    let b = ui.scene.borrow().bounds(view.find("anchor").unwrap());
    click(&mut ui, b.x + 10., b.y + 10.);
    assert!(open.get());
    assert_eq!((clicks.get(), ups.get()), (1, 1));
    press(&mut ui, Key::Escape, Modifiers::default());
    assert!(!open.get());
    assert_eq!((clicks.get(), ups.get()), (1, 1), "no second handler");
}
