use std::{cell::RefCell, rc::Rc, time::Duration};
use zgx::{
    clock::VirtualClock,
    compose::{TaskRunner, Tooltip, prelude::*},
    input::{InputEvent, Key, Modifiers},
    scene::NodeKind,
    task::LocalExecutor,
    widgets::Ui,
};

/// Whether text `label` shows (laid out, on screen, not faded out).
fn showing(ui: &Ui, label: &str) -> bool {
    let scene = ui.scene.borrow();
    scene.paint_items().any(|item| {
        matches!(scene.kind(item.id), NodeKind::Text { text, .. } if &**text == label)
            && item.effects.opacity > 0.
            && scene.visible_bounds(item.id).is_some_and(|b| b.width > 0.)
    })
}

fn hover(ui: &mut Ui, x: f32, y: f32) {
    ui.dispatch(InputEvent::PointerMove { x, y });
    ui.prepare_frame();
}

fn key(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
}

#[test]
fn a_tooltip_shows_on_hover_and_goes_as_the_pointer_leaves_or_a_key_is_pressed() {
    // No task runner: no delays or fades, it shows at once.
    let mut ui = Ui::new(300., 200.);
    ui.mount(
        column()
            .p(60.)
            .gap(20.)
            .child(
                button()
                    .size(60., 24.)
                    .child(text("Copy"))
                    .tooltip("Copy link"),
            )
            .child(div().size(60., 24.)),
    );
    ui.prepare_frame();
    assert!(!showing(&ui, "Copy link"));
    hover(&mut ui, 70., 70.);
    assert!(showing(&ui, "Copy link"), "over it");
    hover(&mut ui, 70., 110.);
    assert!(!showing(&ui, "Copy link"), "gone as the pointer leaves");
    hover(&mut ui, 70., 70.);
    assert!(showing(&ui, "Copy link"));
    key(&mut ui, Key::Character("x".into()));
    assert!(!showing(&ui, "Copy link"), "a key closes it");
}

#[test]
fn a_tooltip_waits_for_the_pointer_to_rest_and_an_interactive_card_takes_typing() {
    let clock = VirtualClock::new();
    let timing = zgx::compose::TooltipTiming::default();
    let mut ui = Ui::new(300., 200.);
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        column()
            .p(60.)
            .gap(20.)
            .child(div().size(60., 24.).tooltip("Plain"))
            .child(div().size(60., 24.).tooltip(
                Tooltip::from(column().size(80., 40.).child(text("Card"))).interactive(true),
            )),
    ));
    ui.prepare_frame();
    hover(&mut ui, 70., 70.);
    assert!(!showing(&ui, "Plain"), "not at once");
    wait(&mut ui, &executor, &clock, timing.delay + STEP);
    assert!(showing(&ui, "Plain"), "after a rest");
    // Within a second of it, the next opens at once (on Linux after GTK's
    // short reshow wait): before a rest would have shown it.
    hover(&mut ui, 70., 110.);
    wait(&mut ui, &executor, &clock, timing.reshow_delay + STEP);
    assert!(showing(&ui, "Card"), "at once");
    assert!(timing.reshow_delay + STEP < timing.delay);
    // Its fade out: 100 ms.
    wait(&mut ui, &executor, &clock, FADE_OUT);
    assert!(!showing(&ui, "Plain"), "the last faded out");
    key(&mut ui, Key::Character("x".into()));
    assert!(showing(&ui, "Card"), "a card takes typing");
    key(&mut ui, Key::Escape);
    wait(&mut ui, &executor, &clock, FADE_OUT);
    assert!(!showing(&ui, "Card"), "and closes on Escape");
}

#[test]
fn a_tooltip_describes_its_view_and_shows_on_keyboard_focus() {
    use zgx::{input::PointerButton, semantics::Role};
    let mut ui = Ui::new(300., 200.);
    let view = ui.mount(
        column()
            .p(60.)
            .gap(20.)
            .child(
                button()
                    .id("copy")
                    .size(60., 24.)
                    .child(text("Copy"))
                    .tooltip("Copy link"),
            )
            .child(button().id("other").size(60., 24.).child(text("Other"))),
    );
    ui.prepare_frame();
    let copy = view.find("copy").unwrap();
    {
        let semantics = ui.semantics.borrow();
        let node = semantics.get(copy).unwrap();
        assert_eq!(node.role, Role::Button);
        assert!(!node.hidden, "the view itself stays");
        assert_eq!(node.description.as_deref(), Some("Copy link"));
    }
    // Tabbed to, it shows; tabbed past, it goes.
    key(&mut ui, Key::Tab);
    assert_eq!(ui.input.focused(), Some(copy));
    assert!(showing(&ui, "Copy link"), "shown on keyboard focus");
    key(&mut ui, Key::Tab);
    assert!(!showing(&ui, "Copy link"), "gone as focus leaves");
    // Its tip is not in the accessibility tree: no dialog of its own.
    key(&mut ui, Key::Tab);
    key(&mut ui, Key::Tab);
    let dialogs = ui
        .semantics
        .borrow()
        .iter()
        .filter(|(_, n)| n.role == Role::Dialog && !n.hidden)
        .count();
    assert_eq!(dialogs, 0);
    // A click focuses it without showing it (the press hides it anyway).
    let b = ui.scene.borrow().bounds(view.find("other").unwrap());
    let (x, y) = (b.x + 5., b.y + 5.);
    ui.dispatch(InputEvent::PointerMove { x, y });
    let button = PointerButton::Primary;
    ui.dispatch(InputEvent::PointerDown { x, y, button });
    ui.dispatch(InputEvent::PointerUp { x, y, button });
    ui.prepare_frame();
    assert!(!showing(&ui, "Copy link"));
}

