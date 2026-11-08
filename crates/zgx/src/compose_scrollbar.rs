//! Shared retained overlay controls for ordinary and virtual viewports.
use crate::{
    input::{EventPhase, InputEvent, Key, PointerButton},
    reactive::Signal,
    scene::{Color, Layout, NodeId, NodeKind, QuadStyle, Style, Transform},
    semantics::{Role, ScrollAxis, SemanticNode},
    theme::tok,
    widgets::Ui,
};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

/// How a scrollbar is drawn and timed: zgx's own, or Windows 11's.
struct Look {
    /// The thumb's thickness at rest and widened (under the pointer, or
    /// dragged), how far its rounded ends sit in from the track's, and the
    /// most its corners round.
    thumb: f32,
    thumb_wide: f32,
    inset: f32,
    radius: f32,
    /// The track's thickness (where it takes the pointer), how far it sits
    /// from the view's edge, and the shortest thumb.
    track: f32,
    margin: f32,
    min_thumb: f64,
    /// How long an overlay scrollbar stays once its view stops scrolling
    /// (or the pointer leaves it), and how long it takes to fade out, a
    /// step a refresh (`Ui::set_refresh_interval`).
    linger: Duration,
    fade: Duration,
    /// Widening: how long the pointer rests on it first, how long after it
    /// leaves it narrows, and how long either takes. `None`: at once.
    widen: Option<(Duration, Duration, Duration)>,
    /// Brighter under the pointer (zgx's), or the same colour (Windows').
    brightens: bool,
    /// Arrow buttons at its ends, each this long, shown as it widens
    /// (Windows'); `None`, none.
    arrows: Option<f32>,
    /// An arrow button held: how long until it repeats its step, and how
    /// often after.
    repeat: (Duration, Duration),
}
/// zgx's: 6 across, 8 under the pointer, in an 8 track at the edge.
const BUILT_IN: Look = Look {
    thumb: 6.,
    thumb_wide: 8.,
    inset: 2.,
    radius: f32::INFINITY,
    track: 8.,
    margin: 0.,
    min_thumb: 24.,
    linger: Duration::from_millis(1000),
    fade: Duration::from_millis(200),
    widen: None,
    brightens: true,
    arrows: None,
    repeat: (Duration::ZERO, Duration::ZERO),
};
/// Windows 11's, as WinUI 3 draws its ScrollBar (microsoft-ui-xaml's
/// `ScrollBar_themeresources.xaml`, `ScrollViewer_themeresources.xaml`): a
/// thin indicator (an 8 thumb inside a 6 transparent stroke) that widens
/// to 6 in a 12 track (`ScrollBarSize`) 0.4 s after the pointer arrives
/// (`ScrollBarExpandBeginTime`) and narrows 0.5 s after it leaves
/// (`ScrollBarContractBeginTime`), each over 167 ms
/// (`ScrollBarExpandDuration`), rounded 3 (`ScrollBarCornerRadius`), 1 from
/// the edge (`ScrollViewerScrollBarMargin`), at least 30 long; hidden 2 s
/// after scrolling stops (`ScrollViewerSeparatorContractDelay`), fading
/// over 83 ms (`ScrollBarOpacityChangeDuration`). Its thumb keeps one
/// colour under the pointer. Widened, arrow buttons 12 long show at its
/// ends (`ScrollBarSize`), a step a press, their carets (CaretSolid8,
/// drawn here as triangles) in the thumb's colour.
const WINDOWS: Look = Look {
    thumb: 2.,
    thumb_wide: 6.,
    inset: 2.,
    radius: 3.,
    track: 12.,
    margin: 1.,
    min_thumb: 30.,
    linger: Duration::from_millis(2000),
    fade: Duration::from_millis(83),
    widen: Some((
        Duration::from_millis(400),
        Duration::from_millis(500),
        Duration::from_millis(167),
    )),
    brightens: false,
    arrows: Some(12.),
    // RepeatButton's `Delay` (500, its default) and the scrollbar's
    // `Interval` (50).
    repeat: (Duration::from_millis(500), Duration::from_millis(50)),
};
const LOOK: Look = if cfg!(windows) { WINDOWS } else { BUILT_IN };

