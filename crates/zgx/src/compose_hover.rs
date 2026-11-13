//! Hover-opened popups (`View::hover_delay`) and the safe polygon: a popup
//! that opens after the pointer rests on its trigger and closes after it has
//! left the trigger and the popup, as Base UI's `openOnHover`, `delay` and
//! `closeDelay` do. Leaving the trigger toward the popup is held open while
//! the pointer stays in the triangle from where it left to the popup's near
//! edge (and the gap between them), and closes once it strays out of it or
//! rests in it too long, as Floating UI's `safePolygon` does.
use crate::{
    compose::{TaskRunner, TaskToken},
    input::{InputEvent, PointerButton},
    reactive::Signal,
    scene::{NodeId, Rect},
    widgets::Ui,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

/// How long the pointer rests on a trigger before its popup opens, and how
/// long after it has left both the popup closes (`View::hover_delay`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoverDelay {
    /// Resting on the trigger before the popup opens.
    pub open: Duration,
    /// Away from the trigger and the popup before it closes.
    pub close: Duration,
}

/// A submenu's: it waits 100 ms under the pointer before it opens, as Base
/// UI's and AppKit's do, and closes at once.
pub(crate) const SUBMENU_HOVER: HoverDelay = HoverDelay {
    open: Duration::from_millis(100),
    close: Duration::ZERO,
};

/// How long the pointer may rest inside the safe polygon, short of the
/// popup, before the popup closes (it has stopped heading there).
pub(crate) const SAFE_TIMEOUT: Duration = Duration::from_millis(300);

/// Whether `point` is in `rect`, its top and left edges included, as hit
/// testing has it: a pointer on the edge two boxes share is on the lower one.
pub(crate) fn contains(rect: Rect, point: (f32, f32)) -> bool {
    point.0 >= rect.x
        && point.0 < rect.x + rect.width
        && point.1 >= rect.y
        && point.1 < rect.y + rect.height
}

/// Whether `p` is in the triangle `a`, `b`, `c` (edges included).
fn in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| {
        (p.0 - r.0) * (q.1 - r.1) - (q.0 - r.0) * (p.1 - r.1)
    };
    let (d1, d2, d3) = (side(p, a, b), side(p, b, c), side(p, c, a));
    let negative = d1 < 0. || d2 < 0. || d3 < 0.;
    let positive = d1 > 0. || d2 > 0. || d3 > 0.;
    !(negative && positive)
}

/// Whether the pointer at `p` is still on its way from `exit` (where it
/// left `trigger`) to `floating`: on the popup, in the triangle from `exit`
/// to the popup's near edge, or in the gap between the two boxes.
pub(crate) fn in_safe_polygon(
    exit: (f32, f32),
    trigger: Rect,
    floating: Rect,
    p: (f32, f32),
) -> bool {
    if contains(floating, p) {
        return true;
    }
    let (t_right, t_bottom) = (trigger.x + trigger.width, trigger.y + trigger.height);
    let (f_right, f_bottom) = (floating.x + floating.width, floating.y + floating.height);
    // The popup's near edge, and the gap between the boxes.
    let (near, gap) = if floating.x >= t_right - 0.5 {
        (
            ((floating.x, floating.y), (floating.x, f_bottom)),
            Rect::new(
                t_right.min(floating.x),
                trigger.y.min(floating.y),
                (floating.x - t_right).abs(),
                t_bottom.max(f_bottom) - trigger.y.min(floating.y),
            ),
        )
    } else if f_right <= trigger.x + 0.5 {
        (
            ((f_right, floating.y), (f_right, f_bottom)),
            Rect::new(
                f_right.min(trigger.x),
                trigger.y.min(floating.y),
                (trigger.x - f_right).abs(),
                t_bottom.max(f_bottom) - trigger.y.min(floating.y),
            ),
        )
    } else if floating.y >= t_bottom - 0.5 {
        (
            ((floating.x, floating.y), (f_right, floating.y)),
            Rect::new(
                trigger.x.min(floating.x),
                t_bottom.min(floating.y),
                t_right.max(f_right) - trigger.x.min(floating.x),
                (floating.y - t_bottom).abs(),
            ),
        )
    } else {
        (
            ((floating.x, f_bottom), (f_right, f_bottom)),
            Rect::new(
                trigger.x.min(floating.x),
                f_bottom.min(trigger.y),
                t_right.max(f_right) - trigger.x.min(floating.x),
                (trigger.y - f_bottom).abs(),
            ),
        )
    };
    contains(gap, p) || in_triangle(p, exit, near.0, near.1)
}

/// A timer a popup's hover logic holds: what it will do, and the task.
#[derive(Default)]
pub(crate) struct Pending {
    task: RefCell<Option<TaskToken>>,
    kind: Cell<Kind>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    None,
    Open,
    /// A close that began as the pointer left (not one waiting on the
    /// safe polygon, which each move starts over).
    Close,
    Safe,
}
impl Pending {
    fn cancel(&self) {
        self.task.borrow_mut().take();
        self.kind.set(Kind::None);
    }
    fn kind(&self) -> Kind {
        self.kind.get()
    }
    /// Run `f` after `delay` (at once without a runner, or with no delay),
    /// replacing what waited.
    fn after(
        &self,
        runner: &Option<Rc<TaskRunner>>,
        delay: Duration,
        kind: Kind,
        f: impl FnOnce() + 'static,
    ) {
        self.cancel();
        match runner {
            Some(runner) if !delay.is_zero() => {
                self.kind.set(kind);
                *self.task.borrow_mut() = Some(runner.spawn(Box::pin(async move {
                    crate::timer::sleep(delay).await;
                    f();
                })));
            }
            _ => f(),
        }
    }
}