#[test]
fn a_popovers_trigger_says_it_opens_a_dialog_and_whether_it_shows() {
    use zgx::semantics::{PopupKind, Role};
    let mut ui = Ui::new(400., 300.);
    let open = ui.signal(false);
    let view = ui.mount(
        column().p(20.).child(
            popover(
                "Filters",
                open.clone(),
                button().id("trigger").size(80., 28.),
            )
            .w(200.)
            .child(text("Panel")),
        ),
    );
    ui.prepare_frame();
    let trigger = view.find("trigger").unwrap();
    let state = |ui: &Ui| {
        let semantics = ui.semantics.borrow();
        let node = semantics.get(trigger).unwrap();
        (node.has_popup, node.expanded)
    };
    open.set(true);
    ui.prepare_frame();
    assert_eq!(state(&ui), (Some(PopupKind::Dialog), Some(true)));
    let modal = ui
        .semantics
        .borrow()
        .iter()
        .any(|(_, n)| n.role == Role::Dialog && n.modal);
    assert!(
        modal,
        "it traps focus, so it is modal to assistive technology"
    );
    open.set(false);
    ui.prepare_frame();
    assert_eq!(state(&ui), (Some(PopupKind::Dialog), Some(false)));
}

#[test]
fn a_menu_matching_its_trigger_is_as_wide_as_it() {
    let mut ui = Ui::new(500., 300.);
    let open = ui.signal(true);
    let view = ui.mount(
        column().p(20.).child(
            menu("Size", open, button().id("trigger").size(220., 28.))
                .id("menu")
                .match_anchor_width()
                .child(menu_item("S")),
        ),
    );
    ui.prepare_frame();
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let (trigger, panel) = (
        scene.bounds(view.find("trigger").unwrap()),
        scene.bounds(view.find("menu").unwrap()),
    );
    assert!(panel.width >= trigger.width, "{panel:?} {trigger:?}");
}

/// Two views with tooltips, with a task runner on a virtual clock: a busy
/// machine's late wake can't pass for a wait.
fn two_tooltips() -> (Ui, Rc<RefCell<LocalExecutor>>, VirtualClock) {
    let clock = VirtualClock::new();
    let mut ui = Ui::new(300., 200.);
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        column()
            .p(60.)
            .gap(20.)
            .child(div().size(60., 24.).tooltip("First"))
            .child(div().size(60., 24.).tooltip("Second")),
    ));
    ui.prepare_frame();
    (ui, executor, clock)
}

/// Move `clock` on `by`, running each task and frame as its sleep falls due.
fn wait(ui: &mut Ui, executor: &RefCell<LocalExecutor>, clock: &VirtualClock, by: Duration) {
    let mut settle = || {
        while executor.borrow_mut().tick() > 0 {}
        ui.prepare_frame();
    };
    // What was asked for starts its sleeps now.
    settle();
    clock.advance(clock.elapsed() + by, &mut settle);
    settle();
}

/// A fade's first step.
const STEP: Duration = Duration::from_millis(8);
/// A fade out, to its last step.
const FADE_OUT: Duration = Duration::from_millis(100 + 8);
const MS: Duration = Duration::from_millis(1);

/// The system's timing (`set_tooltip_timing`, which a desktop host sets:
/// Windows' from its hover time): a reshow within the window waits its own
/// delay rather than none, a single timer.
#[test]
fn a_reshow_waits_as_the_systems_timing_says() {
    use zgx::compose::{TooltipTiming, set_tooltip_timing};
    set_tooltip_timing(TooltipTiming {
        delay: Duration::from_millis(100),
        reshow_within: Duration::from_secs(1),
        reshow_delay: Duration::from_millis(400),
        reshow_from_open: false,
    });
    let (mut ui, executor, clock) = two_tooltips();
    hover(&mut ui, 70., 70.);
    wait(
        &mut ui,
        &executor,
        &clock,
        Duration::from_millis(100) + STEP,
    );
    assert!(showing(&ui, "First"));
    hover(&mut ui, 70., 110.);
    wait(&mut ui, &executor, &clock, Duration::from_millis(400) - MS);
    assert!(!showing(&ui, "Second"), "a reshow waits its delay");
    wait(&mut ui, &executor, &clock, MS + STEP);
    assert!(showing(&ui, "Second"), "then shows");
    set_tooltip_timing(TooltipTiming::default());
}

