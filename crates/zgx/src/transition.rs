//! Style transitions (docs/design/style-transitions.md): a view's opacity
//! or colours easing from the value drawn to a new one over a duration on
//! a curve, as CSS transitions do, whatever changed it (a hover, a
//! `reactive_style`, a theme). Step one: opacity, background and border
//! colour, from Rust (`Styled::transition`).
//!
//! What transitions is the drawn value: the style effect writes its target
//! as ever (`compose_style`), then the window's `Transitions` puts the
//! eased value back in its place and steps it toward the target on the
//! host's interaction deadlines (`Ui::next_interaction_deadline`, as the
//! overlay scrollbar's fade and the drag spring step): one deadline a
//! refresh while something eases, none once nothing does. A transition that
//! ends writes its target exactly and leaves the set.
use crate::scene::{Color, NodeId, NodeKind, Scene};
use crate::theme::{Themed, tok};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// A property a transition eases (`Transition::of`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransitionProp {
    /// The node's opacity.
    Opacity,
    /// Its box's fill.
    Background,
    /// Its border's colour.
    BorderColor,
}
impl TransitionProp {
    const ALL: [Self; 3] = [Self::Opacity, Self::Background, Self::BorderColor];
    fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A transition's curve, from progress 0..1: linear, or a CSS
/// `cubic-bezier` (the theme's `motion.ease.*` tokens, as
/// `cubicBezier(x1, y1, x2, y2)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    /// Straight.
    Linear,
    /// `cubic-bezier(x1, y1, x2, y2)`.
    CubicBezier(f32, f32, f32, f32),
}
impl Ease {
    /// Parsed from `"linear"` or `"cubicBezier(x1, y1, x2, y2)"` (also
    /// `cubic-bezier(…)`); `None` otherwise.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("linear") {
            return Some(Self::Linear);
        }
        let inner = text
            .strip_prefix("cubicBezier(")
            .or_else(|| text.strip_prefix("cubic-bezier("))?
            .strip_suffix(')')?;
        let n: Vec<f32> = inner
            .split(',')
            .map(|part| part.trim().parse().ok())
            .collect::<Option<_>>()?;
        match n[..] {
            [x1, y1, x2, y2] if (0. ..=1.).contains(&x1) && (0. ..=1.).contains(&x2) => {
                Some(Self::CubicBezier(x1, y1, x2, y2))
            }
            _ => None,
        }
    }
    /// The eased value at progress `p`.
    pub fn at(self, p: f32) -> f32 {
        let p = p.clamp(0., 1.);
        let Self::CubicBezier(x1, y1, x2, y2) = self else {
            return p;
        };
        // Its ends exactly: an eased value at either is the end itself.
        if p == 0. || p == 1. {
            return p;
        }
        let bezier = |t: f32, a: f32, b: f32| {
            let u = 1. - t;
            3. * u * u * t * a + 3. * u * t * t * b + t * t * t
        };
        // The curve's t whose x is `p`: bisection, x being monotonic.
        let (mut low, mut high) = (0f32, 1f32);
        for _ in 0..24 {
            let mid = (low + high) / 2.;
            if bezier(mid, x1, x2) < p {
                low = mid;
            } else {
                high = mid;
            }
        }
        bezier((low + high) / 2., y1, y2)
    }
}