/// WinUI's widening curve, `cubic-bezier(0, 0, 0, 1)`: x = t³ and
/// y = 3t² − 2t³, so at progress `p`, t = ∛p.
fn widen_curve(p: f32) -> f32 {
    let t = p.clamp(0., 1.).cbrt();
    t * t * (3. - 2. * t)
}

/// An arrow button's step, run again while it is held.
type Step = Rc<dyn Fn()>;

/// An overlay scrollbar's showing (`Ui::set_overlay_scrollbars`).
pub(crate) struct Fade {
    /// 1 shown, 0 hidden.
    alpha: Signal<f32>,
    /// When it starts fading; `None` while it stays (held) or is hidden.
    fade_at: Cell<Option<Instant>>,
    /// Its next step: `fade_at`, then each frame of the fade.
    next: Cell<Option<Instant>>,
    /// In the document's list of fades.
    listed: Cell<bool>,
    /// How far the thumb has widened, 0 to 1 (`Look::widen`); where it is
    /// going and when that starts; and, while it moves, from where, since
    /// when, and its next step.
    widened: Signal<f32>,
    aim: Cell<Option<(f32, Instant)>>,
    moving: Cell<Option<(f32, f32, Instant)>>,
    step: Cell<Option<Instant>>,
    /// An arrow button held (`Look::repeat`): when it next steps, and the
    /// step.
    repeat: std::cell::RefCell<Option<(Instant, Step)>>,
}
impl Fade {
    /// Widen (`to` 1) or narrow (0) after `delay`, unless it is there or
    /// going there already.
    fn aim(&self, to: f32, delay: Duration, now: Instant) {
        let going = match (self.moving.get(), self.aim.get()) {
            (_, Some((aimed, _))) => aimed,
            (Some((_, target, _)), None) => target,
            (None, None) => self.widened.with_untracked(|w| *w),
        };
        if going == to {
            return;
        }
        // Turned back before it started: it stays as it is.
        if self.aim.get().is_some() && self.moving.get().is_none() {
            self.aim.set(None);
            if self.widened.with_untracked(|w| *w) == to {
                return;
            }
        }
        self.aim.set(Some((to, now + delay)));
    }
    /// When its widening next steps, if it does.
    fn widen_due(&self) -> Option<Instant> {
        match (self.aim.get(), self.step.get()) {
            (Some((_, at)), Some(step)) => Some(at.min(step)),
            (aim, step) => aim.map(|(_, at)| at).or(step),
        }
    }
    fn show(&self, now: Instant) {
        self.alpha.set(1.);
        self.linger(now);
    }
    /// Under the pointer or dragged: it stays.
    fn hold(&self) {
        self.alpha.set(1.);
        self.fade_at.set(None);
        self.next.set(None);
    }
    fn linger(&self, now: Instant) {
        let at = now + LOOK.linger;
        self.fade_at.set(Some(at));
        self.next.set(Some(at));
    }
    fn hide(&self) {
        self.alpha.set(0.);
        self.fade_at.set(None);
        self.next.set(None);
    }
}

/// When the next overlay scrollbar steps toward hidden. None shown, none.
pub(crate) fn next_fade(ui: &Ui) -> Option<Instant> {
    let fades = ui.scrollbar_fades().borrow();
    if fades.is_empty() {
        return None;
    }
    fades
        .iter()
        .filter_map(|fade| {
            let fade = fade.upgrade()?;
            let repeat = fade.repeat.borrow().as_ref().map(|(at, _)| *at);
            [fade.next.get(), fade.widen_due(), repeat]
                .into_iter()
                .flatten()
                .min()
        })
        .min()
}

