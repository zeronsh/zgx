//! `ui.render_canvas`: a drawing as a PNG, with no canvas, window or frame.
mod common;

use common::Harness;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use zgx_script::{ScriptHost, Value};

const APP: &str = r##"
local function draw(c, w, h)
    c:rect(0, 0, w, h, "#ffffff")
    c:rect(10, 10, 30, 20, "#ff0000", 4)
    c:text("Hi", 50, 10, { size = 14, color = "#000000" })
    ui.invoke("measured", (c:measure_text("Hello, world", { size = 14 })))
end
ui.task(function()
    ui.invoke("rendered", ui.render_canvas(draw, { width = 80, height = 40, scale = 2 }):await())
    -- A frame of a board: its point (500, 300) at the top left.
    ui.invoke("moved", ui.render_canvas(function(c)
        c:rect(500, 300, 10, 10, "$accent")
    end, { width = 20, height = 20, x = 500, y = 300 }):await())
end)
return ui.div {}
"##;

/// What the script gave each action, by name.
type Heard = Rc<RefCell<Vec<(String, Value)>>>;

fn app() -> (Harness, Heard) {
    let heard: Heard = Rc::default();
    let h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        for name in ["rendered", "moved", "measured"] {
            let heard = heard.clone();
            host.action(name, move |value| {
                heard.borrow_mut().push((name.into(), value))
            });
        }
        host
    });
    (h, heard)
}

/// What the script gave `name`, waiting for it on the tasks alone: no
/// frame is drawn.
fn wait(h: &Harness, heard: &Heard, name: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        h.settle();
        if let Some((_, value)) = heard.borrow().iter().find(|(n, _)| n == name) {
            return value.clone();
        }
        if let Some(e) = h.host.error().with_untracked(|e| e.clone()) {
            panic!("script error: {e}");
        }
        assert!(Instant::now() < deadline, "no `{name}`");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A PNG in base64, decoded: its size and RGBA pixels.
fn decode(value: &Value) -> (u32, u32, Vec<u8>) {
    let Value::Str(text) = value else {
        panic!("not a string: {value:?}");
    };
    let digit = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => panic!("not base64: {c}"),
    };
    let mut bytes = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let n = chunk
            .iter()
            .filter(|&&c| c != b'=')
            .enumerate()
            .fold(0_u32, |n, (i, &c)| n | u32::from(digit(c)) << (18 - 6 * i));
        let kept = chunk.iter().filter(|&&c| c != b'=').count() - 1;
        bytes.extend_from_slice(&n.to_be_bytes()[1..1 + kept]);
    }
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    (info.width, info.height, pixels)
}

#[test]
fn a_drawing_renders_to_a_png_with_no_window_or_frame() {
    let (h, heard) = app();
    let (w, height, pixels) = decode(&wait(&h, &heard, "rendered"));
    // 80 by 40 points at 2 pixels a point.
    assert_eq!((w, height), (160, 80));
    let at = |x: u32, y: u32| &pixels[((y * w + x) * 4) as usize..][..4];
    assert_eq!(at(2, 2), &[255, 255, 255, 255], "the white ground");
    assert_eq!(
        at(40, 40),
        &[255, 0, 0, 255],
        "the red box, at (10, 10) points"
    );
    assert_eq!(at(78, 2), &[255, 255, 255, 255], "beside the box");
    // "Hi" inked near (50, 10) points, black over white.
    let inked = (100..140)
        .flat_map(|x| (20..60).map(move |y| (x, y)))
        .filter(|&(x, y)| at(x, y)[0] < 100)
        .count();
    assert!(inked > 20, "{inked} pixels of text");
}

#[test]
fn x_and_y_put_that_point_at_the_top_left_with_a_transparent_ground() {
    let (h, heard) = app();
    let (w, height, pixels) = decode(&wait(&h, &heard, "moved"));
    assert_eq!((w, height), (20, 20));
    let at = |x: u32, y: u32| &pixels[((y * w + x) * 4) as usize..][..4];
    assert_eq!(at(5, 5)[3], 255, "the box drawn at (500, 300)");
    assert_eq!(at(15, 15), &[0, 0, 0, 0], "no background: transparent");
}

