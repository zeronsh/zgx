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

/// `ui.find_look` gives the find bar zgx builds the app's look: its field
/// takes the height, and Enter steps only once the navigator shows.
#[test]
fn a_find_look_styles_the_find_bar() {
    let mut h = Harness::inline(
        r##"ui.find_look {
            panel = { rounded = 10 }, field = { h = 28 }, short_count = true, navigator_after = 3,
            placeholder = "Search",
            icons = { search = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'><circle cx='8' cy='8' r='5'/></svg>" },
        }
        return ui.column { w = 800, h = 600, ui.text { "apple apple apple" } }"##,
    );
    h.frames(2);
    assert!(h.ui.open_find());
    h.frames(2);
    let field = h.ui.input.focused().unwrap();
    let height = h.ui.scene.borrow().bounds(field).height;
    let search = |h: &mut Harness, text: &str| {
        h.keys("cmd+a");
        h.key(zgx::input::Key::Backspace);
        h.ui.dispatch(zgx::input::InputEvent::Text(text.into()));
        h.frames(3);
        h.key(zgx::input::Key::Enter);
        h.frames(3);
        h.ui.find_status().current
    };
    let short = search(&mut h, "ap");
    let long = search(&mut h, "apple");
    zgx::find::set_find_look(Default::default());
    assert_eq!(h.error(), None);
    assert_eq!(height, 28.);
    assert_eq!((short, long), (Some(0), Some(1)));
}

/// `ui.tooltip_look { keep_below }`: a tip that would open above a node in
/// that top band (a titlebar's) flips below it, and takes the look's panel.
#[test]
fn a_tooltip_look_keeps_tips_below_the_titlebar_band() {
    let mut h = Harness::inline(
        r##"ui.tooltip_look { panel = { px = 8, py = 4, rounded = 10 }, placement = { side = "top", align = "center", offset = 4 }, keep_below = 40 }
        return ui.column { w = 600, h = 400,
            ui.div { h = 10 },
            ui.button { id = "badge", w = 40, h = 20, tooltip = "3 files in library", tooltip_delay = 0, "3" },
        }"##,
    );
    h.frames(2);
    h.hover("badge");
    h.frames(6);
    let tip = h.target("\"3 files in library\"").ok();
    let at = tip.map(|t| h.ui.scene.borrow().bounds(t));
    let badge = h.bounds("badge");
    zgx::compose::set_tooltip_look(Default::default());
    assert_eq!(h.error(), None);
    let at = at.expect("the tip shows");
    assert!(at.y >= badge.y + badge.height, "below the badge: {at:?} {badge:?}");
}

/// `ui.modal { scrim_blur }` blurs what lies behind its backdrop.
#[test]
fn a_modal_scrim_blur_blurs_behind_the_backdrop() {
    let h = Harness::inline(
        r##"return ui.column { w = 400, h = 300,
            ui.modal { open = ui.state("open", true), scrim_blur = 2, ui.text { "Picture" } },
        }"##,
    );
    h.frames(3);
    assert_eq!(h.error(), None);
    assert!(h.ui.scene.borrow().has_backdrop_blur());
}
