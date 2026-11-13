//! `View::hover_delay` (Base UI's `openOnHover`, `delay`, `closeDelay`) and a submenu's safe polygon:
//! heading for an open submenu across the items between keeps it open, straying or resting out of
//! the polygon closes it, and the delays are the ones given.
use std::{cell::RefCell, rc::Rc, time::Duration};
use zgx::{
    clock::VirtualClock,
    compose::{TaskRunner, ViewHandle, prelude::*},
    input::InputEvent,
    reactive::Signal,
    scene::Rect,
    task::LocalExecutor,
    widgets::Ui,
};

const MS: Duration = Duration::from_millis(1);
/// A step past a sleep, so its task has run.
const STEP: Duration = Duration::from_millis(8);

struct App {
    ui: Ui,
    view: ViewHandle,
    executor: Rc<RefCell<LocalExecutor>>,
    clock: VirtualClock,
}

impl App {
    fn new(view: impl FnOnce(&mut Ui) -> View) -> Self {
        let clock = VirtualClock::new();
        let mut ui = Ui::new(600., 500.);
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let content = view(&mut ui);
        let view = ui.mount(provide(
            TaskRunner::from_executor(executor.clone()),
            content,
        ));
        ui.prepare_frame();
        Self {
            ui,
            view,
            executor,
            clock,
        }
    }
    /// Move the virtual clock on `by`, running each task and frame as its sleep falls due.
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
    fn bounds(&self, id: &str) -> Rect {
        let node = self.view.find(id).unwrap();
        self.ui.scene.borrow().bounds(node)
    }
    fn at(&mut self, (x, y): (f32, f32)) {
        self.ui.dispatch(InputEvent::PointerMove { x, y });
        self.ui.prepare_frame();
    }
    /// The pointer along the line to `to` from `from`, in `steps`, a frame apart.
    fn travel(&mut self, from: (f32, f32), to: (f32, f32), steps: u32) {
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            self.at((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t));
            self.wait(STEP);
        }
    }
}

fn centre(rect: Rect) -> (f32, f32) {
    (rect.x + rect.width / 2., rect.y + rect.height / 2.)
}

/// A menu open, its submenu "More" (five items) between two items of its own.
fn menu_with_submenu(delay: Option<(Duration, Duration)>) -> (App, Signal<bool>) {
    let mut child = None;
    let app = App::new(|ui| {
        let root = ui.signal(true);
        let open = ui.signal(false);
        child = Some(open.clone());
        let mut more = submenu("More", open).trigger_id("more");
        if let Some((open, close)) = delay {
            more = more.hover_delay(open, close);
        }
        let more = (1..=5).fold(more, |more, n| {
            more.child(menu_item(format!("Child {n}")).id(format!("child{n}")))
        });
        column().p(20.).child(
            menu("Actions", root, button().child(text("Actions")))
                .w(200.)
                .p(8.)
                .child(menu_item("Alpha").id("alpha"))
                .child(more)
                .child(menu_item("Beta").id("beta"))
                .child(menu_item("Gamma").id("gamma")),
        )
    });
    (app, child.unwrap())
}

/// Hover the submenu's trigger until it opens; where the pointer left it: its right part.
fn opened(app: &mut App, child: &Signal<bool>) -> (f32, f32) {
    let more = app.bounds("more");
    let from = (more.x + more.width - 6., more.y + more.height / 2.);
    app.at(from);
    app.wait(Duration::from_millis(100) + STEP);
    assert!(child.get(), "hovering the trigger opens it");
    app.ui.prepare_frame();
    from
}

#[test]
fn crossing_the_items_between_diagonally_keeps_the_submenu_open() {
    let (mut app, child) = menu_with_submenu(None);
    let from = opened(&mut app, &child);
    let (beta, last) = (app.bounds("beta"), app.bounds("child5"));
    assert!(
        last.y > beta.y,
        "the way there crosses Beta: {beta:?} {last:?}"
    );
    // On to the submenu's last item, over Beta and Gamma on the way.
    app.travel(from, centre(last), 24);
    assert!(child.get(), "still open on arriving");
    let last = app.bounds("child5");
    app.at(centre(last));
    app.wait(Duration::from_millis(500));
    assert!(child.get(), "and on its panel it stays");
}

#[test]
fn leaving_the_polygon_for_another_item_closes_the_submenu() {
    let (mut app, child) = menu_with_submenu(None);
    let from = opened(&mut app, &child);
    let beta = app.bounds("beta");
    // Straight down-left onto Beta, away from the submenu.
    app.travel(from, (beta.x + 20., beta.y + beta.height / 2.), 6);
    assert!(!child.get(), "off the way to it, it closes");
}

