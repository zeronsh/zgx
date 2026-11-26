//! The example apps' common interactions, each to its frame (and drawn, if
//! there is a drawing): mail (opening a message, a search keystroke,
//! archiving), the dashboard (a feed tick, sorting, a filter keystroke),
//! Sketch (dragging a shape, panning, zooming), the code editor (a
//! keystroke, scrolling a page, a find-all keystroke) and chat (a token of
//! a reply streaming in, a composer keystroke, switching conversations). Each prints its time
//! and instructions retired (macOS) per interaction and, with
//! `ZGX_PROFILE=1`, where they went. `apps_bench` runs them drawn on a GPU
//! with text shaped; CI's instruction gate runs them inside
//! `list_state_bench` (no new binary to link), small and undrawn but for
//! their canvases' CPU work.
//!
//! `APP=mail` (`dashboard`, `sketch`, `code`, `chat`, `stream`) runs one app; `REPS` sets the
//! interactions measured (20); `ROWS` the dashboard's rows (100,000),
//! `SHAPES` Sketch's shapes (500), `LINES` the code editor's file (100,000).
//! `ONLY=name` measures the interactions whose names hold it, `SKIP=name`
//! the others.
use crate::bench_limits;
#[path = "chat_server.rs"]
mod chat_server;
#[path = "gate_marks.rs"]
mod gate_marks;
use std::{
    rc::Rc,
    time::{Duration, Instant},
};
use zgx::{
    input::{InputEvent, Key, PointerButton},
    profile::Phase,
    scene::Rect,
};
use zgx_script::{ScriptHost, testing::TestApp};

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");

/// Where the example apps' scripts are: `ZGX_EXAMPLES` when set (the perf
/// gate runs a base revision's binary on that revision's scripts, not on the
/// head's it was built beside), else this checkout's.
fn examples() -> String {
    std::env::var("ZGX_EXAMPLES").unwrap_or_else(|_| EXAMPLES.to_owned())
}

/// A size from the environment, else `default`.
fn size(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// This process's instructions retired (macOS; 0 elsewhere).
fn instructions() -> u64 {
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct Usage {
            uuid: [u8; 16],
            fields: [u64; 40],
        }
        unsafe extern "C" {
            fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut Usage) -> i32;
        }
        let mut usage = Usage {
            uuid: [0; 16],
            fields: [0; 40],
        };
        // SAFETY: RUSAGE_INFO_V4 into a buffer as large as it; this process.
        unsafe { proc_pid_rusage(std::process::id() as i32, 4, &mut usage) };
        // `ri_instructions`.
        usage.fields[29]
    }
    #[cfg(not(target_os = "macos"))]
    0
}

/// What draws an app's frames and shapes its text: a GPU offscreen
/// (`apps_bench`), or its canvases' CPU work alone (the gate's run, its
/// text laid out by the fallback layout, as the other script benches' is).
pub trait Drawing {
    /// Draw `damage` of `scene`'s frame.
    fn draw(&mut self, scene: &zgx::scene::Scene, damage: &[Rect]);
    /// Wait for what was drawn to be done.
    fn finish(&mut self) {}
}
/// Makes an app's drawing for its scene of `size` (points), if any.
pub type MakeDrawing<'a> =
    &'a dyn Fn(&mut zgx::scene::Scene, (f32, f32)) -> Option<Box<dyn Drawing>>;