#[test]
fn measure_text_measures_as_a_window_does() {
    let (h, heard) = app();
    let Value::Num(width) = wait(&h, &heard, "measured") else {
        panic!("not a width");
    };
    // What a window's renderer measures with (`GpuRenderer::install_text`).
    let mut scene = zgx::scene::Scene::new(1., 1.);
    zgx_gpu::TextServices::new().install(&mut scene);
    let (shown, _) = scene.measure_text_with_font("Hello, world", 14., None, &Default::default());
    assert!(width > 40., "{width}");
    assert_eq!(width as f32, shown);
}

/// A platform whose app has its own fonts, as Zeron has Geist, and keeps
/// those a script adds (as zgx-desktop's `WindowFactory::add_font` does).
struct WithFonts(RefCell<Vec<std::sync::Arc<[u8]>>>);
impl zgx::platform::Platform for WithFonts {
    fn fonts(&self) -> Vec<std::sync::Arc<[u8]>> {
        self.0.borrow().clone()
    }
    fn add_font(&self, bytes: std::sync::Arc<[u8]>) -> Result<Vec<String>, String> {
        let font = zgx_gpu::text::FontData::new(bytes.clone())?;
        self.0.borrow_mut().push(bytes);
        Ok(font.families().to_vec())
    }
    fn clipboard_text(&self) -> Option<String> {
        None
    }
    fn set_clipboard_text(&self, _: String) -> bool {
        false
    }
    fn open_files(
        &self,
        _: zgx::platform::FilePicker,
    ) -> zgx::platform::Pending<Result<Vec<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn save_file(
        &self,
        _: zgx::platform::FilePicker,
    ) -> zgx::platform::Pending<Result<Option<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(None) })
    }
}
const GEIST: &[u8] = include_bytes!("../../zgx-desktop/examples/support/fonts/GeistVariable.ttf");
const LINE: &str = "The quick brown fox jumps";

#[test]
fn the_apps_own_fonts_measure_and_draw_as_in_its_windows() {
    let heard: Heard = Rc::default();
    let sink = heard.clone();
    let h = Harness::wrapped(
        move |runtime| {
            let host = ScriptHost::inline(
                runtime,
                "app",
                r##"
ui.on_mount(function()
    ui.task(function()
        local font = { font = "Geist", size = 16, weight = 600, color = "#000000" }
        ui.invoke("rendered", ui.render_canvas(function(c)
            ui.invoke("measured", (c:measure_text("The quick brown fox jumps", font)))
            c:text("The quick brown fox jumps", 0, 0, font)
        end, { width = 300, height = 30, background = "#ffffff" }):await())
    end)
end)
return ui.div {}
"##,
            );
            for name in ["rendered", "measured"] {
                let heard = sink.clone();
                host.action(name, move |value| {
                    heard.borrow_mut().push((name.into(), value))
                });
            }
            host
        },
        |view| {
            let fonts = WithFonts(RefCell::new(vec![GEIST.into()]));
            zgx::compose::provide(zgx::platform::PlatformService(Rc::new(fonts)), view)
        },
    );
    let Value::Num(measured) = wait(&h, &heard, "measured") else {
        panic!("not a width");
    };
    // A window's measure, with the app's fonts installed as its renderer's.
    let font = zgx::text_layout::FontStyle {
        family: zgx::text_layout::FontFamily::Named("Geist".into()),
        weight: 600,
        ..Default::default()
    };
    let width = |fonts: &[&[u8]]| {
        let services = zgx_gpu::TextServices::new();
        for bytes in fonts {
            let font = zgx_gpu::text::FontData::new(bytes.to_vec()).unwrap();
            font.install(&mut services.text_system().borrow_mut());
        }
        let mut scene = zgx::scene::Scene::new(1., 1.);
        services.install(&mut scene);
        scene.measure_text_with_font(LINE, 16., None, &font).0
    };
    let (geist, fallback) = (width(&[GEIST]), width(&[]));
    assert_eq!(measured as f32, geist);
    assert!((geist - fallback).abs() > 2., "{geist} and {fallback}");
    // Drawn in Geist: its ink ends where Geist's width does.
    let (w, _, pixels) = decode(&wait(&h, &heard, "rendered"));
    let right = (0..w)
        .rev()
        .find(|&x| (0..30).any(|y| pixels[((y * w + x) * 4) as usize] < 128))
        .unwrap() as f32
        + 1.;
    assert!((right - geist).abs() <= 2., "ink to {right}, Geist {geist}");
}

