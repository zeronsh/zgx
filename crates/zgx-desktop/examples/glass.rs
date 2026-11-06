//! The system's glass with zgx under and over it: on iOS through
//! `scripts/ios/sim.py run --example glass`, elsewhere with a stand-in look.
//! `GLASS_MODE`: pane (default), variants, merge, live.
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx::glass::{GlassStyle, GlassVariant, glass, glass_group};
use zgx::scene::Color;
use zgx_desktop::{Application, WindowOptions};

/// The stripes the native reference app draws, and zgx's alone here.
fn stripes() -> View {
    let mut column = div()
        .absolute()
        .left(0.)
        .top(0.)
        .w_full()
        .h_full()
        .flex_col();
    for k in 0..28 {
        let color = if k % 2 == 0 {
            rgb(0xf22626)
        } else {
            rgb(0x1a40f2)
        };
        column = column.child(div().w_full().flex_grow(1.).bg(color));
    }
    column
}

fn pane(x: f32, y: f32, w: f32, h: f32, style: GlassStyle) -> View {
    glass(move || style).absolute().left(x).top(y).w(w).h(h)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = zgx::env::var("GLASS_MODE").unwrap_or_else(|_| "pane".into());
    Application::new()
        .window(WindowOptions {
            title: "zgx — glass".into(),
            width: 402.,
            height: 874.,
            ..Default::default()
        })
        .run(move |cx| {
            let style = |radius: f32| GlassStyle {
                radius,
                ..Default::default()
            };
            let view = match mode.as_str() {
                "variants" => {
                    let row = |i: f32, variant, tint: Option<Color>| {
                        pane(
                            40.,
                            150. + i * 170.,
                            322.,
                            140.,
                            GlassStyle {
                                variant,
                                tint,
                                radius: 40.,
                                ..Default::default()
                            },
                        )
                    };
                    div()
                        .w_full()
                        .h_full()
                        .child(stripes())
                        .child(row(0., GlassVariant::Regular, None))
                        .child(row(1., GlassVariant::Clear, None))
                        .child(row(
                            2.,
                            GlassVariant::Regular,
                            Some(Color(52, 199, 89, 255)),
                        ))
                        .child(row(3., GlassVariant::Clear, Some(Color(255, 149, 0, 255))))
                }
                "merge" => {
                    let gap: f32 = zgx::env::var("GAP")
                        .ok()
                        .and_then(|gap| gap.parse().ok())
                        .unwrap_or(20.);
                    div().w_full().h_full().child(stripes()).child(
                        glass_group(40.)
                            .absolute()
                            .left(0.)
                            .top(0.)
                            .w_full()
                            .h_full()
                            .child(pane(60., 350., 120., 120., style(50.)))
                            .child(pane(180. + gap, 350., 120., 120., style(50.))),
                    )
                }
                "live" => {
                    // The panes move and change style from a timer, with the
                    // stripes under them moving too.
                    let time = cx.ui.signal(0u32);
                    let tick = time.clone();
                    cx.tasks.spawn(async move {
                        loop {
                            zgx::timer::sleep(Duration::from_millis(16)).await;
                            tick.set(tick.get().wrapping_add(1));
                        }
                    });
                    let at = time.clone();
                    let y = time.clone();
                    div()
                        .w_full()
                        .h_full()
                        .child(
                            stripes().h(1100.).reactive_style(move || {
                                Styles::new().top(-((y.get() % 120) as f32))
                            }),
                        )
                        .child(
                            glass(move || GlassStyle {
                                tint: (at.get() / 120 % 2 == 1).then_some(Color(52, 199, 89, 255)),
                                radius: 40.,
                                ..Default::default()
                            })
                            .absolute()
                            .reactive_style({
                                let t = time.clone();
                                move || {
                                    Styles::new().left(40. + ((t.get() as f32) / 30.).sin() * 30.)
                                }
                            })
                            .top(300.)
                            .w(260.)
                            .h(200.),
                        )
                }
                _ => div()
                    .w_full()
                    .h_full()
                    .child(stripes())
                    .child(pane(40., 300., 322., 240., style(40.)))
                    .child(
                        // On the overlay plane, over the glass: as the
                        // reference's top Metal layer.
                        div()
                            .absolute()
                            .left(131.)
                            .top(350.)
                            .w(140.)
                            .h(140.)
                            .rounded(70.)
                            .bg(rgb(0xffff00)),
                    ),
            };
            cx.render(view);
        })
}