/// Step the overlay scrollbars due by `now` toward hidden; whether any did.
pub(crate) fn advance_fades(ui: &Ui, now: Instant) -> bool {
    let fades: Vec<Rc<Fade>> = {
        let mut list = ui.scrollbar_fades().borrow_mut();
        if list.is_empty() {
            return false;
        }
        list.retain(|fade| fade.strong_count() > 0);
        list.iter().filter_map(Weak::upgrade).collect()
    };
    let mut stepped = false;
    for fade in fades {
        // An arrow button held: its step again, and again after `repeat.1`.
        let due = fade
            .repeat
            .borrow()
            .as_ref()
            .filter(|(at, _)| *at <= now)
            .map(|(_, step)| step.clone());
        if let Some(step) = due {
            step();
            if let Some((at, _)) = fade.repeat.borrow_mut().as_mut() {
                *at = now + LOOK.repeat.1;
            }
            stepped = true;
        }
        if let Some((_, _, duration)) = LOOK.widen {
            stepped |= step_widening(&fade, now, duration, ui.refresh_interval());
        }
        let (Some(next), Some(at)) = (fade.next.get(), fade.fade_at.get()) else {
            continue;
        };
        if next > now {
            continue;
        }
        let done = now.saturating_duration_since(at).as_secs_f32() / LOOK.fade.as_secs_f32();
        if done >= 1. {
            fade.hide();
        } else {
            fade.alpha.set(1. - done);
            fade.next.set(Some(now + ui.refresh_interval()));
        }
        stepped = true;
    }
    stepped
}

/// A scrollbar's thumb along a track `length` long (its arrows taking
/// `reserve` at each end) showing `length` of `total`: the thumb's length,
/// how far it travels, and where its travel starts.
fn thumb_geometry(length: f64, total: f64, reserve: f64) -> (f64, f64, f64) {
    let room = (length - 2. * reserve).max(0.);
    let thumb = (room * length / total.max(1.))
        .max(LOOK.min_thumb)
        .min(room);
    ((thumb).max(0.), (room - thumb).max(0.), reserve)
}

/// An arrow button's wash under the pointer (`SubtleFillColorSecondary`).
fn wash_kind(color: Color) -> NodeKind {
    NodeKind::Quad(QuadStyle {
        fill: color,
        radius: LOOK.radius,
        ..Default::default()
    })
}

/// An arrow button's caret, a triangle `width` across pointing `toward`
/// (0 up, 1 right, 2 down, 3 left), centred in a `cell` square.
fn caret(color: Color, cell: f32, toward: u8) -> NodeKind {
    let (w, h) = (6., 3.5);
    let (cx, cy) = (cell / 2., cell / 2.);
    let points = match toward {
        0 => [
            (cx - w / 2., cy + h / 2.),
            (cx + w / 2., cy + h / 2.),
            (cx, cy - h / 2.),
        ],
        2 => [
            (cx - w / 2., cy - h / 2.),
            (cx + w / 2., cy - h / 2.),
            (cx, cy + h / 2.),
        ],
        3 => [
            (cx + h / 2., cy - w / 2.),
            (cx + h / 2., cy + w / 2.),
            (cx - h / 2., cy),
        ],
        _ => [
            (cx - h / 2., cy - w / 2.),
            (cx - h / 2., cy + w / 2.),
            (cx + h / 2., cy),
        ],
    };
    let mut path = crate::canvas::Path::builder();
    path.move_to(points[0].0, points[0].1)
        .line_to(points[1].0, points[1].1)
        .line_to(points[2].0, points[2].1)
        .close();
    let mut canvas = crate::canvas::Canvas::new();
    if let Ok(path) = path.build() {
        canvas.fill(path, color);
    }
    NodeKind::Canvas(std::sync::Arc::new(canvas))
}