#[test]
fn a_font_loaded_with_load_font_renders_too() {
    let heard: Heard = Rc::default();
    let sink = heard.clone();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../zgx-desktop/examples/support/fonts/GeistVariable.ttf"
    )
    .replace('\\', "/");
    let h = Harness::wrapped(
        move |runtime| {
            let code = format!(
                r##"
ui.on_mount(function()
    ui.task(function()
        ui.load_font("{path}"):await()
        ui.render_canvas(function(c)
            ui.invoke("measured", (c:measure_text("{LINE}", {{ font = "Geist", size = 16 }})))
        end, {{ width = 4, height = 4 }}):await()
    end)
end)
return ui.div {{}}
"##
            );
            let host = ScriptHost::inline(runtime, "app", &code);
            let heard = sink.clone();
            host.action("measured", move |value| {
                heard.borrow_mut().push(("measured".into(), value))
            });
            host
        },
        |view| {
            let fonts = WithFonts(RefCell::new(Vec::new()));
            zgx::compose::provide(zgx::platform::PlatformService(Rc::new(fonts)), view)
        },
    );
    let Value::Num(measured) = wait(&h, &heard, "measured") else {
        panic!("not a width");
    };
    let services = zgx_gpu::TextServices::new();
    let geist = zgx_gpu::text::FontData::new(GEIST).unwrap();
    geist.install(&mut services.text_system().borrow_mut());
    let mut scene = zgx::scene::Scene::new(1., 1.);
    services.install(&mut scene);
    let font = zgx::text_layout::FontStyle {
        family: zgx::text_layout::FontFamily::Named("Geist".into()),
        ..Default::default()
    };
    assert_eq!(
        measured as f32,
        scene.measure_text_with_font(LINE, 16., None, &font).0
    );
}

#[test]
fn a_paragraph_wraps_to_its_width_as_it_measures() {
    let heard: Heard = Rc::default();
    let h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r##"
ui.task(function()
    local opts = { size = 14, width = 100, color = "#000000" }
    local text = string.rep("word ", 12)
    ui.invoke("rendered", ui.render_canvas(function(c)
        local w, h = c:measure_text(text, opts)
        ui.invoke("measured", { w, h })
        c:text(text, 0, 0, opts)
    end, { width = 200, height = 200, background = "#ffffff" }):await())
end)
return ui.div {}
"##,
        );
        for name in ["rendered", "measured"] {
            let heard = heard.clone();
            host.action(name, move |value| {
                heard.borrow_mut().push((name.into(), value))
            });
        }
        host
    });
    let measured = wait(&h, &heard, "measured");
    let height = match &measured {
        Value::List(size) => match size[1] {
            Value::Num(h) => h as f32,
            _ => panic!("{measured:?}"),
        },
        _ => panic!("{measured:?}"),
    };
    // What a window measures the paragraph at.
    let mut scene = zgx::scene::Scene::new(1., 1.);
    zgx_gpu::TextServices::new().install(&mut scene);
    let (_, shown) =
        scene.measure_text_with_font(&"word ".repeat(12), 14., Some(100.), &Default::default());
    assert_eq!(height, shown);
    assert!(height > 50., "several lines: {height}");
    // Drawn as measured: ink within 100 points across, down its lines.
    let (w, _, pixels) = decode(&wait(&h, &heard, "rendered"));
    let ink: Vec<(u32, u32)> = (0..w)
        .flat_map(|x| (0..200).map(move |y| (x, y)))
        .filter(|&(x, y)| pixels[((y * w + x) * 4) as usize] < 128)
        .collect();
    let right = ink.iter().map(|p| p.0).max().unwrap() as f32;
    let bottom = ink.iter().map(|p| p.1).max().unwrap() as f32;
    assert!(right < 100., "ink to {right}, past the width");
    assert!(
        bottom > height - 20. && bottom < height,
        "ink down to {bottom} of {height}"
    );
}