/// How a view's properties transition (`Styled::transition`).
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    props: u8,
    duration: Themed<f32>,
    ease: Result<Ease, crate::theme::TextToken>,
    delay: Duration,
}
impl Transition {
    /// Ease `props`, over `motion.duration.normal` on `motion.ease.standard`
    /// unless told otherwise.
    pub fn of(props: impl IntoIterator<Item = TransitionProp>) -> Self {
        Self {
            props: props.into_iter().fold(0, |bits, prop| bits | prop.bit()),
            duration: Themed::Token(tok::MOTION_DURATION_NORMAL.0),
            ease: Err(tok::MOTION_EASE_STANDARD),
            delay: Duration::ZERO,
        }
    }
    /// Ease every property transitions can.
    pub fn all() -> Self {
        Self::of(TransitionProp::ALL)
    }
    /// Over `ms` milliseconds, or a duration token (a theme's motion
    /// durations, 0 under reduced motion).
    pub fn duration(mut self, ms: impl Into<Themed<f32>>) -> Self {
        self.duration = ms.into();
        self
    }
    /// On `ease`.
    pub fn ease(mut self, ease: Ease) -> Self {
        self.ease = Ok(ease);
        self
    }
    /// On an easing token's curve (`motion.ease.*`).
    pub fn ease_token(mut self, token: crate::theme::TextToken) -> Self {
        self.ease = Err(token);
        self
    }
    /// Starting `ms` milliseconds after the change (CSS's
    /// `transition-delay`).
    pub fn delay(mut self, ms: f32) -> Self {
        self.delay = Duration::from_secs_f32(ms.max(0.) / 1000.);
        self
    }
    fn eases(&self, prop: TransitionProp) -> bool {
        self.props & prop.bit() != 0
    }
    /// Its duration and curve in `theme`.
    fn resolved(&self, theme: &crate::theme::ThemeScope) -> (Duration, Ease) {
        let ms = match self.duration {
            Themed::Value(ms) => ms,
            Themed::Token(token) => theme.number_untracked(crate::theme::NumberToken(token)),
        };
        let ease = match self.ease {
            Ok(ease) => ease,
            Err(token) => Ease::parse(&theme.text(token)).unwrap_or(Ease::Linear),
        };
        (Duration::from_secs_f32(ms.max(0.) / 1000.), ease)
    }
}

/// A transitioned property's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Value {
    Number(f32),
    Color(Color),
}
impl Value {
    fn lerp(self, to: Self, t: f32) -> Self {
        match (self, to) {
            (Self::Number(a), Self::Number(b)) => Self::Number(a + (b - a) * t),
            (Self::Color(a), Self::Color(b)) => Self::Color(crate::theme::mix(a, b, t)),
            (_, to) => to,
        }
    }
}

/// What a node draws of the properties transitions ease.
pub(crate) fn drawn(scene: &Scene, node: NodeId) -> [Option<Value>; 3] {
    let quad = match scene.kind(node) {
        NodeKind::Panel { quad, .. } | NodeKind::Quad(quad) => Some(quad),
        _ => None,
    };
    [
        Some(Value::Number(scene.effects(node).opacity)),
        quad.map(|quad| Value::Color(quad.fill)),
        quad.map(|quad| Value::Color(quad.border_color)),
    ]
}

/// Draw `value` as node `node`'s `prop`.
fn write(scene: &mut Scene, node: NodeId, prop: TransitionProp, value: Value) {
    match (prop, value) {
        (TransitionProp::Opacity, Value::Number(opacity)) => {
            let mut effects = scene.effects(node);
            effects.opacity = opacity;
            scene.set_effects(node, effects);
        }
        (TransitionProp::Background | TransitionProp::BorderColor, Value::Color(color)) => {
            let mut kind = scene.kind(node).clone();
            let quad = match &mut kind {
                NodeKind::Panel { quad, .. } | NodeKind::Quad(quad) => quad,
                _ => return,
            };
            match prop {
                TransitionProp::Background => quad.fill = color,
                _ => quad.border_color = color,
            }
            scene.set_kind(node, kind);
        }
        _ => {}
    }
}

/// One property easing.
#[derive(Clone, Copy, Debug)]
struct Running {
    node: NodeId,
    prop: TransitionProp,
    from: Value,
    to: Value,
    start: Instant,
    duration: Duration,
    ease: Ease,
    /// How far it had run when last stepped, 0..1.
    reached: f32,
}
impl Running {
    /// Its progress at `now`, 0..1.
    fn progress(&self, now: Instant) -> f32 {
        if now <= self.start {
            return 0.;
        }
        match self.duration.is_zero() {
            true => 1.,
            false => {
                (now.duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32()).min(1.)
            }
        }
    }
    fn value(&self, now: Instant) -> Value {
        self.from.lerp(self.to, self.ease.at(self.progress(now)))
    }
}

/// A window's transitions running (`Ui::transitions`).
#[derive(Default)]
pub(crate) struct Transitions {
    running: RefCell<Vec<Running>>,
    /// When they next step: now as one starts, a refresh on after each.
    next: std::cell::Cell<Option<Instant>>,
}
impl Transitions {
    /// When they next step; `None` with none running.
    pub(crate) fn next(&self) -> Option<Instant> {
        self.next.get()
    }

