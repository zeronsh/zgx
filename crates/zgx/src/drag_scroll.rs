//! A drag held near a scroll view's edge scrolls it, as a browser's does:
//! the window's own drag, another window's or app's, or files from outside.
//! The innermost scroll view under the pointer that can still move toward
//! that edge scrolls each frame while the pointer is within `BAND` of it,
//! faster nearer the edge, on either axis; then what is under the pointer
//! hears the drag over it again, so a drop indicator follows the content.
//!
//! Nothing is scheduled outside the band, at the end of what can scroll, or
//! with no drag over the window: an idle window does no work.
use crate::{input::InputEvent, scene::NodeId, time::Instant, widgets::Ui};
use std::time::Duration;

/// How far in from a scroll view's edge a drag scrolls it.
const BAND: f32 = 48.;
/// How far in it scrolls at full speed.
const FULL: f32 = 12.;
/// Full speed: points a frame.
const STEP: f32 = 24.;
/// A frame, which `STEP` is per: the speed is per time, so a faster display
/// scrolls as far a second.
const FRAME: Duration = Duration::from_millis(16);
/// How often it ticks: a 120 Hz display's frame, so it moves each of them.
const TICK: Duration = Duration::from_millis(8);
/// The most motion one tick makes up for after a stall.
const STALL: Duration = Duration::from_millis(50);

/// A drag's autoscroll in a window.
#[derive(Default)]
pub(crate) struct State {
    /// Its next tick, and when the last ran (or it was armed).
    ticks: Option<(Instant, Instant)>,
    /// A scroll view that did not move for the drag at this point (it scrolls
    /// on the other axis only): nothing is scheduled until the drag moves.
    stuck: Option<(NodeId, f32, f32)>,
}

/// Points a frame a pointer at `at` scrolls a view showing `start` to
/// `start + extent` along one axis: toward its end (positive) near its end,
/// toward its start near its start, none in the middle or outside it.
fn speed(at: f32, start: f32, extent: f32) -> f32 {
    if !(at >= start && at <= start + extent) {
        return 0.;
    }
    let ramp = |inside: f32| ((BAND - inside) / (BAND - FULL)).clamp(0., 1.) * STEP;
    let (from_start, from_end) = (at - start, start + extent - at);
    if from_end < BAND && from_end <= from_start {
        ramp(from_end)
    } else if from_start < BAND {
        -ramp(from_start)
    } else {
        0.
    }
}

/// What a drag near a scroll view's edge scrolls.
struct Plan {
    viewport: NodeId,
    /// Points a frame, across and down.
    speed: (f32, f32),
    /// How far it can move along each, toward where it scrolls.
    room: (f32, f32),
}

/// What the drag at `x`, `y` scrolls: the innermost scroll view around what
/// is under it, past `preview`, whose edge it is near on an axis it can still
/// move along.
fn plan(ui: &Ui, x: f32, y: f32, preview: Option<NodeId>) -> Option<Plan> {
    let scene = ui.scene.borrow();
    let hit = scene.hit_test_all(x, y).into_iter().find(|hit| {
        preview
            .is_none_or(|preview| *hit != preview && !scene.ancestors(*hit).any(|id| id == preview))
    })?;
    let contents = std::iter::once(hit)
        .chain(scene.ancestors(hit))
        .filter(|id| scene.is_scroll_content(*id));
    for content in contents {
        let Some(viewport) = scene.parent(content) else {
            continue;
        };
        let Some(visible) = scene.visible_bounds(viewport) else {
            continue;
        };
        // How far its content sits before it, and how far it can.
        let (frame, inner) = (
            scene.transformed_bounds(viewport),
            scene.transformed_bounds(content),
        );
        let room = |speed: f32, offset: f32, limit: f32| {
            let room = if speed < 0. { offset } else { limit - offset };
            if speed != 0. && room > 0.01 {
                (speed, room)
            } else {
                (0., 0.)
            }
        };
        let (dx, across) = room(
            speed(x, visible.x, visible.width),
            frame.x - inner.x,
            inner.width - frame.width,
        );
        let (dy, down) = room(
            speed(y, visible.y, visible.height),
            frame.y - inner.y,
            inner.height - frame.height,
        );
        if dx != 0. || dy != 0. {
            return Some(Plan {
                viewport,
                speed: (dx, dy),
                room: (across, down),
            });
        }
    }
    None
}

