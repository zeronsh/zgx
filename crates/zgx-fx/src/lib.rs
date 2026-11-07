//! Ready-made visual effects for zgx, made with its public API alone: layer
//! shaders reading `zgx_backdrop` and painting past their box, shader fills, and
//! `View::shader_drive` keeping them up. An app that doesn't depend on this
//! crate pays nothing for it. Rust: `view.fx(Glow::neon())`. Luau:
//! `luau/fx.luau` in your script's directory, `require("fx")`.
//!
//! Each effect is WGSL with typed parameters and named presets. Given its time
//! and seed a frame is the same on every GPU (integer hashes, no `sin` hash).
//! One with no intensity, a one-shot with none alive and one at rest isn't
//! drawn. What moves steps on the window's frame clock with no script, and only
//! while its node shows. Under reduced motion each holds a calm pose.
#![warn(missing_docs)]
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
use zgx::compose::{Context, NodeHandle, Tasks, View, component};
use zgx::decoration::{FillShader, ShaderUniforms};
use zgx::frame::FrameClock;
use zgx::reactive::Signal;
use zgx::scene::{Color, NodeId, Scene};
use zgx::shader_drive::ShaderDrive;

mod effects;
pub use effects::*;

/// How an effect draws: through its node's layer, or filling its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxKind {
    /// A layer shader: the node's subtree cached, composited through it.
    Layer,
    /// A shader fill: the node's background, under its children. No layer,
    /// nothing past the box.
    Fill,
}

/// A value for a parameter by name (`Fx::build`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FxValue<'a> {
    /// A number.
    Number(f32),
    /// A colour.
    Color(Color),
    /// One of a choice, by name.
    Text(&'a str),
    /// Yes or no.
    Bool(bool),
}

/// A parameter's type, as it goes in a shader's lanes.
pub trait FxField: Sized {
    /// Write it from `lanes`' start.
    fn write(&self, lanes: &mut [f32]);
    /// From a value by name, if that's its type.
    fn from_value(value: FxValue<'_>) -> Option<Self>;
}
impl FxField for f32 {
    fn write(&self, lanes: &mut [f32]) {
        lanes[0] = *self;
    }
    fn from_value(value: FxValue<'_>) -> Option<Self> {
        match value {
            FxValue::Number(number) if number.is_finite() => Some(number),
            _ => None,
        }
    }
}
impl FxField for bool {
    fn write(&self, lanes: &mut [f32]) {
        lanes[0] = f32::from(u8::from(*self));
    }
    fn from_value(value: FxValue<'_>) -> Option<Self> {
        match value {
            FxValue::Bool(on) => Some(on),
            _ => None,
        }
    }
}
impl FxField for Color {
    fn write(&self, lanes: &mut [f32]) {
        lanes[0] = f32::from(self.0) / 255.;
        lanes[1] = f32::from(self.1) / 255.;
        lanes[2] = f32::from(self.2) / 255.;
    }
    fn from_value(value: FxValue<'_>) -> Option<Self> {
        match value {
            FxValue::Color(color) => Some(color),
            _ => None,
        }
    }
}

/// An effect, ready to draw: `From` each effect's parameters. Clones share a
/// one-shot's pool, so one kept to `fire` fires the one on the node.
#[derive(Clone, Debug)]
pub struct Fx {
    name: &'static str,
    kind: FxKind,
    source: &'static str,
    fps: Option<f32>,
    params: [[f32; 4]; 4],
    bleed: f32,
    reach: f32,
    frost: f32,
    animated: bool,
    idle: bool,
    shots: Option<Shots>,
    life: f32,
}
impl Fx {
    /// The effect's name, as the Luau module has it.
    pub fn name(&self) -> &'static str {
        self.name
    }
    /// How it draws.
    pub fn kind(&self) -> FxKind {
        self.kind
    }
    /// Its shader's source: what `zgx_gpu::check_layer_shader` or
    /// `check_fill_shader` takes.
    pub fn source(&self) -> &'static str {
        self.source
    }
    /// How far past the box it paints, in points.
    pub fn bleed(&self) -> f32 {
        self.bleed
    }
    /// How far past the box and bleed it reads what's behind it, in points.
    pub fn reach(&self) -> f32 {
        self.reach
    }
    /// How wide a backdrop blur it draws over (a frosted lens).
    pub fn frost(&self) -> f32 {
        self.frost
    }
    /// Whether it changes with time, so a clock steps it.
    pub fn is_animated(&self) -> bool {
        self.animated
    }
    /// Whether it draws nothing as it is: no intensity, at rest, or a
    /// one-shot with none alive.
    pub fn is_idle(&self) -> bool {
        self.idle || self.shots.as_ref().is_some_and(|shots| !shots.alive())
    }
    /// The one-shots of a one-shot effect.
    pub fn shots(&self) -> Option<&Shots> {
        self.shots.as_ref()
    }
    /// Fire a one-shot at `x`, `y` (points from the box's top left): it
    /// overlaps those alive. Nothing on an effect that isn't one.
    pub fn fire(&self, x: f32, y: f32) {
        if let Some(shots) = &self.shots {
            shots.fire(x, y);
        }
    }
    /// Its shader, one per source and rate for a thread: its pipeline is
    /// made once, and starts compiling for a one-shot ahead of its first firing.
    pub fn shader(&self) -> Arc<FillShader> {
        type Shaders = HashMap<(usize, Option<u32>), Arc<FillShader>>;
        thread_local! {
            static SHADERS: RefCell<Shaders> = RefCell::default();
        }
        SHADERS.with(|shaders| {
            shaders
                .borrow_mut()
                .entry((self.source.as_ptr() as usize, self.fps.map(f32::to_bits)))
                .or_insert_with(|| {
                    let shader = FillShader::new(self.source, self.fps);
                    if self.kind == FxKind::Layer && self.shots.is_some() {
                        zgx::decoration::declare_layer_shader(&shader);
                    }
                    shader
                })
                .clone()
        })
    }
    /// What it draws with at `time` seconds (one-shots, at `now`): `reduced`,
    /// motion is reduced, its calm pose.
    pub fn uniforms(&self, time: f32, reduced: bool, now: Instant) -> ShaderUniforms {
        let mut params = self.params;
        match &self.shots {
            Some(shots) => shots.write(&mut params, self.life, reduced, now),
            None => params[3][2] = f32::from(u8::from(reduced)),
        }
        ShaderUniforms {
            time: if reduced { 0. } else { time },
            params,
        }
    }
    /// What `View::shader_drive` keeps up, as it is now.
    pub fn drive(&self, reduced: bool, now: Instant) -> ShaderDrive {
        let idle = self.is_idle();
        ShaderDrive {
            shader: (self.kind == FxKind::Fill || !idle).then(|| self.shader()),
            fill: self.kind == FxKind::Fill,
            params: self.uniforms(0., reduced, now).params,
            bleed: self.bleed,
            reach: self.reach,
            frost: self.frost,
            running: !idle && self.animated && self.shots.is_none(),
        }
    }
    /// Put it on `node` in `scene`, as it draws at `time`. A fill's shader is
    /// the view's, and here only its uniforms are set.
    pub fn apply(&self, scene: &mut Scene, node: NodeId, time: f32, reduced: bool, now: Instant) {
        let time = if reduced { 0. } else { time };
        self.drive(reduced, now).apply(scene, node, time);
    }
    /// Its rate: at most this many steps a second; `None`, every frame.
    pub fn rate(&self) -> Option<f32> {
        self.fps
    }
}

