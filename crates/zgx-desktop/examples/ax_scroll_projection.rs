//! Nested scroll bounds, text ranges, focus and hit tests over the native
//! accessibility bus, for scripts/ax_scroll_projection_probe.py.
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx_desktop::{Application, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smoke = std::env::args().any(|arg| arg == "--smoke-test");
    Application::new()
        .window(WindowOptions {
            title: "AX scroll projection probe".into(),
            width: 500.,
            height: 500.,
            ..Default::default()
        })
        .run(move |cx| {
            let outer = cx.ui.signal(0_f32);
            let inner = cx.ui.signal(0_f32);
            let outer_step = outer.clone();
            let inner_step = inner.clone();
            let value = cx.ui.signal(String::from("selection stays live"));
            cx.render(
                column()
                    .w_full()
                    .h_full()
                    .p(12.)
                    .gap(8.)
                    .child(button().child("Move outer").on_click(move || {
                        outer_step.update(|v| *v = if *v >= 20. { 0. } else { *v + 4. });
                    }))
                    .child(button().child("Move inner").on_click(move || {
                        inner_step.update(|v| *v = if *v >= 20. { 0. } else { *v + 4. });
                    }))
                    .child(text_input("Projection focus", value).h(40.))
                    .child(
                        scroll(outer).w_full().h(250.).child(
                            column()
                                .w_full()
                                .p(30.)
                                .child(
                                    scroll(inner).w_full().h(120.).child(
                                        column()
                                            .w_full()
                                            .p(30.)
                                            .selectable(true)
                                            .child(text("Projection target").h(28.))
                                            .children(
                                                (0..20).map(|i| {
                                                    text(format!("Nested row {i}")).h(28.)
                                                }),
                                            ),
                                    ),
                                )
                                .child(div().h(500.)),
                        ),
                    ),
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