#[test]
fn resting_in_the_polygon_closes_the_submenu_after_the_timeout() {
    let (mut app, child) = menu_with_submenu(None);
    let from = opened(&mut app, &child);
    let beta = app.bounds("beta");
    let (panel, _) = (app.bounds("child1"), ());
    // Into the polygon over Beta, then no further.
    let rest = (beta.x + beta.width - 1., beta.y + beta.height / 2.);
    assert!(rest.0 < panel.x, "short of the submenu: {rest:?} {panel:?}");
    app.travel(from, rest, 6);
    assert!(child.get(), "held while it may be heading there");
    app.wait(Duration::from_millis(250));
    assert!(child.get(), "short of the timeout");
    app.wait(Duration::from_millis(100));
    assert!(!child.get(), "resting there, it closes");
}

#[test]
fn a_submenu_opens_and_closes_after_its_own_delays() {
    let (mut app, child) = menu_with_submenu(Some((
        Duration::from_millis(250),
        Duration::from_millis(150),
    )));
    let more = app.bounds("more");
    let from = (more.x + more.width - 6., more.y + more.height / 2.);
    app.at(from);
    app.wait(Duration::from_millis(249));
    assert!(!child.get(), "waits its open delay");
    app.wait(MS + STEP);
    assert!(child.get(), "opens after it");
    // A little more motion on the trigger, as a hand has.
    app.at((from.0 + 1., from.1));
    // Onto Beta, away from the polygon: it closes after its own delay.
    let beta = app.bounds("beta");
    app.at((beta.x + 20., beta.y + beta.height / 2.));
    app.wait(Duration::from_millis(149));
    assert!(child.get(), "waits its close delay");
    app.wait(MS + STEP);
    assert!(!child.get(), "closes after it");
}

/// A popover opening on hover over a button, its panel below.
fn hover_popover(open: Duration, close: Duration) -> (App, Signal<bool>) {
    let mut signal = None;
    let app = App::new(|ui| {
        let shown = ui.signal(false);
        signal = Some(shown.clone());
        column().p(40.).child(
            popover(
                "Card",
                shown,
                button()
                    .id("trigger")
                    .size(100., 30.)
                    .p(0.)
                    .child(text("Hover")),
            )
            .hover_delay(open, close)
            .w(160.)
            .h(100.)
            .p(0.)
            .child(div().id("card").w(160.).h(100.)),
        )
    });
    (app, signal.unwrap())
}

#[test]
fn a_popover_opens_after_the_pointer_rests_and_closes_after_it_leaves() {
    let (mut app, shown) = hover_popover(Duration::from_millis(200), Duration::from_millis(100));
    let trigger = app.bounds("trigger");
    app.at(centre(trigger));
    app.wait(Duration::from_millis(199));
    assert!(!shown.get(), "rests its delay first");
    app.wait(MS + STEP);
    assert!(shown.get(), "then opens");
    // Away from the trigger and the panel.
    app.at((550., 450.));
    app.wait(Duration::from_millis(99));
    assert!(shown.get(), "stays through its close delay");
    app.wait(MS + STEP);
    assert!(!shown.get(), "then closes");
}

#[test]
fn passing_over_the_trigger_opens_nothing() {
    let (mut app, shown) = hover_popover(Duration::from_millis(200), Duration::ZERO);
    let trigger = app.bounds("trigger");
    app.at(centre(trigger));
    app.wait(Duration::from_millis(100));
    app.at((550., 450.));
    app.wait(Duration::from_millis(400));
    assert!(!shown.get(), "gone before it rested");
}

#[test]
fn heading_for_the_popover_keeps_it_open() {
    let (mut app, shown) = hover_popover(Duration::ZERO, Duration::ZERO);
    let trigger = app.bounds("trigger");
    app.at(centre(trigger));
    app.wait(STEP);
    assert!(shown.get());
    let card = app.bounds("card");
    // Down and across over the gap between them, onto the panel.
    app.travel(
        (
            trigger.x + trigger.width - 10.,
            trigger.y + trigger.height - 2.,
        ),
        centre(card),
        12,
    );
    assert!(shown.get(), "held while it crosses");
    app.wait(Duration::from_millis(500));
    assert!(shown.get(), "and on the panel");
    // Out of the panel: closed.
    app.at((550., 450.));
    app.wait(STEP);
    assert!(!shown.get());
}

#[test]
fn a_popover_opened_another_way_stays_open() {
    let (mut app, shown) = hover_popover(Duration::from_millis(100), Duration::ZERO);
    shown.set(true);
    app.wait(STEP);
    let trigger = app.bounds("trigger");
    app.at(centre(trigger));
    app.at((550., 450.));
    app.wait(Duration::from_millis(500));
    assert!(shown.get(), "only what hovering opened closes on leaving");
}