/// An effect on a view.
pub trait FxExt {
    /// Draw `fx` on this view: `.fx(Glow::neon())`. Last in its chain: what it
    /// returns wraps the view, already styled.
    fn fx(self, fx: impl Into<Fx>) -> View;
    /// [`fx`](Self::fx), `make` read again when state it reads changes (an
    /// effect's intensity following a hover). It stays an effect of one kind:
    /// a fill's shader is the first's.
    fn fx_with(self, make: impl FnMut() -> Fx + 'static) -> View;
}
impl FxExt for View {
    fn fx(self, fx: impl Into<Fx>) -> View {
        let fx = fx.into();
        self.fx_with(move || fx.clone())
    }
    fn fx_with(self, make: impl FnMut() -> Fx + 'static) -> View {
        component(move |cx| mounted(cx, self, make))
    }
}
fn mounted(cx: &mut Context, view: View, mut make: impl FnMut() -> Fx + 'static) -> View {
    let themes = zgx::theme::Themes::of(&cx.runtime());
    let fired = cx.state(0u64);
    // A one-shot's step: a task of the component's, on its frame clock.
    let stepper = cx
        .try_service::<zgx::compose::TaskRunner>()
        .map(|_| (cx.tasks(), cx.frames()));
    let hooked = fired.clone();
    let held: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let handle = held.clone();
    let view = zgx::shader_drive::attach(cx, view, move || {
        let fx = make();
        fired.get();
        if let Some(shots) = &fx.shots {
            shots.hook(&hooked, stepper.clone(), handle.clone(), fx.clone());
        }
        fx.drive(
            themes.system_preferences().reduced_motion,
            zgx::clock::now(),
        )
    });
    view.on_mount(move |mounted| *held.borrow_mut() = Some(mounted))
}

/// A one-shot effect's shots alive: a pool of three, as many as its uniforms
/// hold. Fired past that, the oldest fades out in a moment (`FADE`, not cut)
/// and the new one starts in its place.
#[derive(Clone, Default)]
pub struct Shots(Rc<RefCell<Pool>>);
impl std::fmt::Debug for Shots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Shots({} alive)", self.0.borrow().live.len())
    }
}
type Stepper = Option<(Tasks, FrameClock)>;
#[derive(Default)]
struct Pool {
    live: VecDeque<Shot>,
    next: Option<(f32, f32)>,
    /// Told when none are alive, or the first is: what's mounted follows it.
    fired: Option<Signal<u64>>,
    stepper: Stepper,
    node: Option<Rc<RefCell<Option<NodeHandle>>>>,
    fx: Option<Box<Fx>>,
    stepping: bool,
}
#[derive(Clone, Copy, Debug)]
struct Shot {
    x: f32,
    y: f32,
    fired: Option<Instant>,
    leaving: Option<Instant>,
}
/// How many shots are alive at once.
pub const SHOTS: usize = 3;
/// How long the oldest takes to leave for a new one, in seconds.
pub const FADE: f32 = 0.16;
impl Shots {
    /// What a mounted effect needs of its component: to tell it, and step it.
    fn hook(
        &self,
        fired: &Signal<u64>,
        stepper: Stepper,
        node: Rc<RefCell<Option<NodeHandle>>>,
        fx: Fx,
    ) {
        let mut pool = self.0.borrow_mut();
        if pool.fired.is_none() {
            pool.fired = Some(fired.clone());
            pool.stepper = stepper;
            pool.node = Some(node);
            pool.fx = Some(Box::new(fx));
        }
    }
    /// Fire one at `x`, `y`.
    pub fn fire(&self, x: f32, y: f32) {
        let mut pool = self.0.borrow_mut();
        let first = pool.live.is_empty();
        if pool.live.len() < SHOTS {
            pool.live.push_back(Shot {
                x,
                y,
                fired: None,
                leaving: None,
            });
        } else {
            pool.next = Some((x, y));
            let now = zgx::clock::now();
            if let Some(oldest) = pool.live.front_mut() {
                oldest.leaving.get_or_insert(now);
            }
        }
        let fired = pool.fired.clone().filter(|_| first);
        let start = !pool.stepping;
        drop(pool);
        if let Some(fired) = fired {
            fired.mutate(|count| {
                *count += 1;
                true
            });
        }
        if start {
            self.step();
        }
    }
    /// Run its frames, until none is alive.
    fn step(&self) {
        let shots = self.clone();
        let mut pool = self.0.borrow_mut();
        let (Some((tasks, frames)), Some(node), Some(fx)) =
            (pool.stepper.clone(), pool.node.clone(), pool.fx.clone())
        else {
            return;
        };
        pool.stepping = true;
        drop(pool);
        tasks.spawn(async move {
            loop {
                let frame = frames.next().await;
                let Some(scene) = node.borrow().as_ref().and_then(NodeHandle::scene) else {
                    break;
                };
                let Some(id) = node.borrow().as_ref().map(NodeHandle::node) else {
                    break;
                };
                let mut uniforms = fx.uniforms(0., false, frame.time);
                if let Ok(mut scene) = scene.try_borrow_mut()
                    && scene.contains(id)
                {
                    uniforms.time = scene
                        .layer_shader(id)
                        .map_or(0., |layer| layer.uniforms.time);
                    scene.set_layer_uniforms(id, uniforms);
                }
                if !shots.alive() {
                    break;
                }
            }
            let mut pool = shots.0.borrow_mut();
            pool.stepping = false;
            let fired = pool.fired.clone();
            drop(pool);
            // None alive: what's mounted takes its shader away.
            if let Some(fired) = fired {
                fired.mutate(|count| {
                    *count += 1;
                    true
                });
            }
        });
    }
    /// Whether any is alive.
    pub fn alive(&self) -> bool {
        !self.0.borrow().live.is_empty()
    }
    fn write(&self, params: &mut [[f32; 4]; 4], life: f32, reduced: bool, now: Instant) {
        // Calmer under reduced motion, and over sooner.
        let life = if reduced { 0.6 } else { life.max(0.01) };
        let intensity = params[3][1];
        let mut pool = self.0.borrow_mut();
        let gone = |shot: &Shot| {
            let age = shot
                .fired
                .is_some_and(|fired| now.saturating_duration_since(fired).as_secs_f32() >= life);
            let left = shot
                .leaving
                .is_some_and(|at| now.saturating_duration_since(at).as_secs_f32() >= FADE);
            age || left
        };
        pool.live.retain(|shot| !gone(shot));
        if pool.live.len() < SHOTS
            && let Some((x, y)) = pool.next.take()
        {
            pool.live.push_back(Shot {
                x,
                y,
                fired: None,
                leaving: None,
            });
        }
        for row in &mut params[1..] {
            *row = [0.; 4];
        }
        for (row, shot) in pool.live.iter_mut().enumerate() {
            let fired = *shot.fired.get_or_insert(now);
            let age = now.saturating_duration_since(fired).as_secs_f32() / life;
            let leaving = shot.leaving.map_or(1., |at| {
                1. - (now.saturating_duration_since(at).as_secs_f32() / FADE).min(1.)
            });
            params[row + 1] = [
                shot.x,
                shot.y,
                // Below 0 with motion reduced (and never 0 there).
                if reduced { -age.max(1e-3) } else { age },
                intensity * leaving,
            ];
        }
    }
}