/// Open `open` as the pointer rests on `anchor` for `delay.open`, and close
/// it after the pointer has been off `anchor` and `panel` for `delay.close`
/// (held while it heads for the panel). A popup opened any other way (a
/// click) stays as it is opened: only one this opened closes on leaving.
pub(crate) fn mount(
    ui: &mut Ui,
    owner: NodeId,
    anchor: NodeId,
    panel: NodeId,
    open: Signal<bool>,
    delay: HoverDelay,
    runner: Option<Rc<TaskRunner>>,
) {
    let weak = ui.downgrade();
    let pending = Rc::new(Pending::default());
    let by_hover = Rc::new(Cell::new(false));
    let exit: Cell<Option<(f32, f32)>> = Cell::new(None);
    ui.on_document_event(owner, move |event| {
        let (x, y) = match *event {
            InputEvent::PointerMove { x, y } => (x, y),
            // A press on the trigger keeps what hovering opened open.
            InputEvent::PointerDown {
                x,
                y,
                button: PointerButton::Primary,
            } => {
                let Some(ui) = weak.upgrade() else { return };
                let scene = ui.scene.borrow();
                if scene.contains(anchor) && contains(scene.world_bounds(anchor), (x, y)) {
                    by_hover.set(false);
                    pending.cancel();
                }
                return;
            }
            _ => return,
        };
        let Some(ui) = weak.upgrade() else { return };
        let (trigger, floating) = {
            let scene = ui.scene.borrow();
            if !scene.contains(anchor) {
                return;
            }
            (
                scene.world_bounds(anchor),
                scene.contains(panel).then(|| scene.world_bounds(panel)),
            )
        };
        let p = (x, y);
        let shown = open.with_untracked(|open| *open);
        if !shown {
            by_hover.set(false);
        }
        if contains(trigger, p) {
            exit.set(Some(p));
            if shown {
                pending.cancel();
            } else if pending.kind() != Kind::Open {
                let (open, by_hover) = (open.clone(), by_hover.clone());
                pending.after(&runner, delay.open, Kind::Open, move || {
                    by_hover.set(true);
                    open.set(true);
                });
            }
            return;
        }
        if shown && floating.is_some_and(|floating| contains(floating, p)) {
            pending.cancel();
            return;
        }
        if !shown {
            if pending.kind() == Kind::Open {
                pending.cancel();
            }
            return;
        }
        if !by_hover.get() {
            return;
        }
        let held = exit
            .get()
            .zip(floating)
            .is_some_and(|(exit, floating)| in_safe_polygon(exit, trigger, floating, p));
        let close = {
            let (open, by_hover) = (open.clone(), by_hover.clone());
            move || {
                by_hover.set(false);
                open.set(false);
            }
        };
        if held {
            // Each move inside it starts the wait over: resting is not
            // heading there.
            pending.after(&runner, SAFE_TIMEOUT, Kind::Safe, close);
        } else if pending.kind() != Kind::Close {
            pending.after(&runner, delay.close, Kind::Close, close);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trigger at 0..100 across and 0..20 down, a popup right of it at 110.
    fn boxes() -> (Rect, Rect) {
        (Rect::new(0., 0., 100., 20.), Rect::new(110., 0., 80., 120.))
    }

    #[test]
    fn a_pointer_heading_diagonally_to_the_popup_is_held() {
        let (trigger, popup) = boxes();
        let exit = (90., 10.);
        // Down and across, within the triangle to the popup's near edge.
        assert!(in_safe_polygon(exit, trigger, popup, (105., 30.)));
        assert!(in_safe_polygon(exit, trigger, popup, (108., 90.)));
        // In the gap, and on the popup.
        assert!(in_safe_polygon(exit, trigger, popup, (105., 5.)));
        assert!(in_safe_polygon(exit, trigger, popup, (120., 60.)));
    }

    #[test]
    fn a_pointer_leaving_the_triangle_is_not_held() {
        let (trigger, popup) = boxes();
        let exit = (90., 10.);
        // Down and away, back left over the trigger's column, or far below.
        assert!(!in_safe_polygon(exit, trigger, popup, (60., 60.)));
        assert!(!in_safe_polygon(exit, trigger, popup, (95., 130.)));
        assert!(!in_safe_polygon(exit, trigger, popup, (105., 200.)));
    }

    #[test]
    fn the_polygon_follows_the_side_the_popup_is_on() {
        let trigger = Rect::new(100., 100., 100., 20.);
        // Below.
        let below = Rect::new(100., 130., 120., 80.);
        assert!(in_safe_polygon((150., 118.), trigger, below, (160., 125.)));
        assert!(!in_safe_polygon((150., 118.), trigger, below, (20., 125.)));
        // Above.
        let above = Rect::new(100., 10., 120., 80.);
        assert!(in_safe_polygon((150., 102.), trigger, above, (160., 95.)));
        // Left.
        let left = Rect::new(10., 100., 80., 120.);
        assert!(in_safe_polygon((102., 110.), trigger, left, (95., 150.)));
        assert!(!in_safe_polygon((102., 110.), trigger, left, (250., 150.)));
    }
}
