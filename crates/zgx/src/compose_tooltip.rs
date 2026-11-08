//! Tooltips (`View::tooltip`), as a browser's and the system's behave: shown
//! over a view 300 ms after the pointer comes to rest on it, at once within a
//! second of another (as a tooltip provider's `timeout`), gone as the pointer
//! leaves or presses, or a key is pressed anywhere. One shows at a time. It
//! fades in over 140 ms and out over 100, takes no focus and lets the pointer
//! through. An interactive one (a hover card) keeps open while the pointer is
//! on the view or on it, takes clicks and typing, and closes on Escape.
use crate::{
    compose::{Placement, PlacementEdge, PlacementSide, TaskRunner, TaskToken, View},
    input::{EventPhase, InputEvent, Key},
    style::{Styled, Styles},
    theme::tok,
    widgets::WeakUi,
};
use std::{
    any::Any,
    cell::{Cell, OnceCell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

/// What a tooltip shows and how (`View::tooltip`): text in the theme's look
/// (an overlay card, small text), or a view of the app's own.
pub struct Tooltip {
    content: Content,
    placement: Option<Placement>,
    interactive: bool,
    delay: Option<Duration>,
}
enum Content {
    Text(String),
    View(Box<View>),
}
impl From<&str> for Tooltip {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}
impl From<String> for Tooltip {
    fn from(text: String) -> Self {
        Self {
            content: Content::Text(text),
            placement: None,
            interactive: false,
            delay: None,
        }
    }
}
impl From<View> for Tooltip {
    fn from(view: View) -> Self {
        Self {
            content: Content::View(Box::new(view)),
            placement: None,
            interactive: false,
            delay: None,
        }
    }
}
impl Tooltip {
    /// Where it sits: above the view, centred, 6 points off by default.
    pub fn placement(mut self, placement: Placement) -> Self {
        self.placement = Some(placement);
        self
    }
    /// A hover card the pointer can move into and click in: it stays while
    /// the pointer is on the view or the card (a moment's grace to cross),
    /// typing goes to its fields, and Escape closes it.
    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }
    /// How long the pointer rests on the view before it shows: the
    /// system's by default (`TooltipTiming`: 300 ms, AppKit's; on Windows
    /// twice the mouse hover time); once one shows, the next shows sooner.
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
}

impl View {
    /// Show `tip` over this view while the pointer rests on it (see the
    /// module). Call it last: what follows styles the tooltip's panel.
    pub fn tooltip(self, tip: impl Into<Tooltip>) -> View {
        wrap(self, tip.into())
    }
}

/// A tooltip showing: its number, what closes it, and whether it is an
/// interactive card.
type Showing = (u64, Rc<dyn Fn()>, bool);
thread_local! {
    /// Tooltips showing, and when the last one closed: within a second of
    /// one, the next opens at once.
    static TOOLTIPS: Cell<(usize, Option<Instant>)> = const { Cell::new((0, None)) };
    /// When the last tooltip opened (`TooltipTiming::reshow_from_open`).
    static OPENED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// The tooltip showing (one at most): its number, and what closes it.
    static SHOWING: RefCell<Option<Showing>> = const { RefCell::new(None) };
    static NEXT: Cell<u64> = const { Cell::new(0) };
    /// What hears keys while a tooltip shows: one document listener.
    static KEYS: RefCell<Option<Box<dyn Any>>> = const { RefCell::new(None) };
}

/// A key pressed anywhere: the tooltip showing closes, so it never covers
/// what is typed; an interactive card only on Escape.
fn on_key(event: &InputEvent) {
    let InputEvent::KeyDown { key, .. } = event else {
        return;
    };
    let close = SHOWING.with(|s| {
        s.borrow_mut()
            .take_if(|(_, _, interactive)| !*interactive || *key == Key::Escape)
    });
    if let Some((_, close, _)) = close {
        close();
    }
}

/// Tooltip `open`'s showing ends (`shown` says whether it was showing).
fn hide(open: &crate::reactive::Signal<bool>, shown: &Cell<bool>) {
    if !shown.replace(false) {
        return;
    }
    let left = TOOLTIPS.with(|t| {
        let (count, _) = t.get();
        let left = count.saturating_sub(1);
        t.set((left, Some(crate::clock::now())));
        left
    });
    if left == 0 {
        let keys = KEYS.with(|k| k.borrow_mut().take());
        drop(keys);
    }
    open.set(false);
}

/// When tooltips show (`set_tooltip_timing`): after resting `delay` on a
/// view; within `reshow_within` of another tooltip, after `reshow_delay`
/// (zero: at once). zgx's: 300 ms, AppKit's, and at once within a second of
/// another; Windows': twice the system's mouse hover time, and the hover
/// time within 200 ms of another opening (WinUI's `ToolTipService`, whose
/// one and a half times truncates to one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipTiming {
    /// How long the pointer rests on a view before its tooltip shows.
    pub delay: Duration,
    /// How soon after another tooltip the next is a reshow.
    pub reshow_within: Duration,
    /// How long a reshow waits; zero, none.
    pub reshow_delay: Duration,
    /// Whether `reshow_within` counts from when the last tooltip opened
    /// (WinUI's) rather than from when it closed or while one shows (zgx's).
    pub reshow_from_open: bool,
}
impl Default for TooltipTiming {
    fn default() -> Self {
        Self {
            delay: Duration::from_millis(300),
            reshow_within: Duration::from_secs(1),
            reshow_delay: Duration::ZERO,
            reshow_from_open: false,
        }
    }
}
thread_local! {
    static TIMING: Cell<TooltipTiming> = Cell::new(TooltipTiming::default());
}
/// Show tooltips with `timing` on this thread from now on (a desktop host
/// sets the system's: Windows' hover time). A tooltip's own `delay` still
/// waits its own.
pub fn set_tooltip_timing(timing: TooltipTiming) {
    TIMING.with(|t| t.set(timing));
}

