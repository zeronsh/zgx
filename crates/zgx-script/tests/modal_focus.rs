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

/// Whether focus is on the modal's panel or in it.
fn inside(h: &mut Harness) -> bool {
    use zgx::semantics::Role;
    let Some(focused) = h.ui.input.focused() else {
        return false;
    };
    let Some(title) = h.find("title") else {
        return false;
    };
    let scene = h.ui.scene.borrow();
    let semantics = h.ui.semantics.borrow();
    let Some(dialog) = scene
        .ancestors(title)
        .find(|node| semantics.get(*node).is_some_and(|s| s.role == Role::Dialog))
    else {
        return false;
    };
    focused == dialog || scene.ancestors(focused).any(|node| node == dialog)
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
        inside(&mut h),
        "a click inside moved focus out: {focused:?}"
    );
    h.click("title");
    h.frames(2);
    let focused = h.ui.input.focused();
    assert!(
        inside(&mut h),
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
    assert_eq!(closed(&mut h), "closed=1", "Escape reached on_close");
}

#[test]
fn closing_gives_focus_back_to_what_had_it() {
    let mut h = open();
    h.key(Key::Escape);
    h.frames(3);
    let opener = h.find("opener");
    assert_eq!(h.ui.input.focused(), opener);
}

#[test]
fn a_press_on_the_backdrop_reaches_on_close_once() {
    let mut h = open();
    h.ui.dispatch(zgx::input::InputEvent::PointerDown {
        x: 2.,
        y: 2.,
        button: zgx::input::PointerButton::Primary,
    });
    h.ui.dispatch(zgx::input::InputEvent::PointerUp {
        x: 2.,
        y: 2.,
        button: zgx::input::PointerButton::Primary,
    });
    h.frames(3);
    assert_eq!(closed(&mut h), "closed=1");
}

#[test]
fn a_controlled_open_hears_its_close_once() {
    let mut h = Harness::inline(
        r##"local open = ui.state("open", true)
        local closed = ui.state("closed", 0)
        return ui.column { w_full = true, h_full = true,
            ui.text { id = "closed", function() return `closed={closed:get()}` end },
            ui.modal {
                open = function() return open:get() end,
                on_close = function() closed:set(closed:peek() + 1); open:set(false) end,
                build = function() return ui.button { id = "done", label = "Done" } end,
            },
        }"##,
    );
    h.frames(3);
    assert_eq!(h.error(), None);
    h.key(Key::Escape);
    h.frames(3);
    assert_eq!(h.text("closed"), "closed=1");
}

/// What a ref in a modal's 250-wide `ui.scroll` reads as its width while the
/// modal opens, its first focusable control in the scroll with the ref
/// (`in_scroll`), or before it (nothing to reveal then).
fn scroll_widths(in_scroll: bool) -> String {
    let (before, inside) = match in_scroll {
        true => (
            "",
            "ui.div { id = \"first\", focusable = true, w = 10, h = 10 }",
        ),
        false => (
            "ui.div { id = \"first\", focusable = true, w = 10, h = 10 },",
            "ui.div { w = 10, h = 10 }",
        ),
    };
    let app = r##"
local widths = ui.state("widths", "")
local open = ui.state("open", false)
local function content(): any
    local r = ui.ref()
    ui.effect(function() widths:set(`{widths:peek()} {r.width}`) end)
    return ui.column { ref = r, w_full = true, INSIDE }
end
return ui.column { w_full = true, h_full = true,
    ui.button { id = "opener", label = "Open", on_click = function() open:set(true) end },
    ui.text { id = "widths", function() return widths:get() end },
    ui.modal { open = open, label = "m", w = 300, p = 0, build = function()
        return ui.column { w_full = true, BEFORE ui.scroll { w = 250, h = 100, content() } }
    end },
}
"##
    .replace("INSIDE", inside)
    .replace("BEFORE", before);
    let mut h = Harness::inline(&app);
    h.frames(3);
    h.click("opener");
    h.frames(3);
    assert_eq!(h.error(), None);
    let first = h.find("first");
    assert!(first.is_some(), "the modal opened");
    assert_eq!(h.ui.input.focused(), first);
    h.text("widths")
}

