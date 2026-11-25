//! A composer that grows with its text: a computed reads the text's length
//! and measures it (`el:text_metrics(width)`), an effect on it publishes the
//! height (at once where the width is the same, else asked again a moment
//! later), and the area is as high, up to a cap. The first frame after an
//! edit paints the area as high as its text asks, scrolled where it stays.
mod common;
use common::Harness;
use zgx::{
    input::{InputEvent, Key, Modifiers},
    scene::NodeId,
};
use zgx_script::Value;

const CODE: &str = r##"
    local ref = ui.ref()
    local text = ui.state("text", "TEXT")
    local h = ui.state("h", 56)
    local width = ui.state("width", 300)
    local runs = ui.state("runs", {})
    local log = {}
    local function note(line)
        table.insert(log, line)
        runs:set(table.clone(log))
    end
    local published
    local function publish(m)
        published = m.width
        h:set(math.clamp(m.wrapped_height, 56, 240))
    end
    local shape = ui.computed(function()
        text:len()
        local el = ref:get()
        return if el then el:text_metrics(width:get()) else nil
    end)
    ui.effect(function()
        local m = shape:get()
        if not m then return end
        note((if published == m.width then "at once " else "later ") .. m.wrapped_height)
        if published == m.width then
            publish(m)
        else
            ui.after(0, function()
                local el = ref:get()
                local again = if el then el:text_metrics(width:peek()) else nil
                if again and again.revision == m.revision then
                    publish(again)
                end
            end)
        end
    end)
    ui.command { id = "set", run = function() text:set("one\ntwo\nthree\nfour\nfive\nsix\nseven") end }
    ui.command { id = "narrow", run = function() width:set(240) end }
    ui.command { id = "clear", run = function() table.clear(log); runs:set({}) end }
    return ui.column {
        w = 400, h = 400,
        ui.textarea { id = "note", ref = ref, value = text,
            w = width, h = h, p = 0, text_size = 10, line_height = 20, wrap = true,
            EXTRA },
    }
"##;
const FENCES: &str = r##"
            highlight = function(line, state)
                if line == "```" then
                    return {}, if state == "code" then 0 else "code"
                end
                return {}, state
            end,
            line_insets = function(line, state)
                if state == "code" and line ~= "```" then
                    return { left = 12, above = 2, below = 1 BLOCK }
                end
                return nil
            end"##;

