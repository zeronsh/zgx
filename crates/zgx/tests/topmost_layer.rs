//! A topmost layer (`View::topmost`): where the host shows popups in windows of its own it is a
//! popup root of the `Topmost` role, filling the window; the pointer passes through it but for
//! the controls it holds, and a press on one closes no popover. Elsewhere it is the window's top
//! layer.
use std::{cell::Cell, rc::Rc};
use zgx::{
    compose::{ViewHandle, prelude::*},
    input::{InputEvent, PointerButton},
    reactive::Signal,
    scene::{NodeId, PopupRole, Rect},
    widgets::Ui,
};

struct App {
    ui: Ui,
    view: ViewHandle,
    under: Rc<Cell<u32>>,
    held: Rc<Cell<u32>>,
    popover: Signal<bool>,
    ghost: Signal<bool>,
}

/// `windows`: as the desktop host on macOS shows popups, each in a window of its own.
fn app(windows: bool) -> App {
    let mut ui = Ui::new(600., 400.);
    if windows {
        ui.set_popup_screens(vec![Rect::new(0., 0., 1000., 800.)]);
        ui.set_topmost_windows(true);
    }
    let (under, held) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (popover, ghost) = (ui.signal(false), ui.signal(true));
    let view = ui.mount(
        column()
            .p(20.)
            .child(popover_over(&popover))
            .child({
                let under = under.clone();
                button()
                    .id("under")
                    .size(120., 40.)
                    .p(0.)
                    .on_click(move || under.set(under.get() + 1))
                    .child(text("Under"))
            })
            .child(
                modal("Ghost", ghost.clone())
                    .topmost()
                    .child(
                        column()
                            .id("dot")
                            .absolute()
                            .left(20.)
                            .top(20.)
                            .size(120., 40.)
                            .pointer_events(false),
                    )
                    .child({
                        let held = held.clone();
                        button()
                            .id("held")
                            .absolute()
                            .left(300.)
                            .top(300.)
                            .size(80., 30.)
                            .p(0.)
                            .on_click(move || held.set(held.get() + 1))
                            .child(text("Held"))
                    }),
            ),
    );
    App {
        ui,
        view,
        under,
        held,
        popover,
        ghost,
    }
}

fn popover_over(open: &Signal<bool>) -> View {
    popover(
        "Menu",
        open.clone(),
        button().id("open").size(60., 20.).p(0.).child(text("Open")),
    )
    .lazy()
    .w(200.)
    .h(100.)
    .p(0.)
    .child(text("panel"))
}

impl App {
    fn node(&self, id: &str) -> NodeId {
        self.view.find(id).unwrap()
    }
    fn center(&self, id: &str) -> (f32, f32) {
        let bounds = self.ui.scene.borrow().bounds(self.node(id));
        (bounds.x + bounds.width / 2., bounds.y + bounds.height / 2.)
    }
    fn press(&mut self, (x, y): (f32, f32)) {
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
            self.ui.dispatch(event);
        }
        self.ui.prepare_frame();
    }
    fn topmost_roots(&self) -> Vec<NodeId> {
        let scene = self.ui.scene.borrow();
        let roots = scene.popups();
        roots
            .iter()
            .copied()
            .filter(|root| scene.popup_role(*root) == Some(PopupRole::Topmost))
            .collect()
    }
}

#[test]
fn a_topmost_layer_is_a_popup_root_filling_the_window() {
    let app = app(true);
    app.ui.prepare_frame();
    let roots = app.topmost_roots();
    assert_eq!(roots.len(), 1, "one topmost popup root");
    let scene = app.ui.scene.borrow();
    let bounds = scene.bounds(roots[0]);
    assert_eq!(
        (bounds.x, bounds.y),
        (0., 0.),
        "its root at the window's 0, 0"
    );
    assert_eq!(
        (bounds.width, bounds.height),
        (600., 400.),
        "over the content"
    );
    assert!(
        scene.popup_bounds(roots[0]).is_some(),
        "it paints something"
    );
}

