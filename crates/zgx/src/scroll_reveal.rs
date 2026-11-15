//! A node scrolled into view as the web's `element.scrollIntoView` scrolls
//! one (`NodeHandle::scroll_into_view`): every scroll view around it,
//! innermost first, moves so it shows where asked along each axis, and an
//! animated reveal eases there over a few frames (none under the system's
//! reduced motion, unless asked).
//!
//! What moves is worked out from the layout as it stands, each scroll view's
//! offset and range read from its content's place in it, before anything
//! moves: a script's handler, whose changes wait for its batch to end, knows
//! at once whether any did.
use crate::{
    compose::RowAlign,
    input::InputEvent,
    scene::{NodeId, Rect},
    time::Instant,
    widgets::Ui,
};
use std::time::Duration;

/// How long an animated reveal takes, easing in and out.
const DURATION: Duration = Duration::from_millis(300);

/// How `NodeHandle::scroll_into_view` shows a node: where along each axis
/// (`block` down, `inline` across), and whether it eases there.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollIntoView {
    /// Down: `Start` by default, as the web's.
    pub block: RowAlign,
    /// Across: `Nearest` by default, as the web's.
    pub inline: RowAlign,
    /// Whether it eases there: by default, unless the system reduces motion.
    pub animate: Option<bool>,
}
impl Default for ScrollIntoView {
    fn default() -> Self {
        Self {
            block: RowAlign::Start,
            inline: RowAlign::Nearest,
            animate: None,
        }
    }
}

/// How far a scroll view showing `from` to `from + length` scrolls so a box
/// at `start`, `size` long, shows as `align` asks; `Nearest` with `room`
/// to spare where it moves (none for the web's alignment, a little for a
/// screen reader's).
pub(crate) fn shift(
    align: RowAlign,
    start: f32,
    size: f32,
    from: f32,
    length: f32,
    room: f32,
) -> f32 {
    match align {
        RowAlign::Nearest => {
            if start < from {
                (start - from - room).max(start + size - from - length)
            } else if start + size > from + length {
                (start + size - from - length + room).min(start - from)
            } else {
                0.
            }
        }
        RowAlign::Start => start - from,
        RowAlign::End => start + size - from - length,
        RowAlign::Center => start + size / 2. - from - length / 2.,
    }
}

/// What each scroll view around `node` moves by (across, down) to show
/// `rect` (the window's points) as `how` asks, innermost first, within the
/// range each can scroll: none for one already showing it so.
fn plan(ui: &Ui, node: NodeId, mut rect: Rect, how: &ScrollIntoView) -> Vec<(NodeId, f32, f32)> {
    let scene = ui.scene.borrow();
    let mut moves = Vec::new();
    let contents: Vec<NodeId> = scene
        .ancestors(node)
        .filter(|id| scene.is_scroll_content(*id))
        .collect();
    for content in contents {
        let Some(viewport) = scene.parent(content) else {
            continue;
        };
        // Its own box, clipped or not (an inner view below what an outer one
        // shows moves too, as the web's); its offset is how far its content
        // sits before it, its range what of the content doesn't fit.
        let (frame, inner) = (
            scene.transformed_bounds(viewport),
            scene.transformed_bounds(content),
        );
        let clamp = |delta: f32, offset: f32, limit: f32| {
            (offset + delta).clamp(0., limit.max(0.)) - offset
        };
        let dx = clamp(
            shift(how.inline, rect.x, rect.width, frame.x, frame.width, 0.),
            frame.x - inner.x,
            inner.width - frame.width,
        );
        let dy = clamp(
            shift(how.block, rect.y, rect.height, frame.y, frame.height, 0.),
            frame.y - inner.y,
            inner.height - frame.height,
        );
        if dx.abs() < 0.01 && dy.abs() < 0.01 {
            continue;
        }
        moves.push((viewport, dx, dy));
        rect = Rect::new(rect.x - dx, rect.y - dy, rect.width, rect.height);
    }
    moves
}

/// A scroll view easing to where a reveal asked: what it moves by in all,
/// and how much of that it has.
struct Reveal {
    viewport: NodeId,
    total: (f32, f32),
    done: (f32, f32),
    from: Instant,
}

