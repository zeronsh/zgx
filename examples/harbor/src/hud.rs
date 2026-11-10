//! The interface over the view: frame stats top left, the settings panel
//! top right. Every control writes a signal of `Settings` and nothing
//! else; every readout reads a signal.
use crate::settings::Settings;
use zgx::{
    compose::{View, prelude::*},
    reactive::Signal,
    style::{Styled, Styles, rgb, rgba},
};
use zgx_3d::ToneMapping;

/// What the HUD reports, set by whoever drives the frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// Frames a second, over the last second (0 before one has passed).
    pub fps: f32,
    /// The last frame's interval, ms (with `fps` 0: the fixed step a
    /// bench takes along the path).
    pub frame_ms: f32,
    /// Seconds into the fly-through.
    pub time: f32,
    /// Objects in the world.
    pub objects: usize,
    /// Triangles in its meshes.
    pub triangles: usize,
    /// Containers.
    pub containers: usize,
    /// Point lights, and how many a frame shades.
    pub lights: (usize, usize),
    /// The world's revision (it changes when the world does).
    pub revision: u64,
}

fn panel() -> View {
    column()
        .absolute()
        .p(12.)
        .gap(6.)
        .rounded(10.)
        .bg(rgba(0x0b0e13c8))
        .text_color(rgb(0xe6edf5))
        .text_size(12.)
}

fn label(text_: &str) -> View {
    text(text_).text_color(rgb(0x9fb0c2)).text_size(11.)
}

/// `viewport` (the world's GPU view, its input already handled) filling
/// the box, with the HUD over it.
pub fn view(viewport: View, settings: &Settings, stats: Signal<Stats>) -> View {
    let s = settings.clone();
    let shown = s.hud.clone();
    let hidden = move || {
        if shown.get() {
            Styles::new()
        } else {
            Styles::new().hidden()
        }
    };
    let line = |f: fn(&Stats) -> String| {
        let stats = stats.clone();
        text_signal(move || stats.with(f))
    };
    let readout = panel()
        .top(12.)
        .left(12.)
        .w(220.)
        .reactive_style(hidden.clone())
        .child(text("Harbor at dawn").text_size(14.))
        .child(line(|s| {
            if s.fps > 0. {
                format!("{:.0} fps  {:.2} ms", s.fps, s.frame_ms)
            } else {
                // Offscreen: no display times the frames.
                format!("fixed step  {:.2} s of path", s.frame_ms / 1e3)
            }
        }))
        .child(line(|s| {
            format!("t {:.2} s  revision {}", s.time, s.revision)
        }))
        .child(line(|s| {
            format!(
                "{} objects  {:.2} M triangles",
                s.objects,
                s.triangles as f64 / 1e6
            )
        }))
        .child(line(|s| format!("{} containers", s.containers)))
        .child(line(|s| {
            format!("{} lamps, {} shaded a frame", s.lights.0, s.lights.1)
        }));
    let value = |signal: &Signal<f32>, f: fn(f32) -> String| {
        let signal = signal.clone();
        text_signal(move || f(signal.get()))
    };
    let tone = s.tone_mapping.clone();
    let controls = panel()
        .top(12.)
        .right(12.)
        .w(240.)
        .reactive_style(hidden)
        .child(text("Settings").text_size(14.))
        .child(checkbox("Fly-through", s.fly.clone()))
        .child(label("Sun elevation"))
        .child(
            row()
                .gap(8.)
                .items_center()
                .child(slider("Sun elevation", s.sun_elevation.clone(), 1. ..=40.).grow())
                .child(value(&s.sun_elevation, |v| format!("{v:.0}°"))),
        )
        .child(label("Exposure"))
        .child(
            row()
                .gap(8.)
                .items_center()
                .child(slider("Exposure", s.exposure.clone(), -3. ..=3.).grow())
                .child(value(&s.exposure, |v| format!("{v:+.1}"))),
        )
        .child(label("Tone mapping"))
        .child(
            row()
                .gap(10.)
                .child(radio("ACES", tone.clone(), ToneMapping::Aces))
                .child(radio("Neutral", tone.clone(), ToneMapping::Neutral))
                .child(radio("Linear", tone, ToneMapping::Linear)),
        )
        .child(checkbox("Sun shadows", s.shadows.clone()))
        .child(checkbox("4x multisampling", s.msaa.clone()))
        .child(checkbox("Lamps", s.lamps.clone()))
        .child(label("Yard filled"))
        .child(
            row()
                .gap(8.)
                .items_center()
                .child(slider("Yard filled", s.yard.clone(), 0. ..=1.).grow())
                .child(value(&s.yard, |v| format!("{:.0}%", v * 100.))),
        );
    // The HUD's own switch stays visible.
    let toggle = div()
        .absolute()
        .bottom(12.)
        .right(12.)
        .p(8.)
        .rounded(8.)
        .bg(rgba(0x0b0e1399))
        .text_color(rgb(0xe6edf5))
        .text_size(12.)
        .child(checkbox("HUD", s.hud.clone()));
    div()
        .relative()
        .w_full()
        .h_full()
        .bg(rgb(0x0b0e13))
        .child(viewport.w_full().h_full())
        .child(readout)
        .child(controls)
        .child(toggle)
}