#[test]
fn a_size_must_be_given_and_over_zero() {
    let runtime = zgx::reactive::Runtime::new();
    for (opts, key) in [
        ("{ height = 10 }", "width"),
        ("{ width = 0, height = 10 }", "width"),
        ("{ width = 10, height = 10, scale = -1 }", "scale"),
    ] {
        let host = ScriptHost::inline(
            &runtime,
            "app",
            format!("ui.render_canvas(function() end, {opts}) return ui.div {{}}"),
        );
        let error = host.reload().unwrap_err();
        assert!(
            error.contains(&format!("`{key}` is a size in points, over 0")),
            "{opts}: {error}"
        );
    }
}

const FORMATS: &str = r##"
local function draw(c, w, h)
    c:rect(10, 10, 30, 20, "#ff0000", 4)
end
ui.task(function()
    ui.invoke("jpeg", ui.render_canvas(draw, { width = 80, height = 40, scale = 2, format = "jpeg", quality = 80 }):await())
    ui.invoke("webp", ui.render_canvas(draw, { width = 80, height = 40, format = "webp" }):await())
    ui.invoke("bad", (pcall(ui.render_canvas, draw, { width = 8, height = 8, format = "tiff" })))
end)
return ui.div {}
"##;

/// Base64 decoded (padded, as `render_canvas` gives).
fn bytes_of(value: &Value) -> Vec<u8> {
    let Value::Str(text) = value else {
        panic!("not a string: {value:?}");
    };
    let digit = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let kept: Vec<u8> = chunk.iter().copied().filter(|&c| c != b'=').collect();
        let n = kept
            .iter()
            .enumerate()
            .fold(0_u32, |n, (i, &c)| n | u32::from(digit(c)) << (18 - 6 * i));
        out.extend_from_slice(&n.to_be_bytes()[1..kept.len()]);
    }
    out
}

#[test]
fn a_drawing_renders_to_a_jpeg_or_a_webp() {
    let heard: Heard = Rc::default();
    let h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", FORMATS);
        for name in ["jpeg", "webp", "bad"] {
            let heard = heard.clone();
            host.action(name, move |value| {
                heard.borrow_mut().push((name.into(), value))
            });
        }
        host
    });
    // A JPEG: its start marker, and its frame's size (SOF0/SOF2) at 2x.
    let jpeg = bytes_of(&wait(&h, &heard, "jpeg"));
    assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF]);
    let sof = jpeg
        .windows(2)
        .position(|w| w[0] == 0xFF && (w[1] == 0xC0 || w[1] == 0xC2))
        .expect("a frame header");
    let size = |i: usize| u16::from_be_bytes([jpeg[i], jpeg[i + 1]]);
    assert_eq!((size(sof + 7), size(sof + 5)), (160, 80));
    // A lossless WebP (VP8L): its size, less one, in 14 bits each.
    let webp = bytes_of(&wait(&h, &heard, "webp"));
    assert_eq!((&webp[..4], &webp[8..16]), (&b"RIFF"[..], &b"WEBPVP8L"[..]));
    let bits = u32::from_le_bytes([webp[21], webp[22], webp[23], webp[24]]);
    assert_eq!(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1), (80, 40));
    // A format it doesn't make: refused as asked.
    assert_eq!(wait(&h, &heard, "bad"), Value::Bool(false));
}