    /// After node `node`'s style was written (with `spec`, drawing `before`
    /// beforehand): each eased property whose target changed starts, or
    /// turns, from what was drawn, and what is drawn is put back as it eases.
    pub(crate) fn settle(
        &self,
        scene: &mut Scene,
        theme: &crate::theme::ThemeScope,
        node: NodeId,
        spec: &Transition,
        before: [Option<Value>; 3],
    ) {
        let now = crate::clock::now();
        let after = drawn(scene, node);
        let mut running = self.running.borrow_mut();
        let mut started = false;
        for (index, prop) in TransitionProp::ALL.into_iter().enumerate() {
            if !spec.eases(prop) {
                continue;
            }
            let Some(target) = after[index] else {
                continue;
            };
            let at = running
                .iter()
                .position(|r| r.node == node && r.prop == prop);
            match at.map(|at| running[at]) {
                // The same target: the write clobbered the eased value.
                Some(current) if current.to == target => {
                    if let Some(drawn) = before[index] {
                        write(scene, node, prop, drawn);
                    }
                }
                Some(current) => {
                    // From what is drawn, wherever it got to.
                    let drawn = before[index].unwrap_or(current.from);
                    let (duration, ease) = spec.resolved(theme);
                    // Reversed (back to where it came from): as long as it
                    // has run, as CSS's reversing shortening factor.
                    let duration = match target == current.from {
                        true => duration.mul_f32(current.reached),
                        false => duration,
                    };
                    running[at.unwrap()] = Running {
                        node,
                        prop,
                        from: drawn,
                        to: target,
                        start: now,
                        duration,
                        ease,
                        reached: 0.,
                    };
                    write(scene, node, prop, drawn);
                    started = true;
                }
                None => {
                    let Some(from) = before[index].filter(|from| *from != target) else {
                        continue;
                    };
                    let (duration, ease) = spec.resolved(theme);
                    if duration.is_zero() && spec.delay.is_zero() {
                        continue;
                    }
                    running.push(Running {
                        node,
                        prop,
                        from,
                        to: target,
                        start: now + spec.delay,
                        duration,
                        ease,
                        reached: 0.,
                    });
                    write(scene, node, prop, from);
                    started = true;
                }
            }
        }
        if started && self.next.get().is_none_or(|next| next > now) {
            self.next.set(Some(now));
        }
    }

    /// Step every transition due by `now` (a step a refresh, `interval`);
    /// whether any did.
    pub(crate) fn advance(&self, scene: &mut Scene, now: Instant, interval: Duration) -> bool {
        if self.next.get().is_none_or(|next| next > now) {
            return false;
        }
        let mut running = self.running.borrow_mut();
        running.retain_mut(|r| {
            if !scene.contains(r.node) {
                return false;
            }
            r.reached = r.progress(now);
            let done = r.reached >= 1.;
            write(
                scene,
                r.node,
                r.prop,
                if done { r.to } else { r.value(now) },
            );
            !done
        });
        self.next.set((!running.is_empty()).then(|| now + interval));
        true
    }

    /// Drop node `node`'s transitions, its targets drawn (`node` leaving,
    /// or its transitions taken away).
    #[allow(dead_code)]
    pub(crate) fn cancel(&self, scene: &mut Scene, node: NodeId) {
        let mut running = self.running.borrow_mut();
        running.retain(|r| {
            if r.node != node {
                return true;
            }
            if scene.contains(node) {
                write(scene, node, r.prop, r.to);
            }
            false
        });
        if running.is_empty() {
            self.next.set(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cubic_bezier_eases_as_css_does() {
        let ease = Ease::parse("cubicBezier(0.2, 0, 0, 1)").unwrap();
        assert_eq!(ease.at(0.), 0.);
        assert_eq!(ease.at(1.), 1.);
        assert!(ease.at(0.5) > 0.5, "decelerating: {}", ease.at(0.5));
        assert_eq!(Ease::parse("linear"), Some(Ease::Linear));
        assert!(Ease::parse("cubic-bezier(0.25, 0.46, 0.45, 0.94)").is_some());
        assert_eq!(Ease::parse("bounce"), None);
    }
}
