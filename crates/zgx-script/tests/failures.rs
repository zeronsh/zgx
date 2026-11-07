//! Failures a running app meets go on: a handler's runtime error is
//! reported (logged, and shown over the window in debug builds) while the
//! rest of the app keeps working; a missing image draws empty with its
//! error reported; a font that isn't installed falls back.
mod common;

use common::Harness;

/// A failing text computed reports its error without stopping the other
/// texts, and can return text again on its next update.
#[test]
fn a_computed_text_that_fails_reports_and_the_rest_update() {
    let mut h = Harness::inline(
        r#"local tick = ui.shared("tick", 0)
        return ui.column {
            ui.text { id = "first", function() tick:get(); return "first", "ignored" end },
            ui.text { id = "bad", function()
                if tick:get() == 1 then error("computed text failure marker") end
                return "recovered"
            end },
            ui.text { id = "last", function() return "last " .. tick:get() end },
            ui.text { id = "none", function() return end },
        }"#,
    );
    assert_eq!(h.text("first"), "first");
    assert_eq!(h.text("none"), "");
    h.host.shared("tick", 0.).set(zgx_script::Value::Num(1.));
    h.settle();
    assert!(h.error().unwrap().contains("computed text failure marker"));
    assert_eq!(h.text("bad"), "⚠");
    assert_eq!(h.text("last"), "last 1");
    h.host.shared("tick", 0.).set(zgx_script::Value::Num(2.));
    h.settle();
    assert_eq!(h.text("bad"), "recovered");
    assert_eq!(h.text("last"), "last 2");
}

#[test]
fn a_handlers_runtime_error_is_reported_and_the_app_keeps_working() {
    let mut h = Harness::inline(
        r#"
local n = ui.state("n", 0)
return ui.column {
	ui.button { id = "broken", on_click = function() local t = nil; t.x = 1 end, "Broken" },
	ui.button { id = "more", on_click = ui.add(n), "More" },
	ui.text { "{n}", id = "n" },
}
"#,
    );
    h.click("broken");
    h.frame();
    let error = h.error().expect("the error is reported");
    assert!(error.contains("attempt to index nil"), "{error}");
    h.click("more");
    h.frame();
    assert_eq!(h.text("n"), "1", "the app still answers");
}

#[test]
fn a_missing_image_draws_empty_and_a_missing_font_falls_back() {
    let mut h = Harness::inline(
        r#"
return ui.column {
	ui.image { src = "no/such/picture.png", id = "picture", w = 40, h = 40 },
	ui.text { "Still here", id = "text", font = "No Such Font Family" },
}
"#,
    );
    h.frame();
    assert_eq!(h.text("text"), "Still here");
    assert!(h.find("picture").is_some(), "its node keeps its place");
    // The file's doing, not the script's: a warning, never an error (which
    // would stop the window's reloads).
    assert_eq!(h.error(), None);
    let warnings = h.0.host.warnings().join("\n");
    assert!(warnings.contains("picture.png"), "{warnings}");
}

/// A missing image is reported with the script line that named it, in
/// the file that names it: it loads once the node mounts, when no script
/// line runs.
#[test]
fn a_missing_image_names_the_line_that_asked_for_it() {
    let warning_of = |name: &str, files: &[(&str, &str)]| {
        let dir =
            std::env::temp_dir().join(format!("zgx-image-site-{}-{name}", std::process::id()));
        for (file, code) in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, code).unwrap();
        }
        let path = dir.join("app.luau");
        let h = Harness::new(|runtime| zgx_script::ScriptHost::new(runtime, path));
        h.frame();
        assert_eq!(h.error(), None);
        h.0.host.warnings().join("\n")
    };
    let error = warning_of(
        "bg",
        &[(
            "app.luau",
            "return ui.column {\n\n\tui.div { bg_image = 'no-such.png', w = 40, h = 40 },\n}\n",
        )],
    );
    assert!(
        error.contains("app.luau:3: ") && error.contains("no-such.png"),
        "{error}"
    );
    let error = warning_of(
        "module",
        &[
            (
                "app.luau",
                "local card = require('./components/card')\nreturn card()\n",
            ),
            (
                "components/card.luau",
                "return function()\n\treturn ui.image { src = \"no/such/picture.png\", w = 40, h = 40 }\nend\n",
            ),
        ],
    );
    // The module as its chunk is named, with the system's separator
    // (`components\card.luau` on Windows).
    assert!(
        error
            .replace('\\', "/")
            .contains("components/card.luau:2: "),
        "{error}"
    );
    // Not a line of tools' files or declarations, which may name the
    // same text (and sort first).
    let error = warning_of(
        "types",
        &[
            (".tool/notes.luau", "-- see \"gone.png\"\n"),
            ("a/extra.d.luau", "-- see \"gone.png\"\n"),
            (
                "app.luau",
                "return ui.image { src = \"gone.png\", w = 40, h = 40 }\n",
            ),
        ],
    );
    assert!(error.contains("app.luau:1: "), "{error}");
}

