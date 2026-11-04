//! A `Titlebar::Inset` window for `scripts/windows_caption_smoke.py`: its
//! caption buttons at a zoom, in an app of zgx's light or dark theme, on the
//! theme's background. Prints where `WindowInfo::controls` reports them, in the
//! content's points, then stays open until `--seconds` pass or it is closed.
//!
//! `--theme light|dark`, `--zoom Z`, `--fixed` (not resizable),
//! `--fixed-limits` (resizable, its smallest and largest sizes the same),
//! `--caption-height H`, `--seconds S`.
use std::time::Duration;
use zgx::{
    compose::prelude::*,
    theme::{Appearance, DARK, LIGHT, tok},
};
use zgx_desktop::{Application, Titlebar, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    let light = value("--theme").as_deref() == Some("light");
    let zoom: f64 = value("--zoom").map_or(Ok(1.), |z| z.parse())?;
    let seconds: f64 = value("--seconds").map_or(Ok(30.), |s| s.parse())?;
    let caption_height = value("--caption-height").map(|h| h.parse()).transpose()?;
    let limits = has("--fixed-limits").then_some((900., 500.));
    Application::new()
        .window(WindowOptions {
            title: "Caption probe".into(),
            width: 900.,
            height: 500.,
            titlebar: Titlebar::Inset,
            resizable: !has("--fixed"),
            min_size: limits,
            max_size: limits,
            caption_height,
            ..Default::default()
        })
        .run(move |cx| {
            cx.ui.set_zoom(zoom);
            cx.window.set_theme(Some(if light {
                Appearance::Light
            } else {
                Appearance::Dark
            }));
            // The buttons' ink is the app's theme's, whatever the system's.
            cx.ui
                .theme()
                .set(if light { LIGHT } else { DARK })
                .expect("a built-in theme");
            cx.render(div().w_full().h_full().bg(tok::BACKGROUND));
            let controls = cx.window_info.controls.clone();
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                zgx::timer::sleep(Duration::from_millis(1500)).await;
                match controls.get() {
                    Some(r) => println!("CONTROLS {} {} {} {}", r.x, r.y, r.width, r.height),
                    None => println!("NO_CONTROLS"),
                }
                println!("READY");
                zgx::timer::sleep(Duration::from_secs_f64(seconds)).await;
                window.close();
            });
        })?;
    Ok(())
}
