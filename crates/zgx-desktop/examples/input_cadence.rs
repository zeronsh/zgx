//! A dashboard typed into and clicked fast, in-app (not the person's pointer
//! or keys): text into a focused field, presses on a counter and on toggles,
//! `ZGX_INPUT_HZ` times a second (240 by default), for
//! `scripts/input_cadence_probe.py`, which counts the frames handed over
//! (`ZGX_PRESENT_LOG=1`). `ZGX_STREAM_HZ` instead changes a label on the
//! app's own timer, with no input (a stream that must not alternate between
//! refreshes); both 0, nothing changes (a window at rest). `ZGX_SECONDS`
//! (4 by default) ends it.
use std::time::Duration;
use zgx::{compose::prelude::*, compose::row as row_view, input::InputEvent, timer::sleep};
use zgx_desktop::{Application, WindowOptions};

const TOGGLES: usize = 6;

fn rate(name: &str, default: f64) -> f64 {
    zgx::env::var_os(name)
        .and_then(|v| v.to_str()?.parse().ok())
        .unwrap_or(default)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_hz = rate("ZGX_INPUT_HZ", 240.);
    let stream_hz = rate("ZGX_STREAM_HZ", 0.);
    let seconds = rate("ZGX_SECONDS", 4.);
    let drift = rate("ZGX_INPUT_DRIFT", 0.) != 0.;
    let heavy = rate("ZGX_HEAVY", 0.) as usize;
    Application::new()
        .window(WindowOptions {
            title: "zgx input cadence".into(),
            width: 720.,
            height: 400.,
            ..Default::default()
        })
        .run(move |cx| {
            let value = cx.ui.signal(String::new());
            let clicks = cx.ui.signal(0_u64);
            let ticks = cx.ui.signal(0_u64);
            let toggles: Vec<_> = (0..TOGGLES).map(|_| cx.ui.signal(false)).collect();
            // Pressed where the probe presses: boxes placed absolutely, taking
            // no focus, so typed text stays in the field.
            let pressed = clicks.clone();
            let counter = div()
                .absolute()
                .ml(40.)
                .mt(100.)
                .w(160.)
                .h(36.)
                .bg(rgb(0x2a3a50))
                .on_click(move || {
                    pressed.update(|n| *n += 1);
                })
                .child(text_signal({
                    let clicks = clicks.clone();
                    move || format!("clicked {}", clicks.get())
                }));
            // The field, focused again before each keystroke: a press on the
            // counter or a toggle takes focus from it, and text typed then
            // would change nothing.
            let field: std::rc::Rc<std::cell::RefCell<Option<zgx::compose::NodeHandle>>> =
                Default::default();
            let mut page = div()
                .w_full()
                .h_full()
                .bg(rgb(0x10141c))
                .text_color(rgb(0xe5edf7))
                .child(
                    text_input("Field", value.clone())
                        .absolute()
                        .ml(40.)
                        .mt(40.)
                        .w(400.)
                        .h(32.)
                        .on_mount({
                            let held = field.clone();
                            move |handle| {
                                handle.focus();
                                *held.borrow_mut() = Some(handle);
                            }
                        }),
                )
                .child(counter)
                .child(
                    text_signal({
                        let ticks = ticks.clone();
                        move || format!("stream {}", ticks.get())
                    })
                    .absolute()
                    .ml(240.)
                    .mt(108.),
                );
            for (i, on) in toggles.iter().enumerate() {
                let (flip, shown) = (on.clone(), on.clone());
                page = page.child(
                    div()
                        .absolute()
                        .ml(40. + i as f32 * 60.)
                        .mt(170.)
                        .w(48.)
                        .h(28.)
                        .reactive_style(move || {
                            Styles::new().bg(rgb(if shown.get() { 0x3d8bfd } else { 0x39404d }))
                        })
                        .on_click(move || {
                            flip.update(|on| *on = !*on);
                        }),
                );
            }
            // `ZGX_HEAVY=N`: N rows of a dashboard's table below, each cell
            // following what was typed and pressed, so every input lays out
            // and paints all of them again (frames of several milliseconds,
            // some running past their refresh).
            if heavy > 0 {
                let mut table = column().absolute().ml(40.).mt(220.).w(640.).gap(2.);
                for row in 0..heavy {
                    let (value, clicks) = (value.clone(), clicks.clone());
                    let mut line = row_view().h(14.).gap(6.);
                    for cell in 0..6 {
                        let (value, clicks) = (value.clone(), clicks.clone());
                        line = line.child(
                            text_signal(move || {
                                let n = value.with(|v| v.len()) as u64 + clicks.get();
                                format!("{:>5} {:x}", (n + row as u64) * (cell + 1), n ^ row as u64)
                            })
                            .w(96.)
                            .text_size(11.),
                        );
                    }
                    let _ = (value, clicks);
                    table = table.child(line);
                }
                page = page.child(table);
            }
            cx.render(page);
            let window = cx.window.clone();
            let ui = cx.ui.downgrade();
            cx.tasks.spawn(async move {
                // Mounted, focused and shown first.
                sleep(Duration::from_millis(500)).await;
                let start = std::time::Instant::now();
                let mut tick: u64 = 0;
                while start.elapsed().as_secs_f64() < seconds {
                    if input_hz > 0. {
                        let Some(mut ui) = ui.upgrade() else { return };
                        zgx_gpu::present_log::log(|| format!("input {tick}"));
                        match tick % 4 {
                            0 | 2 => {
                                if value.with(|v| v.len() > 48) {
                                    value.set(String::new());
                                }
                                if let Some(field) = field.borrow().as_ref() {
                                    field.focus();
                                }
                                ui.dispatch(InputEvent::Text("a".into()));
                            }
                            1 => press(&mut ui, 120., 118.),
                            _ => press(
                                &mut ui,
                                64. + (tick / 4 % TOGGLES as u64) as f32 * 60.,
                                184.,
                            ),
                        }
                        drop(ui);
                        tick += 1;
                        // On a fixed schedule, as a device's events come: ones
                        // due while the window was busy go at once, as queued
                        // events would.
                        // (`ZGX_INPUT_DRIFT=1`: each wait from the last dispatch,
                        // a timer sharing the window's turns.)
                        let due = match drift {
                            true => {
                                std::time::Instant::now() + Duration::from_secs_f64(1. / input_hz)
                            }
                            false => start + Duration::from_secs_f64(tick as f64 / input_hz),
                        };
                        if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
                            sleep(wait).await;
                        }
                    } else if stream_hz > 0. {
                        ticks.update(|n| *n += 1);
                        sleep(Duration::from_secs_f64(1. / stream_hz)).await;
                    } else {
                        sleep(Duration::from_millis(100)).await;
                    }
                }
                eprintln!("input_cadence ticks={tick} clicks={}", clicks.get());
                window.close();
            });
        })
}

/// A primary press and release at `x, y`.
fn press(ui: &mut zgx::widgets::Ui, x: f32, y: f32) {
    use zgx::input::PointerButton::Primary;
    ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: Primary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x,
        y,
        button: Primary,
    });
}
