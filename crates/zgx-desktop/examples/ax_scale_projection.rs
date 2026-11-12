//! Native live-DPI text geometry, for scripts/ax_scale_projection_smoke.py.
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx_desktop::{Application, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smoke = std::env::args().any(|arg| arg == "--smoke-test");
    Application::new()
        .window(WindowOptions {
            title: "AX live scale probe".into(),
            width: 500.,
            height: 500.,
            ..Default::default()
        })
        .run(move |cx| {
            let value = cx.ui.signal(String::from("Ada"));
            cx.render(
                column()
                    .w(200.)
                    .p(10.)
                    .gap(8.)
                    .text_size(16.)
                    // Fixed logical boxes: native character geometry must change
                    // even when a live DPI change does not reflow the text.
                    .child(text("Scale target").w(180.).h(36.))
                    .child(text_input("Scale field", value).w(180.).h(40.)),
            );
            if smoke {
                let window = cx.window.clone();
                cx.tasks.spawn(async move {
                    zgx::timer::sleep(Duration::from_secs(45)).await;
                    window.close();
                });
            }
        })
}