/// `f` after `delay` on `runner`'s tasks; at once without one.
fn after(
    runner: &Option<Rc<TaskRunner>>,
    delay: Duration,
    f: impl FnOnce() + 'static,
) -> Option<TaskToken> {
    match runner {
        Some(runner) => Some(runner.spawn(Box::pin(async move {
            crate::timer::sleep(delay).await;
            f();
        }))),
        None => {
            f();
            None
        }
    }
}

fn wrap(anchor: View, tip: Tooltip) -> View {
    crate::compose::component(move |cx| {
        let runtime = cx.runtime();
        let runner = cx.try_service::<TaskRunner>();
        let Tooltip {
            content,
            placement,
            interactive,
            delay,
        } = tip;
        let timing = TIMING.with(Cell::get);
        let delay = delay.unwrap_or(timing.delay);
        // A text tip describes the view to assistive technology, which
        // doesn't see the tip itself; a view tip is the app's to name.
        let description = match &content {
            Content::Text(text) => Some(text.clone()),
            Content::View(_) => None,
        };
        let content = match content {
            Content::View(view) => *view,
            Content::Text(text) => {
                let look = &crate::compose::OVERLAY;
                let (x, above, below) = look.tooltip_padding;
                let panel = crate::compose::column().px(x);
                let panel = match above == below {
                    true => panel.py(above),
                    false => panel.pt(above).pb(below),
                };
                panel
                    .max_w(320.)
                    .bg(tok::POPOVER_BG)
                    .border(0.5)
                    .border_color(tok::BORDER)
                    .rounded(look.tooltip_radius)
                    .text_size(look.tooltip_font_size)
                    .text_wrap(true)
                    .child(crate::compose::text(text))
            }
        };
        let open = runtime.signal(false);
        let shown = Rc::new(Cell::new(false));
        let ui: Rc<OnceCell<WeakUi>> = Rc::default();
        // Its fade (in over 140 ms, out over 100, both easing out; none
        // when it takes over from another at once).
        let fade = runtime.signal(0_f32);
        let fading: Rc<RefCell<Option<TaskToken>>> = Rc::default();
        let animate = {
            let (fade, fading, runner) = (fade.clone(), fading.clone(), runner.clone());
            move |to: f32, ms: f32, done: Option<Rc<dyn Fn()>>| {
                fading.borrow_mut().take();
                let from = fade.with_untracked(|v| *v);
                let Some(runner) = runner.as_ref().filter(|_| ms > 0. && from != to) else {
                    fade.set(to);
                    if let Some(done) = done {
                        done();
                    }
                    return;
                };
                let start = crate::clock::now();
                let fade = fade.clone();
                *fading.borrow_mut() = Some(runner.spawn(Box::pin(async move {
                    loop {
                        crate::timer::sleep(Duration::from_millis(8)).await;
                        let t = (crate::clock::since(start).as_secs_f32() * 1000. / ms).min(1.);
                        fade.set(from + (to - from) * (1. - (1. - t).powi(3)));
                        if t >= 1. {
                            break;
                        }
                    }
                    if let Some(done) = done {
                        done();
                    }
                })));
            }
        };
        let number = NEXT.with(|n| {
            n.set(n.get() + 1);
            n.get()
        });
        let set = {
            let (open, shown, fade, fading, ui) = (
                open.clone(),
                shown.clone(),
                fade.clone(),
                fading.clone(),
                ui.clone(),
            );
            move |on: bool, instant: bool| {
                if !on {
                    SHOWING.with(|s| {
                        let mut s = s.borrow_mut();
                        if s.as_ref().is_some_and(|(n, _, _)| *n == number) {
                            *s = None;
                        }
                    });
                    if !shown.get() {
                        return;
                    }
                    let (open, shown) = (open.clone(), shown.clone());
                    let close: Rc<dyn Fn()> = Rc::new(move || hide(&open, &shown));
                    animate(0., if instant { 0. } else { 100. }, Some(close));
                    return;
                }
                let close: Rc<dyn Fn()> = {
                    let (open, shown, fade, fading) =
                        (open.clone(), shown.clone(), fade.clone(), fading.clone());
                    Rc::new(move || {
                        fading.borrow_mut().take();
                        fade.set(0.);
                        hide(&open, &shown);
                    })
                };
                // One shows at a time: this one closes the last, at once.
                let last = SHOWING.with(|s| s.borrow_mut().replace((number, close, interactive)));
                if let Some((n, close_last, _)) = last
                    && n != number
                {
                    close_last();
                }
                if !shown.replace(true) {
                    let first = TOOLTIPS.with(|t| {
                        let (count, last) = t.get();
                        t.set((count + 1, last));
                        OPENED.with(|o| o.set(Some(crate::clock::now())));
                        count == 0
                    });
                    if first && let Some(ui) = ui.get().and_then(WeakUi::upgrade) {
                        let keys: Box<dyn Any> = Box::new(ui.listen_document(on_key));
                        KEYS.with(|k| *k.borrow_mut() = Some(keys));
                    }
                    open.set(true);
                }
                animate(1., if instant { 0. } else { 140. }, None);
            }
        };
        let waiting: Rc<RefCell<Option<TaskToken>>> = Rc::default();
        // An interactive card: the pointer may cross from the view to the
        // card (a moment's grace) and stay on either.
        let (over_anchor, over_card) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
        let leave = {
            let (set, runner, waiting) = (set.clone(), runner.clone(), waiting.clone());
            let (over_anchor, over_card) = (over_anchor.clone(), over_card.clone());
            move || {
                if !interactive || runner.is_none() {
                    waiting.borrow_mut().take();
                    set(false, false);
                    return;
                }
                let (set, over_anchor, over_card) =
                    (set.clone(), over_anchor.clone(), over_card.clone());
                *waiting.borrow_mut() = after(&runner, Duration::from_millis(120), move || {
                    if !over_anchor.get() && !over_card.get() {
                        set(false, false);
                    }
                });
            }
        };
        let anchor = match description {
            Some(text) => anchor.accessible_description(text),
            None => anchor,
        };
        let anchor = {
            let (leave, over_anchor, ui) = (leave.clone(), over_anchor.clone(), ui.clone());
            let ui_focus = ui.clone();
            anchor
                .on_mount(move |handle| {
                    let _ = ui.set(handle.weak_ui());
                })
                .on_event(move |cx| {
                    if cx.phase == EventPhase::Capture {
                        return;
                    }
                    match cx.event {
                        InputEvent::PointerEnter if cx.target == cx.current_target => {
                            over_anchor.set(true);
                            // Back while it shows, or another just showed: a
                            // reshow, at once or after its own wait.
                            let reshow = match timing.reshow_from_open {
                                true => OPENED.with(Cell::get).is_some_and(|at| {
                                    crate::clock::since(at) < timing.reshow_within
                                }),
                                false => TOOLTIPS.with(|t| {
                                    let (count, last) = t.get();
                                    count > 0
                                        || last.is_some_and(|at| {
                                            crate::clock::since(at) < timing.reshow_within
                                        })
                                }),
                            };
                            let now = shown.get() || (reshow && timing.reshow_delay.is_zero());
                            if reshow && !now && runner.is_some() {
                                let set = set.clone();
                                *waiting.borrow_mut() =
                                    after(&runner, timing.reshow_delay, move || set(true, false));
                            } else if now || runner.is_none() {
                                // Taking over from another (or back while it
                                // fades): no fade in from nothing.
                                waiting.borrow_mut().take();
                                set(true, !shown.get());
                            } else {
                                let set = set.clone();
                                *waiting.borrow_mut() =
                                    after(&runner, delay, move || set(true, false));
                            }
                        }
                        InputEvent::PointerLeave if cx.target == cx.current_target => {
                            over_anchor.set(false);
                            leave();
                        }
                        // Focused from the keyboard, it shows (as WAI-ARIA's
                        // tooltip does), and goes as focus leaves; a click's
                        // focus shows none.
                        InputEvent::Focus if cx.target == cx.current_target => {
                            let keyboard = ui_focus
                                .get()
                                .and_then(WeakUi::upgrade)
                                .is_some_and(|ui| ui.input.focus_visible_now());
                            if keyboard {
                                waiting.borrow_mut().take();
                                set(true, false);
                            }
                        }
                        InputEvent::Blur
                            if cx.target == cx.current_target && !over_anchor.get() =>
                        {
                            waiting.borrow_mut().take();
                            set(false, false);
                        }
                        InputEvent::PointerDown { .. } | InputEvent::Scroll { .. }
                            if !interactive =>
                        {
                            waiting.borrow_mut().take();
                            set(false, true);
                        }
                        _ => {}
                    }
                })
        };
        let content = if interactive {
            content.on_event(move |cx| {
                if cx.phase == EventPhase::Capture || cx.target != cx.current_target {
                    return;
                }
                match cx.event {
                    InputEvent::PointerEnter => over_card.set(true),
                    InputEvent::PointerLeave => {
                        over_card.set(false);
                        leave();
                    }
                    _ => {}
                }
            })
        } else {
            content
        };
        let placement = placement.unwrap_or(Placement {
            side: PlacementSide::Top,
            align: PlacementEdge::Center,
            offset: 6.,
        });
        let content = crate::compose::column()
            .child(content)
            .reactive_style(move || Styles::new().opacity(fade.get()));
        let panel = crate::compose::popover("Tooltip", open, anchor)
            .layer()
            .lazy()
            .autofocus(false)
            .placement(placement)
            .pointer_events(interactive)
            .child(content);
        // A plain tip is the view's description, not a dialog of its own.
        if interactive {
            panel
        } else {
            panel.panel_hidden()
        }
    })
}
