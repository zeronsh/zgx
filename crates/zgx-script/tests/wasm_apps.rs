//! WebAssembly apps (`ui.app(id, { wasm })`, `ui.surface`): run in this
//! process on a thread of their own, sandboxed, hearing their element's
//! input and drawing with the host's GPU. The guests are
//! `crates/zgx-wasm/guest/*` (built by its `build.sh` into `built/`).
#![cfg(feature = "wasm")]
mod common;
use common::Harness;
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use zgx::input::{InputEvent, Key, Modifiers, PointerButton};
use zgx_script::{Capability, GuardKind, Plugin, ScriptHost, WasmLaunch, WasmSpec};

fn guest(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../zgx-wasm/guest/built")
        .join(format!("{name}.wasm"))
        .canonicalize()
        .expect("the built guests (crates/zgx-wasm/guest/build.sh)")
}

/// Settle and lay out until `done`, for up to ten seconds of the wall
/// clock: the guest compiles and runs on a thread of its own.
fn until(h: &mut Harness, what: &str, mut done: impl FnMut(&mut Harness) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !done(h) {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out: {what}\n{}",
            h.tree()
        );
        h.settle();
        h.frame();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A press and release at the middle of `id`, as a person's click: it
/// focuses the element.
fn click(h: &mut Harness, id: &str) {
    let b = h.bounds(id);
    let (x, y) = (b.x + b.width / 2., b.y + b.height / 2.);
    h.ui.dispatch(InputEvent::PointerMove { x, y });
    h.ui.input.note_gesture();
    let button = PointerButton::Primary;
    h.ui.dispatch(InputEvent::PointerDown { x, y, button });
    h.ui.dispatch(InputEvent::PointerUp { x, y, button });
}
/// A key pressed by its code (`KeyW`), with the character it types.
fn key(h: &mut Harness, code: &str, typed: Option<&str>) {
    h.ui.input.set_key_code(Some(code.into()));
    h.ui.input.note_gesture();
    let key = match typed {
        Some(text) => Key::Character(text.into()),
        None => Key::Escape,
    };
    h.ui.dispatch(InputEvent::KeyDown {
        key: key.clone(),
        modifiers: Modifiers::default(),
        repeat: false,
    });
    h.ui.dispatch(InputEvent::KeyUp {
        key,
        modifiers: Modifiers::default(),
    });
}

/// A field of the pixels guest's title (`events=3`).
fn field(title: &str, name: &str) -> String {
    title
        .split(' ')
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
        .unwrap_or_default()
        .to_owned()
}

/// A headless renderer of the harness's window, if this machine has a GPU.
fn renderer() -> Option<zgx_gpu::GpuRenderer> {
    match zgx_gpu::GpuRenderer::new(400, 300) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("no GPU here ({error:?}): the drawing half is skipped");
            None
        }
    }
}
fn render(h: &mut Harness, gpu: &mut zgx_gpu::GpuRenderer) {
    h.frame();
    let damage = h.ui.scene.borrow_mut().flush().damage;
    gpu.render(&h.ui.scene.borrow(), &damage).expect("a frame");
}
/// The RGBA pixel at `(x, y)` of what the renderer drew.
fn pixel(gpu: &zgx_gpu::GpuRenderer, x: u32, y: u32) -> [u8; 4] {
    let pixels = gpu.readback().expect("its pixels");
    let at = ((y * 400 + x) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}

const PIXELS: &str = r#"
local width = ui.state("width", 200)
local app = ui.app("pixels", { wasm = "{PIXELS}" })
return ui.column {
    ui.text { id = "status", function() return `{app.status} {app.error}` end },
    ui.text { id = "title", function() return app.title end },
    ui.button { id = "wider", w = 40, h = 20, on_click = function() width:set(260) end, "wider" },
    ui.surface { id = "surface", app = app, w = width, h = 100 },
}
"#;

/// Input reaches the guest after its element is drawn and after it
/// resizes: an external surface's input once died at its first resize,
/// unseen by tests that never drew.
#[test]
fn a_wasm_app_hears_its_input_after_a_draw_and_a_resize() {
    let script = PIXELS.replace("{PIXELS}", &guest("pixels").to_string_lossy());
    let mut h = Harness::inline(&script);
    until(&mut h, "init at the layout's size", |h| {
        field(&h.text("title"), "size") == "200x100"
    });
    assert!(
        h.text("status").starts_with("running"),
        "{}",
        h.text("status")
    );

    let mut gpu = renderer();
    if let Some(gpu) = &mut gpu {
        // Drawn: its frame (the CPU's pixels, uploaded) shows in its box.
        let surface = h.bounds("surface");
        let (x, y) = ((surface.x + 10.) as u32, (surface.y + 10.) as u32);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while pixel(gpu, x, y)[3] != 255 {
            assert!(
                std::time::Instant::now() < deadline,
                "no frame drawn: {:?}",
                pixel(gpu, x, y)
            );
            render(&mut h, gpu);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let [r, g, b, _] = pixel(gpu, x, y);
        assert!(
            b > 120 && r < 120 && g < 120,
            "the guest's gradient: {r} {g} {b}"
        );
    }

    // A new size: the guest hears it (from the painter when drawn).
    h.click("wider");
    let wider = |h: &mut Harness| field(&h.text("title"), "size") == "260x100";
    match &mut gpu {
        Some(gpu) => {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !wider(&mut h) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "no resize: {}",
                    h.text("title")
                );
                render(&mut h, gpu);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        None => until(&mut h, "the new size", wider),
    }

    // And then its input: a press (focusing it), a key and what it types,
    // the wheel.
    click(&mut h, "surface");
    key(&mut h, "KeyW", Some("w"));
    let b = h.bounds("surface");
    h.ui.dispatch(InputEvent::Scroll {
        x: b.x + 5.,
        y: b.y + 5.,
        delta_x: 0.,
        delta_y: 30.,
    });
    until(&mut h, "the input after the resize", |h| {
        let title = h.text("title");
        field(&title, "wheel") == "1"
    });
    let title = h.text("title");
    assert_eq!(field(&title, "keys"), "1", "{title}");
    assert_eq!(field(&title, "text"), "1", "the key typed `w`: {title}");
    assert_eq!(field(&title, "buttons"), "2", "{title}");
    assert_eq!(field(&title, "focused"), "true", "{title}");
    if let Some(gpu) = &mut gpu {
        // Still drawn after all that.
        for _ in 0..3 {
            render(&mut h, gpu);
        }
        let surface = h.bounds("surface");
        assert_eq!(
            pixel(gpu, (surface.x + 250.) as u32, (surface.y + 10.) as u32)[3],
            255
        );
    }
}

const TWO: &str = r#"
local exited = ui.state("exited", "none")
local good = ui.app("good", { wasm = "{GOOD}" })
local bad = ui.app("bad", { wasm = "{BAD}", timeout = 0.3, memory = 32, on_exit = function(code) exited:set(tostring(code)) end })
return ui.column {
    ui.text { id = "bad", function() return `{bad.status}|{bad.error}|{bad.exit_code}|{bad.title}` end },
    ui.text { id = "good", function() return good.title end },
    ui.text { id = "exited", function() return exited:get() end },
    ui.surface { id = "bad-surface", app = bad, w = 120, h = 60 },
    ui.surface { id = "good-surface", app = good, w = 120, h = 60 },
}
"#;
fn two() -> Harness {
    let script = TWO
        .replace("{BAD}", &guest("misbehave").to_string_lossy())
        .replace("{GOOD}", &guest("pixels").to_string_lossy());
    let mut h = Harness::inline(&script);
    until(&mut h, "both running", |h| {
        h.text("bad").starts_with("running") && h.text("good").starts_with("size=")
    });
    h
}
/// The bad guest is pushed to `code`; the good one still hears its input.
fn goes_wrong(code: &str) -> (Harness, String) {
    let mut h = two();
    click(&mut h, "bad-surface");
    key(&mut h, code, None);
    until(&mut h, "the bad session's end", |h| {
        let bad = h.text("bad");
        bad.starts_with("failed") || bad.starts_with("exited")
    });
    let bad = h.text("bad");
    assert_eq!(h.error(), None, "the app is fine");
    click(&mut h, "good-surface");
    key(&mut h, "KeyG", Some("g"));
    until(&mut h, "the good one's input", |h| {
        field(&h.text("good"), "keys") == "1"
    });
    (h, bad)
}

#[test]
fn a_trap_ends_its_session_alone() {
    let (_, bad) = goes_wrong("KeyT");
    assert!(bad.starts_with("failed|`event` trapped"), "{bad}");
    assert!(
        bad.contains("a trap on purpose"),
        "the panic's message: {bad}"
    );
}

#[test]
fn a_guest_looping_forever_is_interrupted() {
    let started = std::time::Instant::now();
    let (_, bad) = goes_wrong("KeyL");
    assert!(
        bad.starts_with("failed|`event` ran longer than 0.3 s"),
        "{bad}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(8));
}

#[test]
fn the_memory_limit_holds() {
    let (_, bad) = goes_wrong("KeyM");
    assert!(bad.starts_with("failed|`event` trapped"), "{bad}");
    assert!(bad.contains("memory"), "{bad}");
}

#[test]
fn an_exit_is_the_sessions_exit_code() {
    let (mut h, bad) = goes_wrong("KeyE");
    // WASI 0.2's exit says success or failure: 7 is 1.
    assert!(bad.starts_with("exited|nil|1|"), "{bad}");
    until(&mut h, "on_exit", |h| h.text("exited") == "1");
}

#[test]
fn the_sandbox_sees_no_files() {
    let mut h = two();
    click(&mut h, "bad-surface");
    key(&mut h, "KeyF", None);
    until(&mut h, "its try", |h| h.text("bad").contains("file="));
    let bad = h.text("bad");
    assert!(
        bad.contains("file=Err(") && bad.contains("dir=Err("),
        "{bad}"
    );
    assert!(bad.starts_with("running"), "{bad}");
}

/// A session outlives reloads, as a program's does, and ends as the script
/// closes it.
#[test]
fn a_session_outlives_reloads_and_closes() {
    let script = format!(
        r#"local app = ui.app("p", {{ wasm = "{}" }})
return ui.column {{
    ui.text {{ id = "status", function() return app.status end }},
    ui.text {{ id = "title", function() return app.title end }},
    ui.button {{ id = "close", w = 40, h = 20, on_click = function() app:close() end, "close" }},
    ui.surface {{ id = "s", app = app, w = 100, h = 50 }},
}}"#,
        guest("pixels").display()
    );
    let mut h = Harness::inline(&script);
    until(&mut h, "running", |h| h.text("status") == "running");
    click(&mut h, "s");
    until(&mut h, "its press", |h| {
        field(&h.text("title"), "buttons") == "2"
    });
    h.host.reload().unwrap();
    h.settle();
    h.frame();
    assert_eq!(h.text("status"), "running");
    assert_eq!(
        field(&h.text("title"), "buttons"),
        "2",
        "the same session after a reload"
    );
    h.click("close");
    until(&mut h, "closed", |h| h.text("status") == "exited");
}

/// A plugin runs a WebAssembly app only granted `Capability::Wasm` (or
/// `Apps`), only from its own folder, and only as the host's hook says.
#[test]
fn a_plugin_needs_the_grant_its_folder_and_the_hook() {
    let dir = std::env::temp_dir().join(format!("zgx-wasm-plugin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(guest("pixels"), dir.join("app.wasm")).unwrap();
    let plugin = |name: &str, code: &str, grants: Vec<Capability>| {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::copy(guest("pixels"), folder.join("app.wasm")).unwrap();
        std::fs::write(folder.join("init.luau"), code).unwrap();
        Plugin {
            name: name.into(),
            entry: folder.join("init.luau"),
            grants,
        }
    };
    let run = |what: &str| {
        format!(
            r#"local ok, err = pcall(ui.app, "a", {{ wasm = "{what}" }})
ui.contribute("bar", ui.text {{ id = "{{ID}}", ok and "started" or tostring(err) }})"#
        )
    };
    let asked: Rc<RefCell<Vec<WasmLaunch>>> = Rc::default();
    let heard = asked.clone();
    let outside = guest("pixels").to_string_lossy().into_owned();
    let plugins = vec![
        plugin("none", &run("app.wasm").replace("{ID}", "none"), vec![]),
        plugin(
            "granted",
            &run("app.wasm").replace("{ID}", "granted"),
            vec![Capability::Wasm],
        ),
        plugin(
            "apps",
            &run("app.wasm").replace("{ID}", "apps"),
            vec![Capability::Apps],
        ),
        plugin(
            "outside",
            &run(&outside).replace("{ID}", "outside"),
            vec![Capability::Wasm],
        ),
        plugin(
            "climbing",
            &run("../app.wasm").replace("{ID}", "climbing"),
            vec![Capability::Wasm],
        ),
        plugin(
            "refused",
            &run("app.wasm").replace("{ID}", "refused"),
            vec![Capability::Wasm],
        ),
    ];
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.column { ui.slot("bar", { id = "bar" }) }"#,
        );
        host.vet_wasm(move |launch| {
            heard.borrow_mut().push(launch.clone());
            if launch.plugin.as_deref() == Some("refused") {
                return Err("not this one".into());
            }
            Ok(launch.asked.clone().memory_limit(64 << 20))
        });
        host.set_plugins(plugins);
        host
    });
    assert!(
        h.text("none").contains("Capability::Apps") || h.text("none").contains("grants it"),
        "{}",
        h.text("none")
    );
    assert_eq!(h.text("granted"), "started");
    assert_eq!(h.text("apps"), "started", "Apps allows less than it");
    assert!(
        h.text("outside").contains("not inside the plugin's folder"),
        "{}",
        h.text("outside")
    );
    assert!(
        h.text("climbing")
            .contains("not inside the plugin's folder"),
        "{}",
        h.text("climbing")
    );
    assert!(
        h.text("refused").contains("not this one"),
        "{}",
        h.text("refused")
    );
    let asked = asked.borrow();
    let granted = asked
        .iter()
        .find(|l| l.plugin.as_deref() == Some("granted"))
        .expect("vetted");
    assert_eq!(
        granted.asked.module,
        dir.join("granted/app.wasm").canonicalize().unwrap()
    );
    assert!(!granted.running);
    assert!(
        asked.iter().all(|l| l.plugin.as_deref() != Some("outside")),
        "refused before the hook"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The app's own VM asks its guard, by the module's path.
#[test]
fn the_app_guard_is_asked_by_the_module() {
    let module = guest("pixels");
    let seen: Rc<RefCell<Vec<(GuardKind, String)>>> = Rc::default();
    let heard = seen.clone();
    let script = format!(
        r#"local ok, err = pcall(ui.app, "a", {{ wasm = "{}" }})
return ui.text {{ id = "r", ok and "started" or tostring(err) }}"#,
        module.display()
    );
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", &script);
        host.guard(move |kind, name| {
            heard.borrow_mut().push((kind, name.to_owned()));
            match kind {
                GuardKind::Wasm => Err("no apps in a preview".into()),
                _ => Ok(()),
            }
        });
        host
    });
    h.host.reload().unwrap();
    h.settle();
    assert!(
        h.text("r").contains("no apps in a preview"),
        "{}",
        h.text("r")
    );
    assert!(
        seen.borrow()
            .iter()
            .any(|(kind, name)| *kind == GuardKind::Wasm && *name == module.to_string_lossy())
    );
}

/// The 3D guest draws with WebGPU on the host's own device: its frames
/// show in its box, lit and turning.
#[test]
fn a_gpu_guest_draws_on_the_hosts_device() {
    let Some(mut gpu) = renderer() else { return };
    let script = format!(
        r#"local app = ui.app("cube", {{ wasm = "{}" }})
return ui.column {{
    ui.text {{ id = "title", function() return `{{app.status}} {{app.error}} {{app.title}}` end }},
    ui.surface {{ id = "cube", app = app, w = 320, h = 200 }},
}}"#,
        guest("cube").display()
    );
    let mut h = Harness::inline(&script);
    until(&mut h, "running", |h| {
        h.text("title").starts_with("running")
    });
    let b = h.bounds("cube");
    let (cx, cy) = ((b.x + b.width / 2.) as u32, (b.y + b.height / 2.) as u32);
    let corner = ((b.x + 3.) as u32, (b.y + 3.) as u32);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut seen = std::collections::HashSet::new();
    loop {
        render(&mut h, &mut gpu);
        let centre = pixel(&gpu, cx, cy);
        if centre[3] == 255 {
            seen.insert(centre);
        }
        // Frames that differ (it turns), and a lit cube amid the clear.
        if seen.len() >= 3 && pixel(&gpu, corner.0, corner.1)[3] == 255 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no turning cube: {:?} {}",
            seen,
            h.text("title")
        );
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    let title = h.text("title");
    assert!(title.contains("cube ·"), "{title}");
    assert!(!title.contains("failed"), "{title}");
}

/// What `opts` raises, as text.
fn raised(opts: &str) -> String {
    let mut h = Harness::inline(&format!(
        r#"local ok, err = pcall(ui.app, "x", {opts})
return ui.text {{ id = "error", if ok then "no error" else tostring(err) }}"#
    ));
    h.text("error")
}

#[test]
fn options_are_checked() {
    let module = guest("pixels").display().to_string();
    assert!(
        raised(&format!(r#"{{ wasm = "{module}", command = "sh" }}"#))
            .contains("`command` is a program's")
    );
    assert!(raised(&format!(r#"{{ wasm = "{module}", memory = 0 }}"#)).contains("`memory` is MiB"));
    assert!(
        raised(&format!(r#"{{ wasm = "{module}", timeout = 600 }}"#))
            .contains("`timeout` is seconds")
    );
    assert!(raised(r#"{ command = "sh", memory = 4 }"#).contains("a WebAssembly app's"));
    assert!(raised(r#"{ wasm = "/nonexistent/x.wasm" }"#).contains("/nonexistent/x.wasm"));
    // Not a component: a failed session, with why.
    let mut h = Harness::inline(&format!(
        r#"local app = ui.app("x", {{ wasm = "{}" }})
return ui.text {{ id = "s", function() return `{{app.status}}: {{app.error}}` end }}"#,
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("Cargo.toml")
            .display()
    ));
    until(&mut h, "failed", |h| h.text("s").starts_with("failed"));
    assert!(h.text("s").contains("not a component"), "{}", h.text("s"));
    let _ = WasmSpec::new("x");
}