/// An app, run headless as a window runs it: a frame laid out, its damage
/// found and drawn (if there is a drawing).
struct Bench {
    app: TestApp,
    drawing: Option<Box<dyn Drawing>>,
    reps: usize,
    /// The area the frames so far damaged, in points squared, where a scene
    /// asks (`stream`): none for the others, whose counts it would change.
    damaged: Option<f64>,
    /// Where the pointer rests, once a scene puts it there (`stream`): each
    /// frame then asks for the cursor under it, as a desktop window does
    /// after drawing (`sync_cursor`: again whenever the point or geometry
    /// moved), and what it last asked at.
    cursor: Option<((f32, f32), Option<u64>)>,
    /// The time a scene's script and fades read (`ui.now()`, `fade_in`),
    /// where a scene asks (`stream`): moved a frame's 1/60 s each frame,
    /// whatever the wall clock says, so what the time decides (a fade's step,
    /// a boost) falls on the same frame in every run, as fast or slow as it
    /// runs (under cachegrind, some fifty times slower). With the frames
    /// moved so far.
    clock: Option<(zgx::clock::VirtualClock, u32)>,
}
impl Bench {
    fn new(script: &str, size: (f32, f32), args: &[String], make: MakeDrawing) -> Self {
        Self::with(script, size, args, make, |_| {})
    }
    /// `new`, with `setup` run on the app before its first frame.
    fn with(
        script: &str,
        size: (f32, f32),
        args: &[String],
        make: MakeDrawing,
        setup: impl FnOnce(&mut TestApp),
    ) -> Self {
        Self::hosted(script, size, args, make, |_| {}, setup)
    }
    /// `with`, the host given to `host` before it loads (a server).
    fn hosted(
        script: &str,
        size: (f32, f32),
        args: &[String],
        make: MakeDrawing,
        host: impl FnOnce(&ScriptHost),
        setup: impl FnOnce(&mut TestApp),
    ) -> Self {
        let path = format!("{}/{script}/app.luau", examples());
        let mut app = TestApp::with_desktop(|runtime| {
            let made = ScriptHost::new(runtime, path)
                .limits(bench_limits::limits())
                .args(args.iter().cloned());
            host(&made);
            made
        });
        app.ui.scene.borrow_mut().resize(size.0, size.1);
        // As a desktop window with no screen reader on: names and values
        // made from state wait for one.
        app.ui.set_assistive_technology(false);
        setup(&mut app);
        let drawing = make(&mut app.ui.scene.borrow_mut(), size);
        let reps = self::size("REPS", 20);
        let mut bench = Self {
            app,
            drawing,
            reps,
            damaged: None,
            cursor: None,
            clock: None,
        };
        bench.frames(3);
        assert_eq!(bench.app.error(), None, "{script} loads");
        bench
    }
    /// A frame: what the app changed, laid out, its damage drawn.
    fn frame(&mut self) {
        if let Self {
            clock: Some((clock, frames)),
            app,
            ..
        } = self
        {
            *frames += 1;
            let elapsed = Duration::from_secs_f64(f64::from(*frames) / 60.);
            clock.advance(elapsed, || app.settle());
        }
        self.app.frame();
        let report = self.app.ui.scene.borrow_mut().flush();
        if let Some(damaged) = &mut self.damaged {
            *damaged += report
                .damage
                .iter()
                .map(|r| f64::from(r.width * r.height))
                .sum::<f64>();
        }
        if zgx_fingerprint::on() {
            zgx_fingerprint::frame(&self.app.ui.scene.borrow(), &report);
        }
        if let Some(drawing) = &mut self.drawing {
            drawing.draw(&self.app.ui.scene.borrow(), &report.damage);
        }
        if let Some(((x, y), asked)) = &mut self.cursor {
            let scene = self.app.ui.scene.borrow();
            let revision = scene.interaction_revision();
            if *asked != Some(revision) {
                *asked = Some(revision);
                std::hint::black_box(self.app.ui.input.cursor_at(&scene, *x, *y));
            }
        }
        zgx::profile::frame();
    }
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame();
        }
    }
    /// `interaction` (the `k`th), each followed by its frame, `reps` times
    /// after two to warm up: its time, instructions and phases, printed.
    fn measure(&mut self, name: &str, mut interaction: impl FnMut(&mut Self, usize)) {
        // `ONLY=sort`: the interactions whose names hold it alone (a profile
        // of one); `SKIP=sort`: all but those (the gate's scene of an app
        // without an interaction it counts on its own).
        if std::env::var("ONLY").is_ok_and(|only| !name.contains(only.as_str()))
            || std::env::var("SKIP").is_ok_and(|skip| name.contains(skip.as_str()))
        {
            return;
        }
        for k in 0..2 {
            interaction(self, k);
            self.frame();
        }
        if let Some(drawing) = &mut self.drawing {
            drawing.finish();
        }
        zgx::profile::take();
        let (start, before) = (Instant::now(), instructions());
        for k in 0..self.reps {
            interaction(self, k + 2);
            self.frame();
        }
        if let Some(drawing) = &mut self.drawing {
            drawing.finish();
        }
        let reps = self.reps as f64;
        let ms = start.elapsed().as_secs_f64() * 1e3 / reps;
        let millions = (instructions() - before) as f64 / 1e6 / reps;
        let mut line = format!("{name}: {ms:.2} ms, {millions:.2} M instructions");
        let phases = zgx::profile::take();
        if !phases.is_empty() {
            let counted: u64 = phases.iter().map(|p| p.1).sum();
            for (phase, count, _) in &phases {
                if *count > 0 && !matches!(phase, Phase::Submit | Phase::Present | Phase::Wait) {
                    line += &format!(" | {} {:.2}", phase.name(), *count as f64 / 1e6 / reps);
                }
            }
            let other = millions - counted as f64 / 1e6 / reps;
            line += &format!(" | other {other:.2}");
        }
        println!("{line}");
        assert_eq!(self.app.error(), None, "{name}");
    }
    fn click_at(&mut self, (x, y): (f32, f32)) {
        let button = PointerButton::Primary;
        self.app
            .ui
            .dispatch(InputEvent::PointerDown { x, y, button });
        self.app.ui.dispatch(InputEvent::PointerUp { x, y, button });
    }
    fn bounds(&mut self, id: &str) -> Rect {
        self.app.bounds(id)
    }
}