/// A modal's first control in a `ui.scroll` with its own `w`: a ref in it
/// never reads a scroll view's default width (320) on the way, and reads just
/// what it does with the control outside the scroll.
#[test]
fn a_scroll_in_an_opening_modal_is_never_its_default_width() {
    let widths = scroll_widths(true);
    assert!(
        !widths.contains("320") && widths.ends_with(" 250"),
        "{widths}"
    );
    assert_eq!(widths, scroll_widths(false));
}

/// Its first control below the fold is shown as the modal opens.
#[test]
fn a_first_control_below_the_fold_shows_as_the_modal_opens() {
    let mut h = Harness::inline(
        r##"
local open = ui.state("open", false)
local y = ui.state("y", 0)
return ui.column { w_full = true, h_full = true,
    ui.button { id = "opener", label = "Open", on_click = function() open:set(true) end },
    ui.text { id = "y", function() return `{y:get()}` end },
    ui.modal { open = open, label = "m", w = 300, p = 0, build = function()
        return ui.column { w_full = true, ui.scroll { w = 250, h = 100, offset = y,
            ui.column { w_full = true, ui.div { h = 300 }, ui.div { id = "first", focusable = true, w = 10, h = 10 } } } }
    end },
}
"##,
    );
    h.frames(3);
    h.click("opener");
    h.frames(3);
    assert_eq!(h.error(), None);
    assert!(h.find("first").is_some(), "the modal opened");
    assert_eq!(h.text("y"), "210");
}

/// Opened beside a layout that never settles, its reveal waits: no frame is
/// asked for and none changes anything, so the window idles. Once the layout
/// settles (mended by a hover, which moves no focus), the control shows.
#[test]
fn a_reveal_waiting_on_a_layout_that_never_settles_keeps_no_window_awake() {
    let mut h = Harness::inline(
        r##"
local open = ui.state("open", false)
local broken = ui.state("broken", true)
local y = ui.state("y", 0)
local r = ui.ref()
return ui.column { w_full = true, h_full = true,
    ui.button { id = "opener", label = "Open", on_click = function() open:set(true) end },
    ui.text { id = "y", function() return `{y:get()}` end },
    ui.modal { open = open, label = "m", w = 300, p = 0, build = function()
        return ui.column { w_full = true,
            ui.div { ref = r, h = 5, w = function()
                if not broken:get() then return 50 end
                return (r.width or 100) + 1
            end },
            ui.div { id = "mend", w = 40, h = 20, on_hover = function(inside) if inside then broken:set(false) end end },
            ui.scroll { w = 250, h = 100, offset = y,
                ui.column { w_full = true, ui.div { h = 300 }, ui.div { id = "first", focusable = true, w = 10, h = 10 } } } }
    end },
}
"##,
    );
    h.frames(3);
    h.click("opener");
    // Any transition of its opening over.
    h.frames(60);
    let error = h.error().unwrap_or_default();
    assert!(error.contains("did not settle"), "{error:?}");
    let first = h.find("first");
    assert!(first.is_some() && h.ui.input.focused() == first);
    // Still waiting: nothing runs it while the layout is held.
    assert_eq!(h.text("y"), "0", "revealed while the layout was held");
    let (offset, at) = (h.text("y"), h.bounds("first"));
    h.ui.scene.borrow_mut().flush();
    for frame in 0..5 {
        assert!(!h.wants_frame(), "frame {frame} asked for another");
        assert!(!h.ui.spare_work_wanted());
        h.ui.try_prepare_frame().unwrap();
        assert!(
            h.ui.scene.borrow_mut().flush().damage.is_empty(),
            "frame {frame} drew"
        );
    }
    assert_eq!((h.text("y"), h.bounds("first")), (offset, at));
    // Mended by a hover, which moves no focus: the reveal goes on.
    h.hover("mend");
    h.frames(3);
    assert_eq!(h.ui.input.focused(), first, "focus stayed");
    assert_eq!(h.text("y"), "210");
}