/// An empty `src` is no picture yet, as the web's: an image, a
/// background or an SVG shows nothing and is no error, and a missing file
/// is warned of once `src` names it.
#[test]
fn an_empty_image_src_shows_nothing_and_is_no_error() {
    let mut h = Harness::inline(
        r#"
local src = ui.state("src", "")
return ui.column {
	ui.image { src = src, id = "picture", w = 40, h = 40 },
	ui.div { bg_image = src, id = "backdrop", w = 40, h = 40 },
	ui.svg { src = src, id = "icon", w = 20, h = 20 },
}
"#,
    );
    h.frame();
    assert!(h.find("picture").is_some(), "its node keeps its place");
    assert_eq!(h.error(), None);
    h.0.host
        .set_state("src", zgx_script::Value::Str("no/such/picture.png".into()))
        .unwrap();
    h.frame();
    assert_eq!(h.error(), None);
    let warnings = h.0.host.warnings().join("\n");
    assert!(warnings.contains("picture.png"), "{warnings}");
}

/// A 2×2 PNG.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0,
    0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 132,
    25, 96, 12, 0, 71, 202, 7, 249, 103, 89, 110, 183, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
    130,
];

/// A picture's file gone, or not a picture, is told to `on_error` (no
/// warning then) and shows `fallback`; one that loads tells `on_load` its
/// size, and `ui.image_size` reads it from the file's header, before
/// anything draws it.
#[test]
fn an_image_tells_what_became_of_its_file() {
    let dir = std::env::temp_dir().join(format!("zgx-image-told-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("shot.png"), PNG).unwrap();
    std::fs::write(dir.join("notes.png"), b"not a picture").unwrap();
    let app = dir.join("app.luau");
    std::fs::write(
        &app,
        r#"
local heard = ui.state("heard", "")
local function hear(what: string)
	heard:append(what .. ";")
end
local w, h = ui.image_size("shot.png")
local none = ui.image_size("gone.png")
hear(`size {w}x{h} {none}`)
return ui.column {
	ui.image {
		src = "shot.png", w = 20, h = 20,
		on_load = function(size) hear(`load {size.w}x{size.h}`) end,
		on_error = function() hear("shot failed") end,
	},
	ui.image {
		src = "gone.png", w = 20, h = 20,
		on_error = function(why: string) hear(if why:find("gone.png") then "gone" else why) end,
		fallback = ui.text { "Couldn't be shown", id = "fallback" },
	},
	ui.image {
		src = "notes.png", w = 20, h = 20,
		on_error = function() hear("not a picture") end,
	},
	ui.text { heard, id = "heard" },
}
"#,
    )
    .unwrap();
    let mut h = Harness::new(|runtime| zgx_script::ScriptHost::new(runtime, app));
    h.frame();
    h.frame();
    assert_eq!(h.error(), None);
    assert!(h.find("fallback").is_some(), "a file gone shows `fallback`");
    let heard = h.text("heard");
    for what in ["size 2x2 nil", "load 2x2", "gone", "not a picture"] {
        assert!(heard.contains(what), "{what}: {heard}");
    }
    assert!(!heard.contains("shot failed"), "{heard}");
    // Each told once, and those told need no warning.
    h.frame();
    assert_eq!(h.text("heard").matches("load").count(), 1, "{heard}");
    assert!(h.0.host.warnings().is_empty(), "{:?}", h.0.host.warnings());
}

/// An 8×8 GIF of two frames (Zeron's `blink.gif`).
const GIF: &[u8] = &[
    71, 73, 70, 56, 57, 97, 8, 0, 8, 0, 129, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 33, 255, 11,
    78, 69, 84, 83, 67, 65, 80, 69, 50, 46, 48, 3, 1, 0, 0, 0, 33, 249, 4, 0, 20, 0, 0, 0, 44, 0,
    0, 0, 0, 8, 0, 8, 0, 0, 8, 15, 0, 1, 8, 28, 72, 176, 160, 193, 131, 8, 19, 42, 76, 24, 16, 0,
    33, 249, 4, 1, 20, 0, 1, 0, 44, 0, 0, 0, 0, 8, 0, 8, 0, 129, 255, 255, 255, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 8, 15, 0, 1, 8, 28, 72, 176, 160, 193, 131, 8, 19, 42, 76, 24, 16, 0, 59,
];

/// A GIF in `ui.image` (zgx-gpu's `gif` feature, on by default) shows,
/// sized as its frames are, no error, and plays.
#[test]
fn a_gif_shows() {
    let dir = std::env::temp_dir().join(format!("zgx-image-gif-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("blink.gif"), GIF).unwrap();
    let app = dir.join("app.luau");
    std::fs::write(
        &app,
        r#"local w, h = ui.image_size("blink.gif")
return ui.column {
	ui.image { src = "blink.gif", id = "gif", on_error = function(why: string) print(why) end },
	ui.text { `{w}x{h}`, id = "size" },
}
"#,
    )
    .unwrap();
    let mut h = Harness::new(|runtime| zgx_script::ScriptHost::new(runtime, app));
    h.frame();
    assert_eq!(h.error(), None);
    assert!(h.0.host.warnings().is_empty(), "{:?}", h.0.host.warnings());
    assert_eq!(h.text("size"), "8x8");
    let node = h.find("gif").unwrap();
    let bounds = h.ui.scene.borrow().bounds(node);
    assert_eq!(
        (bounds.width, bounds.height),
        (8., 8.),
        "as large as its pixels"
    );
    // It plays: its frames (200 ms each) take turns.
    let frame = |h: &mut Harness| {
        let scene = h.ui.scene.borrow();
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if let zgx::scene::NodeKind::Image(image) = scene.kind(n) {
                return image.pixels().to_vec();
            }
            stack.extend(scene.children(n).iter().copied());
        }
        Vec::new()
    };
    let first = frame(&mut h);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while frame(&mut h) == first && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.settle();
        h.frame();
    }
    assert_ne!(frame(&mut h), first, "the next frame shows");
}
