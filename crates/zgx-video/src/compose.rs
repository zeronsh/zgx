//! Compositions in Rust: what `require("@video")` gives a script, for views.
//!
//! ```no_run
//! use zgx::compose::prelude::*;
//! use zgx_video::compose::{RustComposition, sequence};
//!
//! let composition = RustComposition { width: 1280., height: 720., fps: 30., duration: 90 };
//! let options = zgx_video::RenderOptions { output: "out.mp4".into(), ..Default::default() };
//! zgx_video::compose::render_view(&composition, &options, |_clock| {
//!     div().size(1280., 720.).child(sequence(10., None, |local| {
//!         text("Hello").reactive_style(move || {
//!             Styles::new().opacity(local.map(&[0., 20.], &[0., 1.], Default::default()) as f32)
//!         })
//!     }))
//! }, |_| true).unwrap();
//! ```
use crate::film::{Film, Player, Ticks};
use crate::render::{self, Progress, RenderOptions, Report};
use crate::stage::{Stage, StageOptions};
use std::rc::Rc;
use zgx::compose::{View, component, provide};
use zgx::reactive::Signal;
use zgx::scene::Color;

/// A video in Rust: its measure.
#[derive(Clone, Copy, Debug)]
pub struct RustComposition {
    /// Width, in points.
    pub width: f32,
    /// Height, in points.
    pub height: f32,
    /// Frames per second.
    pub fps: f64,
    /// How many frames it has.
    pub duration: u64,
}

/// An easing, from 0 to 1.
pub type Ease = fn(f64) -> f64;
/// Common easings, as `video.eases` has them.
pub mod eases {
    /// As it goes.
    pub fn linear(x: f64) -> f64 {
        x
    }
    /// Starts slowly.
    pub fn in_quad(x: f64) -> f64 {
        x * x
    }
    /// Ends slowly.
    pub fn out_quad(x: f64) -> f64 {
        1. - (1. - x) * (1. - x)
    }
    /// Starts slowly.
    pub fn in_cubic(x: f64) -> f64 {
        x * x * x
    }
    /// Ends slowly.
    pub fn out_cubic(x: f64) -> f64 {
        1. - (1. - x).powi(3)
    }
    /// Starts and ends slowly.
    pub fn in_out_cubic(x: f64) -> f64 {
        if x < 0.5 {
            4. * x * x * x
        } else {
            1. - (2. - 2. * x).powi(3) / 2.
        }
    }
    /// Ends very slowly.
    pub fn out_expo(x: f64) -> f64 {
        if x >= 1. {
            1.
        } else {
            1. - 2f64.powf(-10. * x)
        }
    }
    /// Passes its end and comes back.
    pub fn out_back(x: f64) -> f64 {
        let y = x - 1.;
        1. + 2.70158 * y * y * y + 1.70158 * y * y
    }
}

/// How [`interpolate`] goes between its points.
#[derive(Clone, Copy, Debug)]
pub struct Interpolate {
    /// Each piece's easing.
    pub ease: Ease,
    /// Hold at the ends rather than going on as the end's piece does.
    pub clamp: bool,
}
impl Default for Interpolate {
    fn default() -> Self {
        Self {
            ease: eases::linear,
            clamp: true,
        }
    }
}

/// `x` mapped from `input` (rising) to `output`, a piece at a time.
///
/// # Panics
/// Unless `input` and `output` have the same length, two or more.
pub fn interpolate(x: f64, input: &[f64], output: &[f64], options: Interpolate) -> f64 {
    assert!(
        input.len() == output.len() && input.len() >= 2,
        "interpolate takes lists of the same length, two or more"
    );
    let last = input.len() - 1;
    if options.clamp {
        if x <= input[0] {
            return output[0];
        }
        if x >= input[last] {
            return output[last];
        }
    }
    let piece = input[..last]
        .iter()
        .rposition(|at| x >= *at)
        .unwrap_or(0)
        .min(last - 1);
    let mut t = (x - input[piece]) / (input[piece + 1] - input[piece]);
    if (0. ..=1.).contains(&t) {
        t = (options.ease)(t);
    }
    output[piece] + (output[piece + 1] - output[piece]) * t
}