/// The reveals easing in a window.
#[derive(Default)]
pub(crate) struct State {
    reveals: Vec<Reveal>,
    next: Option<Instant>,
}

/// Scroll `viewport` by `dx`, `dy` in its own points, as a key or find
/// scrolls it.
fn scroll(ui: &Ui, viewport: NodeId, dx: f32, dy: f32) {
    ui.input.dispatch_to(
        &ui.scene,
        viewport,
        InputEvent::Scroll {
            x: 0.,
            y: 0.,
            delta_x: dx,
            delta_y: dy,
        },
    );
}

impl Ui {
    /// Scroll the scroll views around `node`, innermost first, so it shows as
    /// `how` asks (`NodeHandle::scroll_into_view`): whether any moved. A node
    /// no longer in the window (a list's row scrolled away) moves none.
    #[doc(hidden)]
    pub fn scroll_node_into_view(&self, node: NodeId, how: ScrollIntoView) -> bool {
        {
            let scene = self.scene.borrow();
            if !scene.contains(node) || !scene.layout_visible(node) {
                return false;
            }
        }
        // Laid out as it stands: what was made since is placed.
        self.prepare_frame();
        let rect = self.scene.borrow().transformed_bounds(node);
        let moves = plan(self, node, rect, &how);
        // What eased toward another reveal stops: this one says where.
        {
            let mut state = self.scroll_reveal_state().borrow_mut();
            state
                .reveals
                .retain(|r| !moves.iter().any(|(viewport, ..)| *viewport == r.viewport));
        }
        if moves.is_empty() {
            return false;
        }
        let animate = how.animate.unwrap_or_else(|| {
            !crate::theme::Themes::of(&self.runtime)
                .system_preferences()
                .reduced_motion
        });
        if animate {
            let now = crate::clock::now();
            let mut state = self.scroll_reveal_state().borrow_mut();
            for (viewport, dx, dy) in moves {
                state.reveals.push(Reveal {
                    viewport,
                    total: (dx, dy),
                    done: (0., 0.),
                    from: now,
                });
            }
            state.next = Some(now);
        } else {
            for (viewport, dx, dy) in moves {
                scroll(self, viewport, dx, dy);
            }
        }
        true
    }
}

/// Ease in and out: slow at both ends, as a browser's smooth scroll.
fn ease(t: f32) -> f32 {
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// When a reveal next steps. None easing, nothing.
pub(crate) fn next_step(ui: &Ui) -> Option<Instant> {
    ui.scroll_reveal_state().borrow().next
}

/// Step the reveals due by `now`: whether any moved.
pub(crate) fn advance(ui: &Ui, now: Instant) -> bool {
    let due = {
        let state = ui.scroll_reveal_state().borrow();
        state.next.is_some_and(|next| next <= now)
    };
    if !due {
        return false;
    }
    let reveals = std::mem::take(&mut ui.scroll_reveal_state().borrow_mut().reveals);
    let mut going = Vec::new();
    for mut reveal in reveals {
        if !ui.scene.borrow().contains(reveal.viewport) {
            continue;
        }
        let t = (now.saturating_duration_since(reveal.from).as_secs_f32() / DURATION.as_secs_f32())
            .clamp(0., 1.);
        let f = ease(t);
        let at = (reveal.total.0 * f, reveal.total.1 * f);
        scroll(
            ui,
            reveal.viewport,
            at.0 - reveal.done.0,
            at.1 - reveal.done.1,
        );
        reveal.done = at;
        if t < 1. {
            going.push(reveal);
        }
    }
    let mut state = ui.scroll_reveal_state().borrow_mut();
    // A reveal asked for while these stepped joins them.
    going.append(&mut state.reveals);
    state.reveals = going;
    state.next = (!state.reveals.is_empty()).then(|| now + ui.refresh_interval());
    true
}

/// The person scrolled: what eases stops where it is, as a browser's smooth
/// scroll stops for the wheel.
pub(crate) fn cancel(ui: &Ui) {
    let mut state = ui.scroll_reveal_state().borrow_mut();
    if !state.reveals.is_empty() {
        state.reveals.clear();
        state.next = None;
    }
}
