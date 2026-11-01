//! A window of fixed size that stays resizable to the system: its minimum
//! and maximum the same (`WindowOptions::min_size`, `max_size`). The person
//! can't resize it, and on macOS its size and position still read as
//! settable, so window managers (Raycast, Rectangle) can move it, which
//! they refuse for a window made with `resizable: false`. Prints `READY`
//! and quits after `ZGX_QUIT_AFTER_MS` (none: stays), for
//! `scripts/macos_ax_windows_probe.py`.
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx::scene::Color;
use zgx_desktop::{Application, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let quit_after: Option<u64> = std::env::var("ZGX_QUIT_AFTER_MS")
        .ok()
        .and_then(|ms| ms.parse().ok());
    Application::new()
        .window(WindowOptions {
            title: "zgx fixed size".into(),
            width: 420.,
            height: 300.,
            min_size: Some((420., 300.)),
            max_size: Some((420., 300.)),
            ..Default::default()
        })
        .run(move |cx| {
            cx.render(
                column().p(24.).bg(Color(244, 246, 250, 255)).child(
                    text("420 × 300, resizable to the system, fixed to the person")
                        .text_size(16.)
                        .text_color(Color(30, 30, 40, 255)),
                ),
            );
            if let Some(ms) = quit_after {
                let windows = cx.windows.clone();
                cx.tasks.spawn(async move {
                    zgx::timer::sleep(Duration::from_millis(ms)).await;
                    windows.quit();
                });
            }
            println!("READY");
        })
}