/// A platform whose save dialog answers with `saves`.
struct SavesTo(std::path::PathBuf);
impl zgx::platform::Platform for SavesTo {
    fn clipboard_text(&self) -> Option<String> {
        None
    }
    fn set_clipboard_text(&self, _: String) -> bool {
        false
    }
    fn open_files(
        &self,
        _: zgx::platform::FilePicker,
    ) -> zgx::platform::Pending<Result<Vec<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn save_file(
        &self,
        _: zgx::platform::FilePicker,
    ) -> zgx::platform::Pending<Result<Option<std::path::PathBuf>, String>> {
        let path = self.0.clone();
        Box::pin(async move { Ok(Some(path)) })
    }
}

#[test]
fn a_big_drawing_renders_straight_to_a_picked_file() {
    let dir = std::env::temp_dir().join(format!("zgx-render-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let saves = dir.join("board.png");
    let heard: Heard = Rc::default();
    let sink = heard.clone();
    let to = saves.clone();
    let h = Harness::wrapped(
        move |runtime| {
            let host = ScriptHost::inline(
                runtime,
                "app",
                r##"
local function board(c, w, h)
    c:rect(0, 0, w, h, "#ffffff")
    -- Clips inside clips and a shadow the board's size: at 3x each mask
    -- and the shadow are over the 32 MiB a window's picture keeps to.
    for _ = 1, 4 do
        c:clip(ui.path():move(0, 0):line(w, 0):line(w, h):line(0, h):close())
    end
    c:shadow(ui.path():move(20, 20):line(w - 20, 20):line(w - 20, h - 20):line(20, h - 20):close(), { y = 8, blur = 24 })
    c:rect(100, 100, 400, 200, "#ff0000", 16)
    for _ = 1, 4 do
        c:restore()
    end
end
ui.task(function()
    -- A 1440 by 900 board at 3x is 46 MiB of pixels: more than a picture
    -- given back may be, written to a file all the same.
    local given = pcall(function()
        ui.render_canvas(board, { width = 1440, height = 900, scale = 3 }):await()
    end)
    local file = ui.save_file({ name = "board.png" }):await()
    local made = ui.render_canvas(board, { width = 1440, height = 900, scale = 3, file = file }):await()
    ui.render_canvas(board, { width = 1440, height = 900, scale = 3, file = file, format = "jpeg" }):await()
    local sneaky = pcall(ui.render_canvas, board, { width = 8, height = 8, file = "/tmp/elsewhere.png" })
    ui.invoke("done", `{given} {made == nil} {sneaky}`)
end)
return ui.div {}
"##,
            );
            let heard = sink.clone();
            host.action("done", move |value| {
                heard.borrow_mut().push(("done".into(), value))
            });
            host
        },
        move |view| {
            zgx::compose::provide(
                zgx::platform::PlatformService(Rc::new(SavesTo(to.clone()))),
                view,
            )
        },
    );
    // (Three 4320 by 2700 pictures, slow in a debug build: its own wait.)
    let deadline = Instant::now() + Duration::from_secs(180);
    let done = loop {
        h.settle();
        if let Some((_, value)) = heard.borrow().iter().find(|(n, _)| n == "done") {
            break value.clone();
        }
        if let Some(e) = h.host.error().with_untracked(|e| e.clone()) {
            panic!("script error: {e}");
        }
        assert!(Instant::now() < deadline, "no `done`");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(done, Value::Str("false true false".into()));
    // The last write, a JPEG: its frame 4320 by 2700 pixels.
    let jpeg = std::fs::read(&saves).unwrap();
    assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF]);
    let sof = jpeg
        .windows(2)
        .position(|w| w[0] == 0xFF && (w[1] == 0xC0 || w[1] == 0xC2))
        .unwrap();
    let size = |i: usize| u16::from_be_bytes([jpeg[i], jpeg[i + 1]]);
    assert_eq!((size(sof + 7), size(sof + 5)), (4320, 2700));
    assert!(!saves.with_extension("zgx-write").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