/// The toolbar button named `label`'s bounds.
fn button(bench: &Bench, label: &str) -> Rect {
    let semantics = bench.app.ui.semantics.borrow();
    let scene = bench.app.ui.scene.borrow();
    let mut pending = vec![scene.root()];
    while let Some(id) = pending.pop() {
        if semantics
            .get(id)
            .is_some_and(|n| n.role == zgx::semantics::Role::Button && n.label == label)
        {
            return scene.bounds(id);
        }
        pending.extend(scene.children(id).iter().copied());
    }
    panic!("no {label} button")
}

fn mail(make: MakeDrawing) {
    let mut b = Bench::new("mail", (1180., 760.), &[], make);
    b.measure("mail: open a message", |b, k| {
        b.app.click_row("messages", k % 2 + 1);
    });
    b.measure("mail: search keystroke", |b, k| {
        if k % 2 == 0 {
            b.app.type_text("search", "r");
        } else {
            b.app.key(Key::Backspace);
        }
    });
    b.app.click_row("messages", 1);
    b.frames(30);
    b.measure("mail: archive", |b, _| {
        let archive = button(b, "Archive");
        b.click_at((archive.x + 4., archive.y + archive.height / 2.));
    });
}

fn dashboard(make: MakeDrawing) {
    let rows = size("ROWS", 100_000).to_string();
    // The live feed's timer ticks by the wall clock, as often as the run is
    // slow: paused from the start (a run slow enough, cachegrind's cold
    // start, ticked it before a click could pause it, a feed tick more in
    // the count), and checked paused, so each tick is the one this measures.
    let args = [rows, "paused".to_owned()];
    let mut b = Bench::with("dashboard", (1280., 800.), &args, make, |_| {});
    let calls = b.app.calls();
    std::thread::sleep(std::time::Duration::from_millis(300));
    b.frames(2);
    assert_eq!(b.app.calls(), calls, "the live feed is paused");
    b.app.click("sort-price");
    b.frames(2);
    b.measure("dashboard: feed tick, sorted by price", |b, _| {
        assert!(b.app.host.run_command("refresh"));
    });
    // Two columns in turn: each sort back to the other's order, kept.
    b.measure("dashboard: sort", |b, k| {
        b.app.click(if k % 2 == 0 {
            "sort-volume"
        } else {
            "sort-symbol"
        });
    });
    // Three others in turn: none the last sort's, so each sorts it all.
    b.measure("dashboard: sort, a column not sorted by last", |b, k| {
        b.app
            .click(["sort-price", "sort-change", "sort-sector"][k % 3]);
    });
    b.measure("dashboard: filter keystroke", |b, k| {
        if k % 2 == 0 {
            b.app.type_text("search", "B");
        } else {
            b.app.key(Key::Backspace);
        }
    });
}