/// By default (`TooltipTiming::default`) a tooltip waits as its platform's
/// do: on Linux as GTK's (500 ms; within 500 ms of another closing, 60),
/// elsewhere zgx's (300 ms; at once within a second): the second of two
/// reshows after a wait there, at once here.
#[test]
fn a_reshow_waits_as_its_platforms_do() {
    // (delay, reshow delay): GTK's on Linux, zgx's elsewhere.
    let (delay, reshow_delay) = match cfg!(target_os = "linux") {
        true => (500, 60),
        false => (300, 0),
    };
    let (delay, reshow_delay) = (
        Duration::from_millis(delay),
        Duration::from_millis(reshow_delay),
    );
    let (mut ui, executor, clock) = two_tooltips();
    hover(&mut ui, 70., 70.);
    wait(&mut ui, &executor, &clock, delay - MS);
    assert!(!showing(&ui, "First"), "waits its delay");
    wait(&mut ui, &executor, &clock, MS + STEP);
    assert!(showing(&ui, "First"), "shows after it");
    hover(&mut ui, 70., 110.);
    // At once (zgx's) shows within a frame; GTK's 60 ms waits a little.
    let at_once = showing(&ui, "Second");
    assert_eq!(
        at_once,
        !cfg!(target_os = "linux"),
        "a reshow at once only off Linux"
    );
    if !at_once {
        wait(&mut ui, &executor, &clock, reshow_delay - MS);
        assert!(!showing(&ui, "Second"), "waits its reshow delay");
    }
    wait(&mut ui, &executor, &clock, MS + STEP);
    assert!(showing(&ui, "Second"), "then shows");
}

/// A menu and a tooltip in popup windows, as a desktop host has them.
fn with_popup_windows() -> (Ui, zgx::reactive::Signal<bool>, Rc<RefCell<LocalExecutor>>) {
    let mut ui = Ui::new(400., 300.);
    let open = ui.signal(false);
    ui.set_popup_screens(vec![zgx::scene::Rect::new(0., 0., 1000., 1000.)]);
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        column()
            .p(20.)
            .gap(20.)
            .child(
                menu("Menu", open.clone(), button().size(60., 24.))
                    .w(100.)
                    .child(menu_item("Item")),
            )
            .child(div().size(60., 24.).tooltip("Tip")),
    ));
    ui.prepare_frame();
    (ui, open, executor)
}

#[test]
fn a_popup_wanted_while_the_app_is_at_the_back_waits_for_it_to_come_forward() {
    let (ui, open, _executor) = with_popup_windows();
    ui.set_app_active(false);
    open.set(true);
    ui.prepare_frame();
    {
        let scene = ui.scene.borrow();
        assert!(scene.popups().is_empty(), "wanted, not shown");
        // Nor is it there for the pointer, where it would be.
        let root = view_root_of_open_popup(&ui);
        let b = scene.bounds(root);
        assert!(
            !scene
                .hit_test_all(b.x + b.width / 2., b.y + b.height / 2.)
                .contains(&root)
        );
    }
    assert!(open.get(), "the app's state isn't changed");
    ui.set_app_active(true);
    ui.prepare_frame();
    assert_eq!(
        ui.scene.borrow().popups().len(),
        1,
        "shows as it comes forward"
    );
    assert!(open.get());
    // Going to the back again: what is open closes, as ever, and stays closed.
    ui.dismiss_popups();
    ui.set_app_active(false);
    ui.prepare_frame();
    assert!(!open.get());
    ui.set_app_active(true);
    ui.prepare_frame();
    assert!(
        ui.scene.borrow().popups().is_empty(),
        "no flash of it coming back"
    );
}

/// The open popup's root, held or not.
fn view_root_of_open_popup(ui: &Ui) -> zgx::scene::NodeId {
    let scene = ui.scene.borrow();
    let mut pending = vec![scene.root()];
    while let Some(node) = pending.pop() {
        if scene.is_popup(node) {
            return node;
        }
        pending.extend(scene.children(node).iter().copied());
    }
    panic!("no popup");
}

#[test]
fn a_tooltip_at_the_back_does_not_show_until_the_app_comes_forward() {
    let clock = VirtualClock::new();
    let (mut ui, _open, executor) = with_popup_windows();
    ui.set_app_active(false);
    hover(&mut ui, 40., 80.);
    wait(&mut ui, &executor, &clock, Duration::from_millis(900));
    assert!(
        ui.scene.borrow().popups().is_empty(),
        "under the pointer, at the back"
    );
    ui.set_app_active(true);
    ui.prepare_frame();
    assert_eq!(ui.scene.borrow().popups().len(), 1);
    assert_eq!(
        ui.scene.borrow().popup_role(ui.scene.borrow().popups()[0]),
        Some(zgx::scene::PopupRole::Tooltip)
    );
}
