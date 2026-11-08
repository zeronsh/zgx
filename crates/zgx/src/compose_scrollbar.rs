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
/// colour under the pointer.
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
};
const LOOK: Look = if cfg!(windows) { WINDOWS } else { BUILT_IN };

/// WinUI's widening curve, `cubic-bezier(0, 0, 0, 1)`: x = t³ and
/// y = 3t² − 2t³, so at progress `p`, t = ∛p.
fn widen_curve(p: f32) -> f32 {
    let t = p.clamp(0., 1.).cbrt();
    t * t * (3. - 2. * t)
}

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
            match (fade.next.get(), fade.widen_due()) {
                (Some(next), Some(widen)) => Some(next.min(widen)),
                (next, widen) => next.or(widen),
            }
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
    let thumb = ui.scene.borrow_mut().append(
        track,
        thumb_kind(theme.color_untracked(tok::SCROLLBAR_THUMB), LOOK.thumb),
        Style::default(),
    );
    let main = move |size: (f32, f32)| if horizontal { size.0 } else { size.1 };
    let event_size = size.clone();
    let event_extent = extent.clone();
    let event_offset = offset.clone();
    let scene = ui.scene.clone();
    let focused = ui.signal(false);
    let write_focus = focused.clone();
    // Under the pointer, or dragged: the thumb widens and brightens.
    let active = ui.signal(false);
    let write_active = active.clone();
    let mut grab = None;
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
        let thumb_length = (length * length / total.max(1.))
            .max(LOOK.min_thumb)
            .min(length);
        let travel = (length - thumb_length).max(0.);
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
        let before = event_offset.get() as f64;
        let next = match &cx.event {
            InputEvent::PointerDown {
                button: PointerButton::Primary,
                ..
            } if limit > 0. => {
                cx.focus();
                let pointer = position.unwrap();
                let thumb_start = before / limit * travel;
                if pointer >= thumb_start && pointer <= thumb_start + thumb_length {
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
                (position.unwrap() - grab.unwrap()) / travel * limit
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
        let thumb_length = if visible && alpha > 0. {
            ((length as f64 * length as f64 / total as f64)
                .max(LOOK.min_thumb)
                .min(length as f64)) as f32
        } else {
            0.
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
            (value as f64 / limit as f64 * (length - thumb_length) as f64) as f32
        } else {
            0.
        };
        let position = if rtl && horizontal {
            length - thumb_length - position
        } else {
            position
        };
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