/// A damped spring, as a function of the frame.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    /// Where it starts.
    pub from: f64,
    /// Where it goes.
    pub to: f64,
    /// How hard it pulls.
    pub stiffness: f64,
    /// How much it slows.
    pub damping: f64,
    /// How heavy it is.
    pub mass: f64,
    /// Its speed at the start, in spans a second.
    pub velocity: f64,
    /// Frames before it starts.
    pub delay: f64,
    /// `false`: it holds at its end instead of passing it.
    pub overshoot: bool,
}
impl Default for Spring {
    fn default() -> Self {
        Self {
            from: 0.,
            to: 1.,
            stiffness: 100.,
            damping: 10.,
            mass: 1.,
            velocity: 0.,
            delay: 0.,
            overshoot: true,
        }
    }
}
/// Where `spring` is at `frame` of a `fps` video, in closed form: any frame on
/// its own, whichever came before.
pub fn spring(frame: f64, fps: f64, spring: &Spring) -> f64 {
    let t = (frame - spring.delay) / fps;
    let mut p = if t <= 0. {
        0.
    } else {
        let w = (spring.stiffness / spring.mass).sqrt();
        let z = spring.damping / (2. * (spring.stiffness * spring.mass).sqrt());
        let v = -spring.velocity;
        if z < 1. {
            let d = w * (1. - z * z).sqrt();
            let e = (-z * w * t).exp();
            1. - e * ((d * t).cos() + ((z * w - v) / d) * (d * t).sin())
        } else if z == 1. {
            1. - (-w * t).exp() * (1. + (w - v) * t)
        } else {
            let d = w * (z * z - 1.).sqrt();
            let (r1, r2) = (-z * w + d, -z * w - d);
            let a = (v - r2) / (r1 - r2);
            1. - (a * (r1 * t).exp() + (1. - a) * (r2 * t).exp())
        }
    };
    if !spring.overshoot {
        p = p.min(1.);
    }
    spring.from + (spring.to - spring.from) * p
}

/// The frame, from the start of a sequence: `get` read in a binding follows it.
#[derive(Clone)]
pub struct Clock {
    frame: Signal<f64>,
    /// Where its time starts, in the composition's frames.
    pub offset: f64,
    /// How long it lasts, in frames.
    pub duration: f64,
    /// Frames per second.
    pub fps: f64,
}
impl Clock {
    /// The frame, followed where it's read.
    pub fn get(&self) -> f64 {
        self.frame.get() - self.offset
    }
    /// The time since its start, in seconds, followed where it's read.
    pub fn seconds(&self) -> f64 {
        self.get() / self.fps
    }
    /// From 0 at its start to 1 at its end, followed where it's read.
    pub fn progress(&self) -> f64 {
        (self.get() / (self.duration - 1.).max(1.)).clamp(0., 1.)
    }
    /// [`interpolate`] of the frame, followed where it's read.
    pub fn map(&self, input: &[f64], output: &[f64], options: Interpolate) -> f64 {
        interpolate(self.get(), input, output, options)
    }
    /// [`spring`] at the frame, followed where it's read.
    pub fn spring(&self, config: &Spring) -> f64 {
        spring(self.get(), self.fps, config)
    }
}

/// The clock of the sequence `cx` is built in.
pub fn clock_of(cx: &zgx::compose::Context) -> Clock {
    (*cx.service::<Clock>()).clone()
}

/// Shows what `build` makes only from frame `from` for `duration` frames (on
/// to the end, if `None`), its time starting then. What starts as it mounts
/// starts then.
pub fn sequence(
    from: f64,
    duration: Option<f64>,
    build: impl FnMut(Clock) -> View + 'static,
) -> View {
    component(move |cx| {
        let parent = clock_of(cx);
        let clock = Clock {
            frame: parent.frame.clone(),
            offset: parent.offset + from,
            duration: duration.unwrap_or(f64::INFINITY),
            fps: parent.fps,
        };
        let shown = clock.clone();
        let mut build = build;
        zgx::compose::div().child_when(
            move || {
                let at = shown.get();
                (at >= 0. && at < shown.duration).then_some(())
            },
            move |(), _| provide(clock.clone(), build(clock.clone())),
        )
    })
}

/// `composition`'s stage with `build`'s view, as [`crate::render()`] plays it.
pub fn film(
    composition: &RustComposition,
    options: &RenderOptions,
    background: Color,
    build: impl FnOnce(Clock) -> View + 'static,
) -> Result<Film, String> {
    let ticks = Ticks::of(options.timing(), composition.fps);
    let mut setter = None;
    let fps = composition.fps;
    let duration = composition.duration;
    let stage = Stage::new(
        StageOptions {
            width: composition.width,
            height: composition.height,
            fps: ticks.stage_fps(),
            scale: options.scale,
            background,
            ..Default::default()
        },
        |setting| {
            let frame = setting.runtime.signal(0f64);
            setter = Some(frame.clone());
            let clock = Clock {
                frame,
                offset: 0.,
                duration: duration as f64,
                fps,
            };
            Ok(provide(clock.clone(), component(move |_| build(clock))))
        },
    )?;
    let frame: Rc<Signal<f64>> = Rc::new(setter.expect("built with the stage"));
    Film::new(
        stage,
        ticks,
        fps,
        duration,
        Player {
            set_frame: Box::new(move |at| {
                frame.set(at);
            }),
            check: Box::new(|| Ok(())),
            holds: Box::new(Vec::new),
            after_step: Box::new(|| Ok(false)),
        },
    )
}

/// Render `build`'s view to `options.output`, as [`crate::render()`] does a script.
pub fn render_view(
    composition: &RustComposition,
    options: &RenderOptions,
    build: impl FnOnce(Clock) -> View + 'static,
    progress: impl FnMut(Progress) -> bool,
) -> Result<Report, String> {
    render::render_film(options, progress, |background| {
        film(composition, options, background, build)
    })
    .map(|(report, _)| report)
}
