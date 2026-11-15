//! A measured list following its end that glides there
//! (`VariableHeights::glide_end`): its last row growing (a reply being
//! written) moves the view to the end on a spring, not in one jump; it says
//! it rests at its end all the while; a scroll stops it where it is; rows
//! growing above the view move nothing in it; and unless it's wanted the
//! list jumps to its end as `anchor_end` does.
use std::{cell::RefCell, rc::Rc, time::Duration};
use zgx::{
    clock::VirtualClock,
    compose::{ViewHandle, prelude::*},
    input::InputEvent,
    reactive::Signal,
    widgets::Ui,
};

const ROWS: usize = 21;
const LAST: usize = ROWS - 1;

struct Chat {
    ui: Ui,
    clock: VirtualClock,
    ran: Duration,
    offset: Signal<f32>,
    heights: VariableHeights,
    view: ViewHandle,
    /// The first row's height, and the last's (the reply).
    head: Signal<f32>,
    tail: Signal<f32>,
    /// Whether it glides now.
    wanted: Signal<bool>,
    /// What `on_at_end` said, in order.
    told: Rc<RefCell<Vec<bool>>>,
}
impl Chat {
    /// Rows 60 tall (the first `head`, the last `tail`) in a 400-point list,
    /// resting at its end, gliding wanted from then on (as an app wants it
    /// while a reply is written, not as a list opens).
    fn new() -> Self {
        let clock = VirtualClock::new();
        let mut ui = Ui::new(600., 500.);
        let offset = ui.signal(f32::MAX);
        let (head, tail, wanted) = (ui.signal(60_f32), ui.signal(60_f32), ui.signal(false));
        let when = wanted.clone();
        let heights = VariableHeights::new(&ui.runtime, ROWS, 40.)
            .glide_end(move || when.with_untracked(|w| *w));
        let told = Rc::new(RefCell::new(Vec::new()));
        let tell = told.clone();
        let (h, t) = (head.clone(), tail.clone());
        let view = ui.mount(
            measured_virtual_list(
                offset.clone(),
                heights.clone(),
                1,
                |index| index,
                move |_, index, _| {
                    let height = match index {
                        0 => Some(h.clone()),
                        LAST => Some(t.clone()),
                        _ => None,
                    };
                    match height {
                        Some(height) => div()
                            .reactive_style(move || Styles::new().h(height.get()))
                            .id(format!("row-{index}")),
                        None => div().h(60.).id(format!("row-{index}")),
                    }
                },
            )
            .on_at_end(move |at_end| tell.borrow_mut().push(at_end))
            .size(500., 400.),
        );
        let mut chat = Self {
            ui,
            clock,
            ran: Duration::ZERO,
            offset,
            heights,
            view,
            head,
            tail,
            wanted,
            told,
        };
        // Settled (its rows measured, at its end), then a reply is written.
        chat.run(100);
        chat.wanted.set(true);
        chat.told.borrow_mut().clear();
        chat.told.borrow_mut().push(true);
        chat
    }
    fn frame(&mut self) {
        self.ui.prepare_frame();
    }
    /// Frames of 8 ms for `ms`, the clock with them.
    fn run(&mut self, ms: u64) {
        for _ in 0..ms / 8 {
            self.ran += Duration::from_millis(8);
            self.clock.advance(self.ran, || {});
            self.ui.advance_interactions(self.clock.now()).unwrap();
            self.ui.prepare_frame();
        }
    }
    /// How far short of its end the view is.
    fn short(&self) -> f32 {
        (self.heights.content_height() - 400.) - self.offset.get()
    }
    /// Whether it last said it rests at its end.
    fn at_end(&self) -> bool {
        self.told.borrow().last().copied().unwrap_or(false)
    }
    /// Row `index`'s top in the window, if it shows.
    fn top(&self, index: usize) -> Option<f32> {
        let scene = self.ui.scene.borrow();
        self.view
            .find(&format!("row-{index}"))
            .and_then(|id| scene.contains(id).then(|| scene.transformed_bounds(id).y))
    }
}

#[test]
fn the_last_row_growing_glides_the_view_to_the_end() {
    let mut chat = Chat::new();
    assert!(chat.short().abs() < 0.5, "at its end: {}", chat.short());
    assert!(chat.at_end());
    chat.tail.set(180.);
    chat.frame();
    // A frame on, what showed is where it was; the end is 120 below.
    assert!(
        (chat.short() - 120.).abs() < 1.,
        "nothing moved yet: {} short",
        chat.short()
    );
    assert!(chat.at_end(), "still resting at its end, as it says");
    // Then it goes, a little each frame, never back, never past.
    let mut was = chat.short();
    let mut steps = 0;
    for _ in 0..190 {
        chat.run(8);
        let now = chat.short();
        assert!(now <= was + 0.01, "never back: {was} then {now}");
        assert!(now >= -0.01, "never past the end: {now}");
        if now < was - 0.01 {
            steps += 1;
        }
        was = now;
    }
    assert!(steps >= 5, "glided over several frames, not one: {steps}");
    assert!(
        chat.short().abs() < 0.5,
        "landed on the end: {}",
        chat.short()
    );
    assert!(
        chat.told.borrow().iter().all(|at| *at),
        "said it rests at its end throughout: {:?}",
        chat.told.borrow()
    );
}

#[test]
fn not_wanted_it_jumps_to_the_end() {
    let mut chat = Chat::new();
    chat.wanted.set(false);
    chat.tail.set(180.);
    chat.frame();
    chat.frame();
    assert!(
        chat.short().abs() < 0.5,
        "at the end at once: {}",
        chat.short()
    );
}

#[test]
fn a_scroll_stops_the_glide_where_it_is() {
    let mut chat = Chat::new();
    chat.tail.set(400.);
    chat.frame();
    chat.run(48);
    let gliding = chat.short();
    assert!(gliding > 1., "part way: {gliding} short");
    chat.ui.dispatch(InputEvent::Scroll {
        x: 100.,
        y: 100.,
        delta_x: 0.,
        delta_y: -100.,
    });
    chat.frame();
    let left = chat.offset.get();
    chat.run(600);
    assert_eq!(chat.offset.get(), left, "stays where the scroll took it");
    assert!(!chat.at_end(), "away from its end");
    // More written meanwhile pulls nothing.
    chat.tail.set(600.);
    chat.run(300);
    assert_eq!(chat.offset.get(), left);
}

#[test]
fn a_row_growing_above_the_view_moves_nothing_in_it() {
    let mut chat = Chat::new();
    let bottom = chat.top(LAST).expect("the last row shows");
    assert!(chat.top(0).is_none(), "the first row is far above the view");
    chat.head.set(120.);
    chat.run(16);
    // (The first row isn't built: grown, it's measured as it comes back.
    // Its height is set as the list would have it.)
    chat.heights.set_height(0, 120.).unwrap();
    chat.frame();
    chat.run(200);
    assert_eq!(chat.top(LAST), Some(bottom), "the last row where it was");
    assert!(chat.short().abs() < 0.5, "still at the end");
}

#[test]
fn screens_from_the_end_it_jumps_to_a_screen_short_then_glides() {
    let mut chat = Chat::new();
    // 2,000 points more: five screens.
    chat.tail.set(2060.);
    chat.frame();
    assert!(
        (chat.short() - 400.).abs() < 1.,
        "a screen short of the end: {}",
        chat.short()
    );
    chat.run(1500);
    assert!(
        chat.short().abs() < 0.5,
        "then glided there: {}",
        chat.short()
    );
    assert!(chat.at_end());
}
