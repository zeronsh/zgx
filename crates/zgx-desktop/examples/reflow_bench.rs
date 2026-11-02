//! A reflow-heavy scene to measure: a PR-like file list, `REFLOW_FILES`
//! (30) files of `REFLOW_LINES` (50) lines, some 3,000 rows of wrapping text
//! in all, each file a list with a heading and two buttons, in a scroll view.
//! The window is resized `REFLOW_STEPS` (150) times, one every
//! `REFLOW_INTERVAL_MS` (33), then the app quits.
//!
//! Row text wraps, broken again as the width changes; `REFLOW_WRAP=0` keeps
//! it on one line (measured once, served from cache), as the scene was
//! before: layout's own row work alone.
//!
//! `REFLOW_MODE`:
//! - `drag` (the default): 1100 points narrowing by 3 a step, every frame a
//!   width layout hasn't seen, as a person dragging the window's edge.
//! - `alternate`: 700 and 1100 in turn, which after two frames layout
//!   answers from its cached measurements: what a cache saves.
//!
//! With `ZGX_PROFILE=1` each phase's instructions and time are reported on
//! stderr every 120 frames and at the end (build with `--features
//! zgx/profile-nested` for text within layout):
//!
//! ```sh
//! ZGX_PROFILE=1 cargo run --release -p zgx-desktop --example reflow_bench --features zgx/profile-nested
//! ```
//!
//! Prints `READY` once built, `RESIZING <mode>` as the resizes start and
//! `DONE <steps>` before quitting. Used by the accessibility, paint-walk and
//! text re-break work (with an assistive client attached for the first).
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx::scene::Color;
use zgx::semantics::Role;
use zgx_desktop::{Application, WindowOptions};

fn setting(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (files, lines) = (setting("REFLOW_FILES", 30), setting("REFLOW_LINES", 50));
    let (steps, interval) = (
        setting("REFLOW_STEPS", 150),
        setting("REFLOW_INTERVAL_MS", 33),
    );
    let alternate = std::env::var("REFLOW_MODE").is_ok_and(|mode| mode == "alternate");
    let wrap = setting("REFLOW_WRAP", 1) != 0;
    Application::new()
        .window(WindowOptions {
            title: "reflow bench".into(),
            width: 1100.,
            height: 800.,
            ..Default::default()
        })
        .run(move |cx| {
            let file = move |f: u32| {
                let rows = (0..lines).map(move |l| {
                    div()
                        .role(Role::ListItem)
                        .flex_row()
                        .gap(6.)
                        .child(text(format!("{l:>3}")).text_size(12.).w(32.))
                        .child(
                            text(format!(
                                "+ let value_{f}_{l} = compute(input, options).map(|x| x * {l}).unwrap_or_default(); // a long line that wraps as the window narrows"
                            ))
                            .text_size(12.)
                            .text_wrap(wrap)
                            .flex_grow(1.)
                            .flex_shrink(1.)
                            .min_w(0.),
                        )
                });
                div()
                    .role(Role::List)
                    .mt(8.)
                    .p(6.)
                    .bg(Color(250, 250, 252, 255))
                    .child(
                        div()
                            .flex_row()
                            .gap(8.)
                            .child(
                                text(format!("crates/zgx/src/file_{f}.rs"))
                                    .role(Role::Heading)
                                    .text_size(14.),
                            )
                            .child(button().child("Viewed"))
                            .child(button().child("Comment")),
                    )
                    .children(rows)
            };
            let offset = cx.ui.signal(0f32);
            cx.render(
                scroll(offset)
                    .w_full()
                    .h_full()
                    .child(column().p(12.).children((0..files).map(file))),
            );
            let window = cx.window.clone();
            let windows = cx.windows.clone();
            cx.tasks.spawn(async move {
                // Settled first: the window shown and drawn once.
                zgx::timer::sleep(Duration::from_millis(1500)).await;
                println!("RESIZING {}", if alternate { "alternate" } else { "drag" });
                for step in 0..steps {
                    let width = if alternate {
                        if step % 2 == 0 { 700. } else { 1100. }
                    } else {
                        (1100. - 3. * step as f64).max(320.)
                    };
                    window.set_inner_size(width, 800.);
                    zgx::timer::sleep(Duration::from_millis(interval.into())).await;
                }
                zgx::timer::sleep(Duration::from_millis(300)).await;
                zgx::profile::report();
                println!("DONE {steps}");
                windows.quit();
            });
            println!("READY");
        })
}
