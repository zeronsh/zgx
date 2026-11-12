//! Widths from what a box holds, as scripts write them: `w = "max-content"`.
mod common;

use common::Harness;

fn width(h: &mut Harness, id: &str) -> f32 {
    h.frames(2);
    let node = h.find(id).unwrap();
    h.ui.scene.borrow().bounds(node).width
}

/// A text that wraps, in a wide column: as wide as its line, as narrow as
/// its longest word, and as wide as the room in between.
#[test]
fn text_takes_its_line_its_longest_word_or_the_room() {
    let mut h = Harness::inline(
        r##"local words = "several words that wrap"
        return ui.column { w = 1000, h = 400,
            ui.text { id = "stretched", words },
            ui.text { id = "line", w = "max-content", wrap = true, words },
            ui.text { id = "word", w = "min-content", wrap = true, words },
            ui.column { w = 60, ui.text { id = "fit", w = "fit-content", wrap = true, words } },
            ui.row { ui.text { id = "auto", words } },
        }"##,
    );
    let (line, word, auto) = (
        width(&mut h, "line"),
        width(&mut h, "word"),
        width(&mut h, "auto"),
    );
    assert_eq!(width(&mut h, "stretched"), 1000.);
    assert_eq!(line, auto, "its line, as a row lays it out");
    assert!(word < line && word > 0., "{word} {line}");
    let fit = width(&mut h, "fit");
    assert_eq!(fit, word.max(60.), "the room, or its longest word if wider");
    assert_eq!(h.error(), None);
}

#[test]
fn not_on_the_spacing() {
    let error = zgx_script::testing::TestApp::try_new(|runtime| {
        zgx_script::ScriptHost::inline(runtime, "app", r#"return ui.div { p = "min-content" }"#)
    })
    .err()
    .expect("an error");
    assert!(error.contains("size w, h"), "{error}");
}

/// A menu takes its widest row's width at `w = "max-content"`, as a context
/// menu sized to its labels does, above its `min_w`.
#[test]
fn a_menu_is_as_wide_as_its_widest_row() {
    let mut h = Harness::inline(
        r##"local open = ui.state("open", true)
        return ui.column { w = 800, h = 600,
            ui.menu { id = "menu", open = open, w = "max-content", min_w = 140, p = 0,
                anchor = ui.button { "Open" },
                ui.menu_item { "Short" },
                ui.div { w = 300, h = 4 },
            },
            ui.popover { id = "pop", open = ui.state("pop", true), w = "max-content", min_w = 140, p = 0,
                anchor = ui.button { "Pop" },
                ui.div { w = 300, h = 4 },
            },
        }"##,
    );
    let pop = width(&mut h, "pop");
    let menu = width(&mut h, "menu");
    assert_eq!(h.error(), None);
    assert_eq!((menu, pop), (300., 300.));
}

/// `ui.menu_look` styles the menus the app doesn't style itself, zgx's own
/// among them: an item takes its row height, and one with a height of its
/// own keeps it.
#[test]
fn a_menu_look_styles_items_that_do_not_style_themselves() {
    let mut h = Harness::inline(
        r##"ui.menu_look { panel = { p = 0 }, item = { h = 24, px = 12 } }
        return ui.column { w = 800, h = 600,
            ui.menu { id = "menu", open = ui.state("open", true), anchor = ui.button { "Open" },
                ui.menu_item { id = "plain", "Open Link" },
                ui.menu_item { id = "own", h = 40, "Copy Link" },
            },
        }"##,
    );
    h.frames(2);
    let height = |h: &mut Harness, id: &str| {
        let node = h.find(id).unwrap();
        h.ui.scene.borrow().bounds(node).height
    };
    let (plain, own) = (height(&mut h, "plain"), height(&mut h, "own"));
    zgx::compose::set_menu_look(Default::default());
    assert_eq!(h.error(), None);
    assert_eq!((plain, own), (24., 40.));
}

/// With `ui.menu_look { fade }`, a menu fades out as Escape closes it (still
/// drawn just after, gone once the fade ends) and goes at once when an item
/// is chosen, as a web page's menus do.
#[test]
fn a_menu_fades_out_unless_an_item_was_chosen() {
    let mut h = Harness::inline(
        r##"ui.menu_look { fade = 100 }
        local open = ui.state("open", true)
        return ui.column { p = 20,
            ui.menu { open = open, w = 160, anchor = ui.button { id = "anchor", w = 40, h = 20, "a" },
                ui.menu_item { id = "go", "Go" } } }"##,
    );
    let drawn = |h: &mut Harness| {
        h.ui.prepare_frame();
        h.find("go").is_some_and(|n| {
            h.ui.scene
                .borrow()
                .visible_bounds(n)
                .is_some_and(|b| b.width > 0.)
        })
    };
    h.frames(3);
    std::thread::sleep(std::time::Duration::from_millis(130));
    h.frames(3);
    assert!(drawn(&mut h), "open");
    h.key(zgx::input::Key::Escape);
    h.frames(1);
    assert!(drawn(&mut h), "fading out after Escape");
    std::thread::sleep(std::time::Duration::from_millis(150));
    h.frames(3);
    assert!(!drawn(&mut h), "gone once faded");
    h.host.set_state("open", zgx_script::Value::Bool(true)).unwrap();
    h.frames(3);
    std::thread::sleep(std::time::Duration::from_millis(130));
    h.frames(3);
    assert!(drawn(&mut h), "open again");
    h.click("go");
    h.frames(2);
    zgx::compose::set_menu_look(Default::default());
    assert!(!drawn(&mut h), "a chosen item closes it at once");
}