/// Start or step `fade`'s widening due by `now` (`duration` long, a step
/// each `interval`); whether it did.
fn step_widening(fade: &Fade, now: Instant, duration: Duration, interval: Duration) -> bool {
    if let Some((to, at)) = fade.aim.get()
        && at <= now
    {
        fade.aim.set(None);
        fade.moving
            .set(Some((fade.widened.with_untracked(|w| *w), to, now)));
    }
    let Some((from, to, since)) = fade.moving.get() else {
        fade.step.set(None);
        return false;
    };
    if fade.step.get().is_some_and(|step| step > now) {
        return false;
    }
    let p = now.saturating_duration_since(since).as_secs_f32() / duration.as_secs_f32();
    if p >= 1. {
        fade.widened.set(to);
        fade.moving.set(None);
        fade.step.set(None);
    } else {
        fade.widened.set(from + (to - from) * widen_curve(p));
        fade.step.set(Some(now + interval));
    }
    true
}

/// A rounded thumb of `color`, `thickness` across.
fn thumb_kind(color: Color, thickness: f32) -> NodeKind {
    NodeKind::Quad(QuadStyle {
        fill: color,
        radius: (thickness / 2.).min(LOOK.radius),
        ..Default::default()
    })
}

pub(crate) fn mount(
    ui: &mut Ui,
    owner: NodeId,
    offset: Signal<f32>,
    size: Signal<(f32, f32)>,
    extent: Signal<f32>,
    horizontal: bool,
) {
    let overlay = ui.container(owner, Layout::Overlay, Style::default());
    // The view's own fade (`fade`) fades its rows, not its scrollbar.
    ui.scene.borrow_mut().set_outside_fade(overlay);
    // Its parts are placed by transforms from its corner, in points across
    // whichever way the view runs.
    ui.scene
        .borrow_mut()
        .set_direction(overlay, Some(crate::scene::Direction::Ltr));
    let track = ui.container(overlay, Layout::Overlay, Style::default());
    let theme = ui.theme_scope();
    let background = ui.scene.borrow_mut().append(
        track,
        NodeKind::Rect(theme.color_untracked(tok::SCROLLBAR_TRACK)),
        Style::default(),
    );
    // Its arrow buttons' carets (Windows'): decrease, increase.
    let arrows = LOOK.arrows.map(|cell| {
        let mut scene = ui.scene.borrow_mut();
        let color = Color(0, 0, 0, 0);
        let mut button = |toward| {
            // Its wash under the pointer, then its caret.
            let wash = scene.append(track, wash_kind(color), Style::default());
            (
                wash,
                scene.append(track, caret(color, cell, toward), Style::default()),
            )
        };
        [button(0), button(2)]
    });
    // Which arrow button the pointer is on: 0 none, 1 decrease, 2 increase.
    let arrow_hover = ui.signal(0u8);
    let event_hover = arrow_hover.clone();
    let thumb = ui.scene.borrow_mut().append(
        track,
        thumb_kind(theme.color_untracked(tok::SCROLLBAR_THUMB), LOOK.thumb),
        Style::default(),
    );
    let main = move |size: (f32, f32)| if horizontal { size.0 } else { size.1 };
    // How long its arrow buttons are at each end as drawn (0 without),
    // for its events to place the thumb as the drawing does.
    let reserve = Rc::new(Cell::new(0f32));
    let event_reserve = reserve.clone();

    let event_size = size.clone();
    let event_extent = extent.clone();
    let event_offset = offset.clone();
    // As an overlay scrollbar: shown as the user scrolls, then fading.
    let overlay_mode = ui.overlay_scrollbars();
    let fade = Rc::new(Fade {
        alpha: ui.signal(1.),
        fade_at: Cell::new(None),
        next: Cell::new(None),
        listed: Cell::new(false),
        widened: ui.signal(0.),
        aim: Cell::new(None),
        moving: Cell::new(None),
        step: Cell::new(None),
        repeat: Default::default(),
    });
    let scene = ui.scene.clone();
    let focused = ui.signal(false);
    let write_focus = focused.clone();
    // Under the pointer, or dragged: the thumb widens and brightens.
    let active = ui.signal(false);
    let write_active = active.clone();
    let mut grab = None;
    let (event_fade, event_ui) = (fade.clone(), ui.downgrade());
    ui.on_event(track, true, move |cx| {
        if cx.phase == EventPhase::Capture {
            return;
        }
        if matches!(cx.event, InputEvent::Focus) {
            write_focus.set(true);
        }
        if matches!(cx.event, InputEvent::Blur) {
            write_focus.set(false);
        }
        match cx.event {
            InputEvent::PointerEnter | InputEvent::PointerMove { .. } => {
                write_active.set(true);
            }
            // A drag ended: the next move over the bar lights it again.
            InputEvent::PointerLeave if grab.is_none() => {
                write_active.set(false);
            }
            InputEvent::PointerUp { .. } | InputEvent::PointerCancel | InputEvent::Blur => {
                write_active.set(false);
            }
            _ => {}
        }
        if cx.default_prevented()
            && !matches!(
                cx.event,
                InputEvent::PointerUp { .. }
                    | InputEvent::PointerCancel
                    | InputEvent::Blur
                    | InputEvent::Focus
            )
        {
            return;
        }
        let length = main(event_size.get()) as f64;
        let total = event_extent.get() as f64;
        let limit = (total - length).max(0.);
        let reserve = event_reserve.get() as f64;
        let (thumb_length, travel, from) = thumb_geometry(length, total, reserve);
        // Right to left, a horizontal bar counts from its right end.
        let rtl = horizontal && scene.borrow().direction(owner) == crate::scene::Direction::Rtl;
        let position = match cx.event {
            InputEvent::PointerDown { x, y, .. } | InputEvent::PointerMove { x, y } => {
                let bounds = scene.borrow().bounds(track);
                Some(if rtl {
                    length - (x as f64 - bounds.x as f64)
                } else if horizontal {
                    x as f64 - bounds.x as f64
                } else {
                    y as f64 - bounds.y as f64
                })
            }
            _ => None,
        };
        if position.is_some_and(|position| !position.is_finite()) {
            return;
        }
        // On which arrow button the pointer is, for its wash.
        if reserve > 0. {
            let on = match (&cx.event, position) {
                (InputEvent::PointerMove { .. }, Some(at)) if at < reserve => Some(1),
                (InputEvent::PointerMove { .. }, Some(at)) if at > length - reserve => Some(2),
                (InputEvent::PointerMove { .. }, Some(_)) | (InputEvent::PointerLeave, _) => {
                    Some(0)
                }
                _ => None,
            };
            if let Some(on) = on {
                event_hover.set(on);
            }
        }
        let before = event_offset.get() as f64;
        let next = match &cx.event {
            InputEvent::PointerDown {
                button: PointerButton::Primary,
                ..
            } if limit > 0. => {
                cx.focus();
                let pointer = position.unwrap();
                let thumb_start = from + before / limit * travel;
                // On an arrow button: a step (as an arrow key's), again and
                // again while it is held (`Look::repeat`), each a wake.
                if reserve > 0. && (pointer < reserve || pointer > length - reserve) {
                    let step = if pointer < reserve { -40. } else { 40. };
                    if !LOOK.repeat.0.is_zero() {
                        let (offset, size, extent) = (
                            event_offset.clone(),
                            event_size.clone(),
                            event_extent.clone(),
                        );
                        let again: Step = Rc::new(move || {
                            let length = main(size.get()) as f64;
                            let limit = (extent.get() as f64 - length).max(0.);
                            offset.set((offset.get() as f64 + step).clamp(0., limit) as f32);
                        });
                        *event_fade.repeat.borrow_mut() =
                            Some((crate::clock::now() + LOOK.repeat.0, again));
                        if !event_fade.listed.replace(true)
                            && let Some(ui) = event_ui.upgrade()
                        {
                            ui.scrollbar_fades()
                                .borrow_mut()
                                .push(Rc::downgrade(&event_fade));
                        }
                        cx.capture_pointer();
                    }
                    Some(before + step)
                } else if pointer >= thumb_start && pointer <= thumb_start + thumb_length {
                    grab = Some(pointer - thumb_start);
                    cx.capture_pointer();
                    None
                } else {
                    Some(
                        before
                            + if pointer < thumb_start {
                                -length
                            } else {
                                length
                            },
                    )
                }
            }
            InputEvent::PointerMove { .. } if grab.is_some() => Some(if travel > 0. {
                (position.unwrap() - grab.unwrap() - from) / travel * limit
            } else {
                0.
            }),
            InputEvent::PointerUp {
                button: PointerButton::Primary,
                ..
            }
            | InputEvent::PointerCancel
            | InputEvent::Blur => {
                grab = None;
                event_fade.repeat.borrow_mut().take();
                cx.release_pointer();
                None
            }
            // The wheel over the bar scrolls what it scrolls (a
            // Control-wheel zooms the window instead).
            InputEvent::Scroll {
                delta_x, delta_y, ..
            } if !crate::zoom::wheel_zooms() => {
                let delta = if rtl {
                    -*delta_x
                } else if horizontal {
                    *delta_x
                } else {
                    *delta_y
                };
                (delta.is_finite() && delta != 0.).then_some(before + delta as f64)
            }
            InputEvent::KeyDown { key, .. } => match key {
                Key::Home => Some(0.),
                Key::End => Some(limit),
                Key::PageUp => Some(before - length),
                Key::PageDown => Some(before + length),
                Key::ArrowLeft if rtl => Some(before + 40.),
                Key::ArrowRight if rtl => Some(before - 40.),
                Key::ArrowLeft | Key::ArrowUp => Some(before - 40.),
                Key::ArrowRight | Key::ArrowDown => Some(before + 40.),
                _ => None,
            },
            InputEvent::Increment => Some(before + 40.),
            InputEvent::Decrement => Some(before - 40.),
            InputEvent::SetNumericValue(value) if value.is_finite() => Some(*value),
            InputEvent::SetValue(value) => value.parse::<f64>().ok().filter(|v| v.is_finite()),
            _ => None,
        };
        if let Some(next) = next {
            event_offset.set(next.clamp(0., limit) as f32);
            cx.prevent_default();
            cx.stop_propagation();
        } else if matches!(
            cx.event,
            InputEvent::PointerDown {
                button: PointerButton::Primary,
                ..
            }
        ) {
            cx.prevent_default();
            cx.stop_propagation();
        }
    });
    let (weak, shown, mode) = (ui.downgrade(), fade.clone(), overlay_mode.clone());
    let (scrolled, held, focus) = (offset.clone(), active.clone(), focused.clone());
    let mut last = None;
    ui.bind(overlay, move || {
        // Off, this reads only the mode.
        if !mode.get() {
            if last.take().is_some() {
                shown.hold();
            }
            return;
        }
        let value = scrolled.get();
        let (pointer, focused) = (held.get(), focus.get());
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if !shown.listed.replace(true) {
            ui.scrollbar_fades()
                .borrow_mut()
                .push(Rc::downgrade(&shown));
        }
        let now = crate::clock::now();
        // Windows': it widens a moment after the pointer arrives, narrows a
        // moment after it leaves (or a drag ends), each a wake.
        if let Some((widen, narrow, _)) = LOOK.widen {
            match pointer {
                true => shown.aim(1., widen, now),
                false => shown.aim(0., narrow, now),
            }
        }
        match last.replace(value) {
            // Turned on: hidden until the user scrolls.
            None => shown.hide(),
            // Focused, it shows; under the pointer or dragged, it stays
            // while it shows.
            _ if focused => shown.hold(),
            _ if pointer => {
                if shown.alpha.with_untracked(|alpha| *alpha > 0.) {
                    shown.hold();
                }
            }
            // The user scrolled it, not the app.
            Some(before) if before != value && ui.is_dispatching() => shown.show(now),
            // Let go: it stays a moment, then fades.
            _ if shown.fade_at.get().is_none() && shown.alpha.with_untracked(|a| *a > 0.) => {
                shown.linger(now)
            }
            _ => {}
        }
    });
    ui.retain(overlay, fade.clone());
    let weak = ui.downgrade();
    ui.bind(overlay, move || {
        let Some(mut ui) = weak.upgrade() else {
            return;
        };
        let overlay_mode = overlay_mode.get();
        let alpha = if overlay_mode { fade.alpha.get() } else { 1. };
        let (width, height) = size.get();
        let length = main((width, height));
        let total = extent.get();
        let limit = (total - length).max(0.);
        let visible = limit > 0. && length > 0. && width > 0. && height > 0.;
        if ui
            .input
            .options(track)
            .is_none_or(|options| options.disabled == visible)
        {
            ui.set_disabled(track, !visible);
        }
        let fixed = |width, height| Style {
            width: Some(width),
            height: Some(height),
            ..Default::default()
        };
        let mut scene = ui.scene.borrow_mut();
        // Right to left, a vertical bar is at the left (the start edge, as
        // browsers put it) and a horizontal thumb starts at the right.
        let rtl = scene.direction(owner) == crate::scene::Direction::Rtl;
        // Zero area lets underlying controls receive hits outside the track.
        scene.set_style(
            overlay,
            Style {
                margin: crate::scene::Insets {
                    right: width,
                    ..Default::default()
                },
                ..fixed(0., 0.)
            },
        );
        let thickness = if horizontal { height } else { width }.min(LOOK.track);
        // Hidden, the pointer reaches what is under it.
        let track_size = if !visible || alpha <= 0. {
            (0., 0.)
        } else if horizontal {
            (length, thickness)
        } else {
            (thickness, length)
        };
        scene.set_style(track, fixed(track_size.0, track_size.1));
        scene.set_style(background, fixed(track_size.0, track_size.1));
        scene.set_transform(
            track,
            if horizontal {
                Transform {
                    x: 0.,
                    y: height - thickness,
                }
            } else {
                Transform {
                    x: if rtl { 0. } else { width - thickness },
                    y: 0.,
                }
            },
        );
        // Windows' arrow buttons take their room as it widens (all of it
        // where scrollbars always show).
        let arrow_room = match (LOOK.arrows, LOOK.widen) {
            (Some(cell), Some(_)) if overlay_mode => cell * fade.widened.get(),
            (Some(cell), _) => cell,
            (None, _) => 0.,
        };
        let arrow_room = arrow_room.min(length / 4.);
        reserve.set(arrow_room);
        let (thumb_length, travel, from) =
            thumb_geometry(length as f64, total as f64, arrow_room as f64);
        let (thumb_length, travel, from) = if visible && alpha > 0. {
            (thumb_length as f32, travel as f32, from as f32)
        } else {
            (0., 0., 0.)
        };
        // Drawn slimmer than the track it hits in, centred across it, its
        // rounded ends inset along it.
        let active = active.get();
        // How wide: under the pointer (zgx's), or as far as it has widened
        // (Windows', always wide where scrollbars always show).
        let widened = match LOOK.widen {
            None => f32::from(active),
            Some(_) if !overlay_mode => 1.,
            Some(_) => fade.widened.get(),
        };
        let across = (LOOK.thumb + (LOOK.thumb_wide - LOOK.thumb) * widened).min(thickness);
        let along = if thumb_length > 0. {
            (thumb_length - 2. * LOOK.inset).max(across)
        } else {
            0.
        };
        let drawn = if along <= 0. {
            (0., 0.)
        } else if horizontal {
            (along, across)
        } else {
            (across, along)
        };
        scene.set_style(thumb, fixed(drawn.0, drawn.1));
        let faded =
            |Color(r, g, b, a): Color, alpha: f32| Color(r, g, b, (a as f32 * alpha).round() as u8);
        // An overlay scrollbar's track shows under the pointer only (as far
        // as it has widened, Windows').
        let track_alpha = match LOOK.widen {
            Some(_) if overlay_mode => alpha * widened,
            _ if overlay_mode && !active => 0.,
            _ => alpha,
        };
        scene.set_kind(
            background,
            NodeKind::Rect(faded(theme.color(tok::SCROLLBAR_TRACK), track_alpha)),
        );
        let thumb_color = theme.color(tok::SCROLLBAR_THUMB);
        let color = if focused.get() {
            theme.color(tok::SCROLLBAR_THUMB_ACTIVE)
        } else if active && LOOK.brightens {
            // Brighter: the colour's alpha raised by three quarters.
            let Color(r, g, b, a) = thumb_color;
            Color(r, g, b, (a as f32 * 1.75).min(255.) as u8)
        } else {
            thumb_color
        };
        scene.set_kind(thumb, thumb_kind(faded(color, alpha), across));
        let value = offset.get().clamp(0., limit);
        let position = if limit > 0. {
            from + (value as f64 / limit as f64 * travel as f64) as f32
        } else {
            from
        };
        let position = if rtl && horizontal {
            length - thumb_length - position
        } else {
            position
        };
        // Its carets, shown as it widens, in the thumb's colour: the
        // decrease one at its start (right to left, a horizontal bar's at
        // its right), each pointing that way.
        if let (Some(cell), Some([(decrease_wash, decrease), (increase_wash, increase)])) =
            (LOOK.arrows, arrows)
        {
            let hovered = arrow_hover.get();
            let strength = if arrow_room > 0. && visible && alpha > 0. {
                alpha * (arrow_room / cell)
            } else {
                0.
            };
            let shown = faded(theme.color(tok::SCROLLBAR_THUMB), strength);
            let (start, end) = if horizontal {
                if rtl { (1, 3) } else { (3, 1) }
            } else {
                (0, 2)
            };
            let across = (thickness - cell) / 2.;
            let (first, last) = match rtl && horizontal {
                true => (length - cell, 0.),
                false => (0., length - cell),
            };
            for (wash, node, toward, at, which) in [
                (decrease_wash, decrease, start, first, 1),
                (increase_wash, increase, end, last, 2),
            ] {
                let size = if strength > 0. { cell } else { 0. };
                // Under the pointer: a wash (the text at 5%), its caret in
                // the secondary text's colour
                // (`ScrollBarButtonArrowForegroundPointerOver`).
                let (wash_color, caret_color) = match hovered == which && size > 0. {
                    true => (
                        faded(theme.color(tok::TEXT_PRIMARY), 0.05 * strength),
                        faded(theme.color(tok::TEXT_SECONDARY), strength),
                    ),
                    false => (Color(0, 0, 0, 0), shown),
                };
                let place = if horizontal {
                    Transform { x: at, y: across }
                } else {
                    Transform { x: across, y: at }
                };
                scene.set_style(wash, fixed(size, size));
                scene.set_kind(wash, wash_kind(wash_color));
                scene.set_transform(wash, place);
                scene.set_style(node, fixed(size, size));
                scene.set_kind(node, caret(caret_color, cell, toward));
                scene.set_transform(node, place);
            }
        }
        // Centred across the track, which sits `margin` in from the edge.
        let side = (thickness - LOOK.margin * 2. - across) / 2. + LOOK.margin;
        let (start, side) = (position + LOOK.inset, side.max(0.));
        scene.set_transform(
            thumb,
            if horizontal {
                Transform { x: start, y: side }
            } else {
                Transform { x: side, y: start }
            },
        );
        drop(scene);
        if visible {
            let mut semantic = SemanticNode::new(
                Role::ScrollBar,
                if horizontal {
                    "Horizontal scrollbar"
                } else {
                    "Vertical scrollbar"
                },
            );
            semantic.scroll_axis = Some(if horizontal {
                ScrollAxis::Horizontal
            } else {
                ScrollAxis::Vertical
            });
            semantic.min = Some(0.);
            semantic.max = Some(limit as f64);
            semantic.numeric_value = Some(value as f64);
            ui.semantics.borrow_mut().set(track, semantic);
        } else {
            ui.semantics.borrow_mut().remove(track);
        }
    });
}
