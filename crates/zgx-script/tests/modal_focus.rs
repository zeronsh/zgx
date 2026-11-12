//! A `ui.modal` holds keyboard focus as a web dialog does: focus moves into
//! it as it opens, Tab and Shift-Tab cycle within it, a click inside never
//! leaves focus outside, Escape reaches `on_close`, and focus goes back to
//! what had it as it closes.
mod common;
use common::Harness;
use zgx::input::Key;

const APP: &str = r##"
local open = ui.state("open", false)
local closed = ui.state("closed", 0)
return ui.row { w_full = true, h_full = true, gap = 8,
    ui.column { id = "sidebar", gap = 4,
        ui.button { id = "side1", label = "Side 1" },
        ui.button { id = "side2", label = "Side 2" },
    },
    ui.button { id = "opener", label = "Open", on_click = function() open:set(true) end },
    ui.text { id = "closed", function() return `closed={closed:get()}` end },
    ui.modal {
        open = open,
        on_close = function() closed:set(closed:peek() + 1) end,
        dismiss_on_backdrop = true,
        placement = { x = "center", y = "center" },
        w = 300, relative = true, p = 0, overflow = "hidden",
        build = function()
            return ui.column { w_full = true, gap = 8, p = 12,
                ui.text { id = "title", "Settings" },
                ui.row { gap = 4,
                    ui.button { id = "tab1", label = "One" },
                    ui.button { id = "tab2", label = "Two" },
                },
                ui.div { id = "blank", w_full = true, h = 60 },
                ui.button { id = "done", label = "Done", on_click = function() open:set(false) end },
            }
        end,
    },
}
"##;

const INSIDE: [&str; 3] = ["tab1", "tab2", "done"];

fn open() -> Harness {
    let mut h = Harness::inline(APP);
    h.frames(3);
    assert_eq!(h.error(), None);
    let opener = h.find("opener");
    h.ui.input.focus(&h.ui.scene, opener);
    h.click("opener");
    h.frames(3);
    h
}

fn inside(h: &mut Harness) -> bool {
    let focused = h.ui.input.focused();
    INSIDE
        .iter()
        .any(|id| focused.is_some() && h.find(id) == focused)
}

fn closed(h: &mut Harness) -> String {
    h.text("closed")
}

#[test]
fn focus_moves_into_the_modal_as_it_opens() {
    let mut h = open();
    assert!(inside(&mut h), "focused {:?}", h.ui.input.focused());
}

#[test]
fn tab_and_shift_tab_cycle_within_the_modal() {
    let mut h = open();
    for _ in 0..7 {
        h.key(Key::Tab);
        assert!(
            inside(&mut h),
            "Tab left the modal: {:?}",
            h.ui.input.focused()
        );
    }
    for _ in 0..7 {
        h.keys("shift+tab");
        assert!(
            inside(&mut h),
            "Shift-Tab left the modal: {:?}",
            h.ui.input.focused()
        );
    }
}

#[test]
fn a_click_on_blank_space_inside_never_leaves_focus_outside() {
    let mut h = open();
    h.click("blank");
    h.frames(2);
    let focused = h.ui.input.focused();
    assert!(
        focused.is_none() || inside(&mut h),
        "a click inside moved focus out: {focused:?}"
    );
    h.click("title");
    h.frames(2);
    let focused = h.ui.input.focused();
    assert!(
        focused.is_none() || inside(&mut h),
        "a click on the title moved focus out: {focused:?}"
    );
    // And Tab and Escape still work from there.
    h.key(Key::Tab);
    assert!(
        inside(&mut h),
        "Tab after a click: {:?}",
        h.ui.input.focused()
    );
}

#[test]
fn escape_after_a_click_inside_reaches_on_close() {
    let mut h = open();
    h.click("blank");
    h.frames(2);
    assert_eq!(closed(&mut h), "closed=0");
    h.key(Key::Escape);
    h.frames(3);
    assert_eq!(closed(&mut h), "closed=1");
    assert!(h.find("done").is_none(), "the modal closed");
}

#[test]
fn closing_gives_focus_back_to_what_had_it() {
    let mut h = open();
    h.key(Key::Escape);
    h.frames(3);
    let opener = h.find("opener");
    assert_eq!(h.ui.input.focused(), opener);
}