fn sketch(make: MakeDrawing) {
    // A folder name of one length whatever the process's id (see `code`).
    let dir = std::env::temp_dir().join(format!("zgx-apps-bench-{:010}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("busy.json");
    let shapes: Vec<String> = (0..size("SHAPES", 500))
        .map(|i| {
            let (x, y) = ((i % 25) * 40, (i / 25) * 40);
            format!(
                r##"{{"id":"s{}","kind":"{}","name":"S{i}","x":{x},"y":{y},"w":32,"h":32,"fill":"#4c8dff","stroke":"#202124","stroke_width":1,"radius":4}}"##,
                i + 1,
                if i % 2 == 0 { "rect" } else { "ellipse" }
            )
        })
        .collect();
    std::fs::write(
        &path,
        format!(r#"{{"version":1,"shapes":[{}]}}"#, shapes.join(",")),
    )
    .unwrap();
    let mut b = Bench::new("sketch", (1240., 800.), &[], make);
    b.app.desktop.clone().unwrap().pick([&path]);
    assert!(b.app.host.run_command("open"));
    assert!(b.app.wait_until(|h| h.text("title") == "busy.json"));
    b.frames(2);
    // The board's origin: world (0, 0) on the window, at the starting view.
    let board = b.bounds("board");
    let at = |x: f32, y: f32| (board.x + 80. + x, board.y + 60. + y);
    let (x, y) = at(16., 16.);
    let button = PointerButton::Primary;
    b.app.ui.dispatch(InputEvent::PointerDown { x, y, button });
    b.frame();
    b.measure("sketch: drag a shape", |b, k| {
        let k = k as f32;
        b.app.ui.dispatch(InputEvent::PointerMove {
            x: x + k * 2.,
            y: y + k,
        });
    });
    b.app.ui.dispatch(InputEvent::PointerUp { x, y, button });
    b.app.key(Key::Escape);
    b.frame();
    let (x, y) = at(400., 300.);
    b.measure("sketch: pan", |b, k| {
        let step = if (k / 10) % 2 == 0 { 12. } else { -12. };
        b.app.ui.dispatch(InputEvent::Scroll {
            x,
            y,
            delta_x: step,
            delta_y: step / 2.,
        });
    });
    b.measure("sketch: zoom", |b, k| {
        let delta = if k % 2 == 0 { 0.05 } else { -0.05 };
        b.app.ui.dispatch(InputEvent::Pinch { x, y, delta });
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Rust file of `lines` lines, as the code editor's sample makes them.
fn source(lines: usize) -> String {
    let body = [
        "/// A function, its docs and its body.",
        "pub fn compute_N(input: &[u8], scale: f32) -> Result<Vec<u32>, Error> {",
        "    let name = \"item N\"; // a comment",
        "    let total: u32 = input.iter().map(|b| *b as u32 * N).sum();",
        "    /* a block comment",
        "       running on */",
        "    if total > 0xff { return Err(Error::Overflow(total)); }",
        "    Ok(vec![total; scale as usize])",
        "}",
        "",
    ];
    (0..lines)
        .map(|i| body[i % body.len()].replace('N', &i.to_string()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn code(make: MakeDrawing) {
    let lines = size("LINES", 100_000);
    // One name whatever the process's id: the editor shows and measures the
    // file's name (a 7-digit id cost 37 K more than a 6-digit one, and any
    // id drew other glyphs, so the output changed run to run).
    let dir = std::env::temp_dir().join(format!("zgx-apps-bench-{:010}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bench.rs");
    // The bench's own file, made uncounted: what making it costs follows
    // where its lines lie in the binary.
    gate_marks::outside(|| std::fs::write(&path, source(lines)).unwrap());
    let mut b = Bench::new("code_editor", (1100., 760.), &[], make);
    b.app.desktop.clone().unwrap().pick([&path]);
    assert!(b.app.host.run_command("open"));
    assert!(
        b.app
            .wait_until(|h| h.text("status-message").starts_with("Opened"))
    );
    b.frames(3);
    let editor = b.app.find("editor").expect("the editor");
    b.app.ui.input.focus(&b.app.ui.scene, Some(editor));
    let handle = b.app.ui.editor(editor).unwrap();
    let middle = handle.editor.borrow().len() / 2;
    handle.set_selection(middle, middle);
    b.frames(2);
    b.measure("code: keystroke", |b, k| {
        let text = if k % 2 == 0 { "x" } else { "y" };
        b.app.ui.dispatch(InputEvent::Text(text.into()));
    });
    let (x, y) = {
        let r = b.bounds("editor");
        (r.x + r.width / 2., r.y + r.height / 2.)
    };
    b.measure("code: scroll a page", |b, k| {
        let delta_y = if (k / 10) % 2 == 0 { 600. } else { -600. };
        b.app.ui.dispatch(InputEvent::Scroll {
            x,
            y,
            delta_x: 0.,
            delta_y,
        });
    });
    assert!(b.app.host.run_command("find"));
    b.frames(2);
    b.app.type_text("find", "Overflo");
    b.frames(2);
    b.measure("code: find-all keystroke", |b, k| {
        if k % 2 == 0 {
            b.app.ui.dispatch(InputEvent::Text("w".into()));
        } else {
            b.app.key(Key::Backspace);
        }
    });
    // Every query new (a letter more each time, never searched before):
    // the whole text read, where the keystroke above, a letter typed and
    // taken back, is answered from the searches kept.
    b.measure("code: find-all, a new letter", |b, k| {
        let letter = char::from(b'a' + (k % 26) as u8);
        b.app.ui.dispatch(InputEvent::Text(letter.to_string()));
    });
    let _ = std::fs::remove_dir_all(&dir);
}

fn chat(make: MakeDrawing) {
    let server = chat_server::ChatServer::manual();
    let mut b = Bench::hosted(
        "chat",
        (1100., 760.),
        &[],
        make,
        |host| server.serve(host),
        |_| {},
    );
    // A long reply under way, as a model writes one: a token a frame.
    let reply: Vec<String> =
        chat_server::ChatServer::tokens(&chat_server::ChatServer::reply(0).repeat(8))
            .into_iter()
            .map(str::to_owned)
            .collect();
    b.app.type_text("composer", "Write me a function");
    b.app.key(Key::Enter);
    assert!(b.app.wait_until(|_| server.streaming()));
    for token in &reply[..40] {
        server.write(token);
        b.frame();
    }
    b.measure("chat: a streamed token", |b, k| {
        server.write(&reply[(40 + k) % reply.len()]);
        let _ = b;
    });
    server.finish();
    assert!(b.app.wait_until(|h| h.find("send").is_some()));
    b.frames(3);
    b.measure("chat: composer keystroke", |b, k| {
        if k % 2 == 0 {
            b.app.type_text("composer", "a");
        } else {
            b.app.key(Key::Backspace);
        }
    });
    assert!(b.app.host.run_command("new"));
    b.frames(3);
    b.measure("chat: switch conversation", |b, k| {
        let to = if k % 2 == 0 { "previous" } else { "next" };
        assert!(b.app.host.run_command(to));
    });
}

/// A reply streaming into a chat's transcript as an app draws one
/// (`stream_fade/app.luau`): a measured list following its end, the reply
/// under way parsed by `ui.markdown` and drawn by a `ui.each`, a chunk of
/// `CHUNK` tokens (4) every third frame, as a model's server sends some four
/// tokens each 50 ms, its newest words fading in. `FADE` is how: `bindings`
/// (a colour function per fading chunk and a tick state animated every
/// frame, an app doing it in script), `native` (`fade_in`, drawn in Rust) or
/// `none`. `PRIMED` chunks are written first (200), uncounted (and with
/// `OFF=1` the transcript scrolled up off the reply); then `REPS`
/// rounds of a chunk's frame and two frames between chunks, and `COUNT`
/// says which the gate counts: `chunk`, `between` or `all` (the rest go
/// uncounted, `gate_marks::outside`).
fn stream(make: MakeDrawing) {
    let fade = std::env::var("FADE").unwrap_or_else(|_| "bindings".into());
    let count = std::env::var("COUNT").unwrap_or_else(|_| "all".into());
    let (primed, chunk) = (size("PRIMED", 200), size("CHUNK", 4));
    let all = (0..3)
        .map(chat_server::ChatServer::reply)
        .collect::<Vec<_>>()
        .join("\n\n");
    let tokens: Vec<String> = chat_server::ChatServer::tokens(&all)
        .into_iter()
        .map(str::to_owned)
        .collect();
    let next = Rc::new(std::cell::Cell::new(0usize));
    let mode = fade.clone();
    let boost = std::env::var("BOOST").map_or(true, |b| b != "0");
    // Its time a frame's at a time (`Bench::clock`), installed before it
    // loads: the time a chunk starts fading is the clock's too. `CLOCK=real`
    // leaves it on the wall clock (as the gate counted it before; a base
    // whose bench doesn't read `CLOCK` counted that, `READS`).
    let fixed = std::env::var("CLOCK").map_or(true, |c| c != "real");
    let clock = fixed.then(zgx::clock::VirtualClock::new);
    let mut b = Bench::hosted(
        "stream_fade",
        (1400., 900.),
        &[],
        make,
        |host| {
            host.function("stream.mode", move |_| {
                zgx_script::Value::from(mode.as_str())
            });
            host.function("stream.boost", move |_| zgx_script::Value::Bool(boost));
            let next = next.clone();
            host.function("stream.chunk", move |_| {
                let at = next.get();
                next.set(at + chunk);
                let text: String = (at..at + chunk)
                    .map(|k| tokens[k % tokens.len()].as_str())
                    .collect();
                zgx_script::Value::from(text.as_str())
            });
        },
        |_| {},
    );
    b.clock = clock.map(|clock| (clock, 0));
    // `OFF=1`: the transcript scrolled up, off the reply, before the chunks
    // counted: what they cost while it isn't shown (a reply scrolled away
    // from, or behind another chat).
    let off = std::env::var("OFF").is_ok_and(|o| o == "1");
    let pointer = std::env::var("POINTER").map_or(true, |p| p != "0");
    gate_marks::outside(|| {
        for _ in 0..primed {
            assert!(b.app.host.run_command("chunk"));
            b.frames(3);
        }
        if off {
            b.app.scroll("transcript", -1e6);
            b.frames(30);
        }
        // The pointer resting over the transcript, as a reader's does
        // (`POINTER=0` leaves it outside the window): what hovering costs
        // while the rows under it move.
        if pointer {
            b.app
                .ui
                .dispatch(InputEvent::PointerMove { x: 640., y: 450. });
            b.cursor = Some(((640., 450.), None));
            b.frames(2);
            assert!(
                b.app.ui.input.hovered().is_some(),
                "the pointer is over nothing"
            );
        }
    });
    assert_eq!(b.app.error(), None);
    b.damaged = Some(0.);
    let counted = |what: &str| count == "all" || count == what;
    let (mut chunk_time, mut between_time) = (Duration::ZERO, Duration::ZERO);
    let mut between_damage = 0.;
    for _ in 0..b.reps {
        let started = Instant::now();
        let chunk_frame = |b: &mut Bench| {
            assert!(b.app.host.run_command("chunk"));
            b.frame();
        };
        if counted("chunk") {
            chunk_frame(&mut b)
        } else {
            gate_marks::outside(|| chunk_frame(&mut b))
        }
        chunk_time += started.elapsed();
        let (started, damaged) = (Instant::now(), b.damaged.unwrap_or(0.));
        if counted("between") {
            b.frames(2)
        } else {
            gate_marks::outside(|| b.frames(2))
        }
        between_time += started.elapsed();
        between_damage += b.damaged.unwrap_or(0.) - damaged;
    }
    if let Some(drawing) = &mut b.drawing {
        drawing.finish();
    }
    let reps = b.reps as f64;
    println!(
        "stream ({fade}, boost {boost}{}{}): a chunk's frame {:.3} ms, a frame between {:.3} ms, damaging {:.0} pt²",
        if off { ", off screen" } else { "" },
        if pointer { "" } else { ", no pointer" },
        chunk_time.as_secs_f64() * 1e3 / reps,
        between_time.as_secs_f64() * 1e3 / reps / 2.,
        between_damage / reps / 2.
    );
}

/// The apps `APP` names (all of them if none), drawn by what `make` makes.
pub fn run(make: MakeDrawing) {
    let only = std::env::var("APP").ok();
    for (name, run) in [
        ("mail", mail as fn(MakeDrawing)),
        ("dashboard", dashboard),
        ("sketch", sketch),
        ("code", code),
        ("chat", chat),
        ("stream", stream),
    ] {
        if only.as_deref().is_none_or(|only| only == name) {
            run(make);
        }
    }
}
