//! A node's shader kept up by the engine: its uniforms following what they
//! read, and its time stepping on the window's frame clock, with no script,
//! style pass or layout per step.
//!
//! Time steps only while it's asked to (`ShaderDrive::running`), its node shows
//! and its window is presented, at most at its shader's rate. Otherwise it holds
//! no task and asks for no frames: following visibility, not polling it, so an
//! off-screen shader wakes nothing. Under the system's reduced motion it holds
//! still at time 0 (a shader takes a pose of its own from `SystemPreferences`,
//! `ui.prefers.reduced_motion`, through its parameters).
//!
//! What a shader's parameters mean, and any effect made with it, are its
//! author's: nothing here knows one from another.
use crate::compose::{Context, TaskRunner, TaskToken, View};
use crate::decoration::{FillShader, ShaderUniforms};
use crate::frame::{Frame, FrameClock};
use crate::scene::{NodeId, Scene};
use crate::style::Styled;
use crate::widgets::{Ui, WeakUi};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// What a node's shader draws with now (`View::shader_drive`).
#[derive(Clone, Debug, Default)]
pub struct ShaderDrive {
    /// A layer shader (`Scene::set_layer_shader`), or with `fill` a shader
    /// fill. `None`: none. A node with none has no layer, costing nothing.
    pub shader: Option<Arc<FillShader>>,
    /// It's the box's fill: its background, with no layer, nothing past the
    /// box. Its shader is the view's first.
    pub fill: bool,
    /// Its sixteen values.
    pub params: [[f32; 4]; 4],
    /// How far past the box it paints (`Scene::set_layer_extent`).
    pub bleed: f32,
    /// How far past that `zgx_backdrop` reads.
    pub reach: f32,
    /// A backdrop blur under it (`Styled::blur`), for what reads it blurred.
    pub frost: f32,
    /// Whether its time steps.
    pub running: bool,
}
impl ShaderDrive {
    /// Put it on `node` in `scene`, its time `time`.
    pub fn apply(&self, scene: &mut Scene, node: NodeId, time: f32) {
        let uniforms = ShaderUniforms {
            time,
            params: self.params,
        };
        if self.fill {
            scene.set_shader_uniforms(node, uniforms);
            return;
        }
        if self.frost > 0. || scene.effects(node).blur_radius > 0. {
            let mut effects = scene.effects(node);
            effects.blur_radius = match self.shader {
                Some(_) => self.frost,
                None => 0.,
            };
            scene.set_effects(node, effects);
        }
        scene.set_layer_shader(node, self.shader.clone());
        scene.set_layer_extent(node, self.bleed, self.reach);
        scene.set_layer_uniforms(node, uniforms);
    }
}

/// A driven shader's clock.
#[derive(Default)]
struct Clock {
    /// Seconds played before `started`.
    elapsed: f64,
    started: Option<Instant>,
    task: Option<TaskToken>,
}
impl Clock {
    fn at(&mut self, now: Instant) -> f32 {
        let started = *self.started.get_or_insert(now);
        (self.elapsed + now.saturating_duration_since(started).as_secs_f64()) as f32
    }
    fn pause(&mut self, now: Instant) {
        if let Some(started) = self.started.take() {
            self.elapsed += now.saturating_duration_since(started).as_secs_f64();
        }
        self.task = None;
    }
}

/// Keep `view`'s shader as `make` says, reading it again when what it reads
/// changes: see `View::shader_drive`.
pub fn attach(
    cx: &mut Context,
    view: View,
    mut make: impl FnMut() -> ShaderDrive + 'static,
) -> View {
    let runner = cx.try_service::<TaskRunner>();
    let frames = cx.try_service::<FrameClock>();
    let first = make();
    let view = match (&first.shader, first.fill) {
        // Its uniforms come from the scene (`ShaderDrive::apply`): none in the
        // style, whose passes would put its time back.
        (Some(shader), true) => view.bg_shader(shader.clone()),
        // A backdrop blur all have, before what reads it.
        (Some(_), false) if first.frost > 0. => view.blur(first.frost),
        _ => view,
    };
    view.on_mount(move |handle| {
        let weak = handle.ui_handle().clone();
        if let Some(mut ui) = weak.upgrade() {
            mount(&mut ui, weak, handle.node(), make, runner, frames);
        }
    })
}

fn mount(
    ui: &mut Ui,
    weak: WeakUi,
    node: NodeId,
    mut make: impl FnMut() -> ShaderDrive + 'static,
    runner: Option<Rc<TaskRunner>>,
    frames: Option<Rc<FrameClock>>,
) {
    let visible = ui.observe_visibility(node);
    let presented = ui.observe_presentation();
    let themes = crate::theme::Themes::of(&ui.runtime);
    let clock: Rc<RefCell<Clock>> = Default::default();
    ui.retain(node, clock.clone());
    ui.bind(node, move || {
        let drive = make();
        let reduced = themes.system_preferences().reduced_motion;
        let shows = presented.get() && visible.get();
        let now = crate::clock::now();
        let Some(ui) = weak.upgrade() else { return };
        if let Ok(mut scene) = ui.scene.try_borrow_mut()
            && scene.contains(node)
        {
            let time = match reduced {
                true => 0.,
                false => clock.borrow_mut().at(now),
            };
            drive.apply(&mut scene, node, time);
        }
        let wanted = shows && !reduced && drive.running && drive.shader.is_some();
        let (Some(runner), Some(frames), true) = (&runner, &frames, wanted) else {
            clock.borrow_mut().pause(now);
            return;
        };
        let rate = drive.shader.as_deref().and_then(FillShader::fps);
        let task = (runner.0)(Box::pin(step(
            weak.clone(),
            node,
            drive.fill,
            rate,
            clock.clone(),
            FrameClock::clone(frames),
        )));
        clock.borrow_mut().task = Some(task);
    });
}

/// Step `node`'s shader's time at the window's frames, leaving what else its
/// uniforms hold (an animation may be writing them).
async fn step(
    weak: WeakUi,
    node: NodeId,
    fill: bool,
    rate: Option<f32>,
    clock: Rc<RefCell<Clock>>,
    frames: FrameClock,
) {
    loop {
        let next = frames.next();
        let frame: Frame = match rate {
            Some(rate) => next.max_rate(f64::from(rate)).await,
            None => next.await,
        };
        let Some(ui) = weak.upgrade() else { return };
        let mut scene = ui.scene.borrow_mut();
        if !scene.contains(node) {
            return;
        }
        let time = clock.borrow_mut().at(frame.time);
        if fill {
            let mut uniforms = scene.shader_uniforms(node);
            uniforms.time = time;
            scene.set_shader_uniforms(node, uniforms);
        } else if let Some(mut uniforms) = scene.layer_shader(node).map(|layer| layer.uniforms) {
            uniforms.time = time;
            scene.set_layer_uniforms(node, uniforms);
        }
    }
}
