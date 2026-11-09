//! An external surface: another program (zgx-surface's `spinner_client`,
//! on wgpu of its own) draws the box's frames into buffers zgx shares with
//! it, and zgx composites them as any image: rounded, with UI over them.
//! Its input is the program's: drag the ring, click to lock the pointer
//! (Escape lets go), C changes the colour, Space stops the spin.
//!
//! Run: cargo build -p zgx-surface --features wgpu29 --example spinner_client
//!      cargo run -p zgx-desktop --features surfaces --example external_surface
//!
//! - `--shm`: the client offers shared memory alone (read back and copied);
//! - `--client PATH`: another client program (default: `spinner_client`
//!   beside this example, or `ZGX_SURFACE_CLIENT`);
//! - `--quit-after SECONDS`: print the frames shown and quit.
use std::{cell::Cell, rc::Rc, time::Duration};
use zgx::{compose::prelude::*, scene::Color};
use zgx_desktop::{
    Application, WindowOptions,
    surface::{App, AppOptions, AppStatus, surface},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let client = arg("--client")
        .or_else(|| std::env::var("ZGX_SURFACE_CLIENT").ok())
        .unwrap_or_else(|| {
            let exe = std::env::current_exe().expect("the example's path");
            exe.with_file_name("spinner_client")
                .to_string_lossy()
                .into_owned()
        });
    let mut client_args = Vec::new();
    if args.iter().any(|a| a == "--shm") {
        client_args.push("--shm".to_owned());
    }
    if let Some(fps) = arg("--fps") {
        client_args.extend(["--fps".to_owned(), fps]);
    }
    let quit_after = arg("--quit-after").and_then(|s| s.parse::<f64>().ok());
    Application::new()
        .window(WindowOptions {
            title: "zgx external surface".into(),
            width: 900.,
            height: 640.,
            ..Default::default()
        })
        .run(move |cx| {
            let app = App::spawn(&cx.ui.runtime, AppOptions::new(&client).args(client_args))
                .unwrap_or_else(|e| App::failed(&cx.ui.runtime, format!("{client}: {e}")));
            let (status, title) = (app.status(), app.title());
            let pid = app.pid();
            let presses = Rc::new(Cell::new(0_u32));
            let label = cx.ui.signal(String::new());
            let shown_label = label.clone();
            cx.render(
                column()
                    .w_full()
                    .h_full()
                    .p(24.)
                    .gap(14.)
                    .bg(rgb(0x0f141b))
                    .text_color(rgb(0xe0eaf5))
                    .child(
                        row()
                            .gap(12.)
                            .items_center()
                            .child(text("Another process draws this box").text_size(20.))
                            .child(text_signal(move || match status.get() {
                                AppStatus::Starting => "starting".into(),
                                AppStatus::Running => format!("running, pid {}", pid.unwrap_or(0)),
                                AppStatus::Exited(code) => format!("exited ({code:?})"),
                                AppStatus::Failed(why) => format!("failed: {why}"),
                            }))
                            .child(text_signal(move || title.get())),
                    )
                    .child(
                        div()
                            .w_full()
                            .grow()
                            .child(
                                surface(&app)
                                    .w_full()
                                    .h_full()
                                    .rounded(18.)
                                    .overflow_hidden()
                                    .bg(Color(20, 26, 34, 255)),
                            )
                            // zgx's own UI over the program's frames.
                            .child(
                                row()
                                    .absolute()
                                    .top(14.)
                                    .left(14.)
                                    .gap(10.)
                                    .items_center()
                                    .px(12.)
                                    .py(8.)
                                    .rounded(10.)
                                    .bg(Color(10, 14, 20, 170))
                                    .child(text("zgx UI over the surface").text_size(13.))
                                    .child(
                                        button()
                                            .px(10.)
                                            .py(4.)
                                            .rounded(6.)
                                            .bg(rgb(0x2b3d52))
                                            .child(text_signal(move || shown_label.get()))
                                            .on_click({
                                                let (presses, label) =
                                                    (presses.clone(), label.clone());
                                                move || {
                                                    presses.set(presses.get() + 1);
                                                    label.set(format!("Pressed {}", presses.get()));
                                                }
                                            }),
                                    ),
                            ),
                    ),
            );
            label.set("Press".into());
            if let Some(seconds) = quit_after {
                let (window, app) = (cx.window.clone(), app.clone());
                cx.tasks.spawn(async move {
                    zgx::timer::sleep(Duration::from_secs_f64(seconds)).await;
                    println!(
                        "SURFACE status={:?} title={:?}",
                        app.status().get(),
                        app.title().get()
                    );
                    app.close();
                    window.close();
                });
            }
        })
}
