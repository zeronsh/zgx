//! `View::context_menu_open`: a view's context menu says whether it shows,
//! true as it opens and false however it closes; written false, it closes.
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    reactive::Signal,
    widgets::Ui,
};

fn click(ui: &mut Ui, x: f32, y: f32, button: PointerButton) {
    ui.dispatch(InputEvent::PointerDown { x, y, button });
    ui.dispatch(InputEvent::PointerUp { x, y, button });
    ui.prepare_frame();
    ui.prepare_frame();
}
fn key(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
    ui.prepare_frame();
}
/// A page (400 by 300) whose menu says whether it shows in `open`, and the
/// handle of what was mounted.
fn page(ui: &mut Ui, open: Signal<bool>) -> zgx::compose::ViewHandle {
    let view = ui.mount(
        div()
            .id("page")
            .size(400., 300.)
            .focusable(true)
            .context_menu_open(open)
            .context_menu(|_| {
                vec![
                    menu_item("Copy").id("copy").on_click(|| {}),
                    menu_item("Select All").id("all").on_click(|| {}),
                ]
            }),
    );
    ui.prepare_frame();
    view
}
/// The menu opened with a secondary click, and its state read.
fn opened(ui: &mut Ui, open: &Signal<bool>) {
    click(ui, 100., 80., PointerButton::Secondary);
    assert!(open.get(), "true as it opens");
}

#[test]
fn escape_closes_it_and_says_so() {
    let mut ui = Ui::new(400., 300.);
    let open = ui.signal(false);
    let view = page(&mut ui, open.clone());
    opened(&mut ui, &open);
    assert!(view.find("copy").is_some());
    key(&mut ui, Key::Escape);
    assert!(!open.get(), "false as Escape closes it");
}

#[test]
fn a_pick_closes_it_and_says_so() {
    let mut ui = Ui::new(400., 300.);
    let open = ui.signal(false);
    let view = page(&mut ui, open.clone());
    opened(&mut ui, &open);
    let item = view.find("copy").expect("the menu shows");
    let b = ui.scene.borrow().bounds(item);
    click(&mut ui, b.x + 4., b.y + 4., PointerButton::Primary);
    assert!(!open.get(), "false as a pick closes it");
}

#[test]
fn a_click_outside_closes_it_and_says_so() {
    let mut ui = Ui::new(400., 300.);
    let open = ui.signal(false);
    page(&mut ui, open.clone());
    opened(&mut ui, &open);
    click(&mut ui, 390., 290., PointerButton::Primary);
    assert!(!open.get(), "false as a click outside closes it");
}

#[test]
fn tab_closes_it_and_says_so() {
    let mut ui = Ui::new(400., 300.);
    let open = ui.signal(false);
    page(&mut ui, open.clone());
    opened(&mut ui, &open);
    key(&mut ui, Key::Tab);
    assert!(!open.get(), "false as Tab closes it");
}

#[test]
fn the_window_losing_focus_closes_it_and_says_so() {
    let mut ui = Ui::new(400., 300.);
    // Menus in windows of their own, as a desktop host shows them: what its
    // window losing focus closes (`Ui::dismiss_popups`).
    ui.set_popup_screens(vec![zgx::scene::Rect::new(0., 0., 1000., 1000.)]);
    let open = ui.signal(false);
    page(&mut ui, open.clone());
    opened(&mut ui, &open);
    // What a host does as its window loses focus.
    ui.dismiss_popups();
    ui.prepare_frame();
    assert!(!open.get(), "false as the window loses focus");
}

#[test]
fn written_false_it_closes_and_the_view_going_says_so() {
    let mut ui = Ui::new(400., 300.);
    let open = ui.signal(false);
    let view = page(&mut ui, open.clone());
    opened(&mut ui, &open);
    open.set(false);
    ui.prepare_frame();
    // Shown: there for the pointer where it is.
    let shows = |ui: &Ui, view: &zgx::compose::ViewHandle| {
        view.find("copy").is_some_and(|item| {
            let scene = ui.scene.borrow();
            let b = scene.bounds(item);
            scene
                .hit_test_all(b.x + b.width / 2., b.y + b.height / 2.)
                .contains(&item)
        })
    };
    assert!(!shows(&ui, &view), "written false, the menu closes");
    // Open again, then the view goes with it open.
    opened(&mut ui, &open);
    assert!(shows(&ui, &view), "it opens again");
    view.unmount();
    ui.prepare_frame();
    assert!(!open.get(), "false as the view goes");
}

#[test]
fn a_newer_menu_on_the_same_view_keeps_it_true() {
    let mut ui = Ui::new(400., 300.);
    let open = ui.signal(false);
    page(&mut ui, open.clone());
    opened(&mut ui, &open);
    // A second right-click while it shows: a new menu there, still open.
    click(&mut ui, 200., 150., PointerButton::Secondary);
    assert!(open.get(), "a newer menu still shows");
    key(&mut ui, Key::Escape);
    assert!(!open.get());
}