/// When the drag over the window next scrolls a view, if it is near an edge
/// of one that can move.
pub(crate) fn next_step(ui: &Ui) -> Option<Instant> {
    let Some(point) = ui.input.drag_point() else {
        let mut state = ui.drag_scroll_state().borrow_mut();
        if state.ticks.is_some() || state.stuck.is_some() {
            *state = State::default();
        }
        return None;
    };
    if let Some((next, _)) = ui.drag_scroll_state().borrow().ticks {
        return Some(next);
    }
    let viewport = plan(ui, point.x, point.y, point.preview)?.viewport;
    let mut state = ui.drag_scroll_state().borrow_mut();
    if state.stuck == Some((viewport, point.x, point.y)) {
        return None;
    }
    state.stuck = None;
    let now = crate::clock::now();
    let next = now + TICK;
    state.ticks = Some((next, now));
    Some(next)
}

/// Scroll the view the drag holds near its edge, if due, by its speed over
/// the time since the last tick (a stall makes up at most `STALL` of it),
/// then tell what is under the drag it is over it. Whether it scrolled.
pub(crate) fn advance(ui: &Ui, now: Instant) -> bool {
    let Some((next, last)) = ui.drag_scroll_state().borrow().ticks else {
        return false;
    };
    if next > now {
        return false;
    }
    let point = ui.input.drag_point();
    let planned = point
        .as_ref()
        .and_then(|point| plan(ui, point.x, point.y, point.preview));
    let (Some(point), Some(plan)) = (point, planned) else {
        ui.drag_scroll_state().borrow_mut().ticks = None;
        return false;
    };
    let frames = now.saturating_duration_since(last).min(STALL).as_secs_f32() / FRAME.as_secs_f32();
    // No further than it can go: the rest would scroll a view around it.
    let step = |speed: f32, room: f32| (speed * frames).clamp(-room, room);
    let viewport = plan.viewport;
    let scrolled = ui
        .input
        .dispatch_to(
            &ui.scene,
            viewport,
            InputEvent::Scroll {
                x: point.x,
                y: point.y,
                delta_x: step(plan.speed.0, plan.room.0),
                delta_y: step(plan.speed.1, plan.room.1),
            },
        )
        .default_prevented;
    if !scrolled {
        *ui.drag_scroll_state().borrow_mut() = State {
            ticks: None,
            stuck: Some((viewport, point.x, point.y)),
        };
        return false;
    }
    ui.drag_scroll_state().borrow_mut().ticks = Some((now + TICK, now));
    // Laid out where it scrolled to, then over what is under it now.
    let _ = ui.try_prepare_frame();
    ui.input.drag_over_again(&ui.scene, point);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_scrolls_faster_nearer_an_edge_and_not_in_the_middle() {
        // A view 0 to 400 along the axis.
        assert_eq!(speed(200., 0., 400.), 0.);
        assert_eq!(speed(400. - BAND, 0., 400.), 0.);
        assert_eq!(speed(400. - 10., 0., 400.), STEP);
        assert_eq!(speed(400., 0., 400.), STEP);
        assert!((speed(400. - 30., 0., 400.) - STEP / 2.).abs() < 0.01);
        assert_eq!(speed(10., 0., 400.), -STEP);
        assert_eq!(speed(-1., 0., 400.), 0., "outside it");
        assert_eq!(speed(401., 0., 400.), 0., "outside it");
        assert_eq!(speed(f32::NAN, 0., 400.), 0.);
        // Shorter than two bands: toward the nearer edge.
        assert!(speed(30., 0., 50.) > 0.);
        assert!(speed(20., 0., 50.) < 0.);
    }
}
