//! Minimal presentation reference driven by the actual display frame clock.
//! A timer-driven heartbeat adds timer jitter to the supposed system floor.
use std::path::PathBuf;
use zgx_desktop::{Application, Titlebar, WindowOptions};
use zgx_script::{ScriptHost, Value, inspect::Session};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let first = args.next().ok_or("expected --inspect APP")?;
    let directory = PathBuf::from(if first == "--inspect" {
        args.next().ok_or("expected app directory")?
    } else {
        first
    });
    let config = Value::from_json(&std::fs::read_to_string(directory.join("zgx.json"))?)
        .ok_or("invalid window config")?;
    let number = |name| config.field(name).and_then(Value::as_f64).unwrap_or(720.);
    Application::new()
        .window(WindowOptions {
            title: "zgx idle presentation reference".into(),
            width: number("width"),
            height: number("height"),
            min_size: Some((1., 1.)),
            titlebar: Titlebar::Hidden,
            ..Default::default()
        })
        .run(move |cx| {
            zgx_script::inspect::prepare();
            let host = ScriptHost::new(&cx.ui.runtime, directory.join("app.luau"))
                .args(["idle".to_owned()]);
            let beat = host.shared("reference_beat", false);
            let _ = host.reload();
            let handle = cx.render(host.view());
            let session = Session::new(&cx.ui, handle, &host, Some(cx.inspector.clone()));
            let tasks = cx.tasks.clone();
            let server =
                zgx_script::inspect::serve(session, "idle display-clock reference", move |task| {
                    tasks.spawn(task);
                })
                .expect("reference inspect socket");
            let source = cx.frames.clone();
            cx.tasks.spawn(async move {
                let _server = server;
                let mut value = false;
                loop {
                    source.next().await;
                    value = !value;
                    beat.set(Value::Bool(value));
                }
            });
        })
}
