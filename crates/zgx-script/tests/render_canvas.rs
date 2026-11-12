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