#[test]
fn without_windows_for_popups_it_is_the_windows_top_layer() {
    let mut app = app(false);
    app.ui.prepare_frame();
    assert!(app.topmost_roots().is_empty());
    assert!(app.ui.scene.borrow().popups().is_empty());
    // Still live in the window.
    let held = app.center("held");
    app.press(held);
    assert_eq!(app.held.get(), 1);
}

#[test]
fn it_is_a_popup_only_while_open() {
    let app = app(true);
    app.ui.prepare_frame();
    assert_eq!(app.topmost_roots().len(), 1);
    app.ghost.set(false);
    app.ui.prepare_frame();
    assert!(app.topmost_roots().is_empty(), "closed, no window");
    app.ghost.set(true);
    app.ui.prepare_frame();
    assert_eq!(app.topmost_roots().len(), 1, "open again");
}

#[test]
fn the_pointer_passes_through_but_for_its_controls() {
    for windows in [true, false] {
        let mut app = app(windows);
        app.ui.prepare_frame();
        // Under the dot (a pointer-events-none child) and its blank layer.
        let under = app.center("under");
        app.press(under);
        assert_eq!(app.under.get(), 1, "{windows}: the button beneath takes it");
        assert_eq!(app.held.get(), 0);
        // On the dot, over the button beneath it: still through.
        let dot = app.center("dot");
        let under_dot = app.ui.scene.borrow().bounds(app.node("under"));
        assert!(
            dot.0 > under_dot.x && dot.0 < under_dot.x + under_dot.width,
            "the dot is over the button"
        );
        app.press(dot);
        assert_eq!(app.under.get(), 2, "{windows}: a dot with no events");
        // Its own control takes the press.
        let held = app.center("held");
        app.press(held);
        assert_eq!(app.held.get(), 1, "{windows}");
        assert_eq!(app.under.get(), 2);
    }
}

#[test]
fn a_press_on_a_control_in_it_closes_no_popover() {
    let mut app = app(true);
    app.ui.prepare_frame();
    app.popover.set(true);
    app.ui.prepare_frame();
    let held = app.center("held");
    app.press(held);
    assert_eq!(app.held.get(), 1);
    assert!(app.popover.get(), "the app's control, not outside it");
    // Elsewhere in the window, it does close.
    let blank = (500., 150.);
    app.press(blank);
    assert!(!app.popover.get());
}

#[test]
fn of_two_the_later_is_listed_after() {
    let mut ui = Ui::new(300., 200.);
    ui.set_popup_screens(vec![Rect::new(0., 0., 1000., 800.)]);
    ui.set_topmost_windows(true);
    let (one, two) = (ui.signal(true), ui.signal(true));
    let view = ui.mount(
        column()
            .child(modal("One", one).topmost().child(column().id("one")))
            .child(modal("Two", two).topmost().child(column().id("two"))),
    );
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let held_by = |id: &str| {
        let node = view.find(id).unwrap();
        scene
            .popups()
            .iter()
            .position(|root| *root == node || scene.ancestors(node).any(|up| up == *root))
            .unwrap_or_else(|| panic!("{id} is in a popup root"))
    };
    assert!(held_by("one") < held_by("two"), "the later is above");
}

#[test]
fn components_in_a_topmost_layer_or_popover_see_the_windows_services() {
    let mut ui = Ui::new(400., 300.);
    ui.set_popup_screens(vec![Rect::new(0., 0., 1000., 800.)]);
    ui.set_topmost_windows(true);
    let (layer, shown) = (ui.signal(true), ui.signal(true));
    let seen = Rc::new(Cell::new(0));
    let (a, b) = (seen.clone(), seen.clone());
    ui.mount(provide(
        zgx::frame::FrameClock::new(),
        column()
            .child(
                popover("Card", shown, button().child(text("Open")))
                    .lazy()
                    .w(100.)
                    .h(40.)
                    .child(component(move |cx| {
                        let _frames = cx.frames();
                        a.set(a.get() + 1);
                        column()
                    })),
            )
            .child(modal("Ghost", layer).topmost().child(component(move |cx| {
                let _frames = cx.frames();
                b.set(b.get() + 1);
                column()
            }))),
    ));
    ui.prepare_frame();
    assert_eq!(seen.get(), 2, "both asked for the frame clock and got it");
}
