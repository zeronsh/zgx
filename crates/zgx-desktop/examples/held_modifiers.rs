//! A modifier pressed alone reaches `Ui::modifiers`, and the window losing
//! focus clears it; run with --smoke-test and held_modifiers_smoke.py, which
//! reads the `MODIFIERS` line printed at each change.
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx_desktop::{Application, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smoke = std::env::args().any(|arg| arg == "--smoke-test");
    Application::new()
        .window(WindowOptions {
            title: "zgx held modifiers".into(),
            width: 360.,
            height: 160.,
            ..Default::default()
        })
        .run(move |cx| {
            let held = cx.ui.modifiers();
            let shown = held.clone();
            cx.render(column().p(20.).child(text_signal(move || {
                let m = shown.get();
                format!("Command held: {}", if m.meta { "yes" } else { "no" })
            })));
            let report = cx.ui.runtime.effect(move || {
                let m = held.get();
                let names: Vec<&str> = [
                    (m.shift, "shift"),
                    (m.control, "control"),
                    (m.alt, "alt"),
                    (m.meta, "meta"),
                ]
                .into_iter()
                .filter_map(|(down, name)| down.then_some(name))
                .collect();
                let names = if names.is_empty() {
                    "none".to_string()
                } else {
                    names.join("+")
                };
                println!("MODIFIERS {names}");
            });
            cx.on_closed(move || drop(report));
            if smoke {
                let window = cx.window.clone();
                cx.tasks.spawn(async move {
                    zgx::timer::sleep(Duration::from_secs(8)).await;
                    window.close();
                });
            }
        })
}