struct Composer {
    h: Harness,
    node: NodeId,
}
fn composer(text: &str, extra: &str) -> Composer {
    let code = CODE
        .replace("TEXT", &text.replace('\n', "\\n"))
        .replace("EXTRA", extra);
    let mut h = Harness::inline(&code);
    h.frames(4);
    assert_eq!(h.error(), None);
    let node = h.find("note").expect("the note");
    let editor = h.ui.editor(node).unwrap();
    editor.set_selection(0, 0);
    h.ui.input.focus(&h.ui.scene, Some(node));
    h.frames(3);
    assert!(h.host.run_command("clear"));
    h.frames(2);
    Composer { h, node }
}
impl Composer {
    fn runs(&self) -> Vec<String> {
        let states = self.h.host.states();
        let value = states.into_iter().find(|(k, _)| k == "runs").unwrap().1;
        match value {
            Value::List(list) => (list.iter()).map(|v| format!("{v:?}")).collect(),
            _ => Vec::new(),
        }
    }
    /// What the frame paints: how high the area's box is in what a renderer
    /// is given, and how far its text is scrolled.
    fn painted(&self) -> (f32, f32) {
        let scene = self.h.ui.scene.borrow();
        let height = scene.transformed_bounds(self.node).height;
        let editor = self.h.ui.editor(self.node).unwrap();
        (height, editor.scroll_offset().1)
    }
    /// `height`: what its text asks, up to the cap.
    fn check(&mut self, failed: &mut Vec<String>, what: &str, height: f32) {
        let in_turn = self.runs();
        // One frame as a host draws it: laid out once, flushed, painted.
        self.h.ui.prepare_frame();
        self.h.ui.scene.borrow_mut().flush();
        let first = self.painted();
        let after = self.runs();
        self.h.ui.prepare_frame();
        self.h.ui.scene.borrow_mut().flush();
        let second = self.painted();
        self.h.frames(4);
        let want = self.painted();
        let ok = first == want && first.0 == height;
        println!(
            "{} {what}: runs in the turn {in_turn:?}, by the first frame {after:?}; first frame paints (height, scroll) {first:?}, second {second:?}, settled {want:?}",
            if ok { "PASS" } else { "FAIL" }
        );
        if !ok {
            failed.push(format!(
                "{what}: first frame paints {first:?}, settled {want:?}, its text asks {height}"
            ));
        }
        assert!(self.h.host.run_command("clear"));
        self.h.frames(2);
    }
}
impl Composer {
    /// A height asked again a moment later (`ui.after(0)`: a new width, a
    /// theme): what a frame with no timer run paints, and the next, its
    /// timers run before it as a host's loop runs them.
    fn check_later(&mut self, failed: &mut Vec<String>, what: &str, height: f32) {
        let in_turn = self.runs();
        self.h.ui.prepare_frame();
        let early = self.painted();
        let after = self.runs();
        self.h.frames(1);
        let first = self.painted();
        self.h.frames(4);
        let want = self.painted();
        let ok = first == want && want.0 == height;
        println!(
            "{} {what}: runs in the turn {in_turn:?}, then {after:?}; painted before its timer {early:?}, by the frame after it {first:?}, settled {want:?}",
            if ok { "PASS" } else { "FAIL" }
        );
        if !ok {
            failed.push(format!(
                "{what}: painted {first:?}, settled {want:?}, its text asks {height}"
            ));
        }
        assert!(self.h.host.run_command("clear"));
        self.h.frames(2);
    }
}
fn key(h: &mut Harness, key: Key) {
    h.ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
}
/// A row of 300 points holds 50 letters of the test shaper's.
fn start() -> String {
    format!("intro\n\n{}\nend", "x".repeat(49))
}
/// `fenced`: how high the note is with its fence.
fn cases(extra: &str, fenced: f32) {
    let mut failed = Vec::new();
    let mut c = composer(&start(), extra);
    let editor = c.h.ui.editor(c.node).unwrap();
    let wide = start().find("xxx").unwrap() + 49;
    editor.set_selection(wide, wide);
    c.h.ui.dispatch(InputEvent::Text("yy".into()));
    c.check(&mut failed, "a letter that wraps", 100.);
    editor.undo();
    c.check(&mut failed, "undone", 80.);
    editor.redo();
    c.check(&mut failed, "redone", 100.);
    key(&mut c.h, Key::Backspace);
    key(&mut c.h, Key::Backspace);
    c.check(&mut failed, "deleted", 80.);
    editor.set_selection(5, 5);
    c.h.ui.dispatch(InputEvent::Text("\n".into()));
    c.check(&mut failed, "a line break", 100.);
    editor.set_selection(2, 2);
    editor.paste(&"pasted line\n".repeat(4));
    c.check(&mut failed, "a paste", 180.);
    editor.undo();
    c.check(&mut failed, "the paste undone", 100.);
    editor.paste(&"pasted line\n".repeat(40));
    c.check(&mut failed, "a paste past the cap", 240.);
    editor.undo();
    c.check(&mut failed, "that undone", 100.);
    let mut c = composer(&start(), extra);
    let editor = c.h.ui.editor(c.node).unwrap();
    editor.set_selection(6, 6);
    c.h.ui.dispatch(InputEvent::Text("``".into()));
    c.check(&mut failed, "two backticks", 80.);
    c.h.ui.dispatch(InputEvent::Text("`".into()));
    c.check(&mut failed, "the fence", fenced);
    editor.undo();
    c.check(&mut failed, "the fence undone", 80.);
    editor.redo();
    c.check(&mut failed, "the fence redone", fenced);
    editor.set_selection(6, 9);
    key(&mut c.h, Key::Backspace);
    c.check(&mut failed, "the fence deleted", 80.);
    assert!(c.h.host.run_command("set"));
    c.check(&mut failed, "set by the app", 140.);
    // Asked again a moment later: a new width, then a letter that wraps at
    // it in the same turn as another new one, and a theme.
    let mut c = composer(&start(), extra);
    let editor = c.h.ui.editor(c.node).unwrap();
    assert!(c.h.host.run_command("narrow"));
    c.check_later(&mut failed, "narrower", 100.);
    editor.set_selection(5, 5);
    c.h.ui.dispatch(InputEvent::Text("\n".into()));
    c.check(&mut failed, "a line break at it", 120.);
    let mut c = composer(&start(), extra);
    let editor = c.h.ui.editor(c.node).unwrap();
    assert!(c.h.host.run_command("narrow"));
    editor.set_selection(5, 5);
    c.h.ui.dispatch(InputEvent::Text("\n".into()));
    c.check_later(&mut failed, "narrower and a line break in one turn", 120.);
    let theme = zgx::theme::Theme::new("b", zgx::theme::ThemeKind::Dark).with(
        zgx::theme::tok::TEXT_PRIMARY,
        zgx::scene::Color(0, 255, 0, 255),
    );
    c.h.ui.theme().register(theme);
    c.h.ui.theme().set("b").unwrap();
    c.check_later(&mut failed, "a theme", 120.);
    assert!(
        failed.is_empty(),
        "{} failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
}
#[test]
fn a_composers_first_frame_is_as_high_as_its_text_plain() {
    cases("", 80.);
}
#[test]
fn a_composers_first_frame_is_as_high_as_its_text_in_blocks() {
    cases(&FENCES.replace("BLOCK", ", group = 1, nowrap = true"), 86.);
}
#[test]
fn a_composers_first_frame_is_as_high_as_its_text_in_wrapped_code() {
    cases(&FENCES.replace("BLOCK", ""), 106.);
}
