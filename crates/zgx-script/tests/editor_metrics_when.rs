//! What `el:text_metrics(width)` answers where a script may ask it around an
//! edit, at the one width it always asks: an edit's text at once, a value the
//! app sets once the field has taken it.
mod common;
use common::Harness;
use zgx::input::InputEvent;
use zgx_script::Value;

const CODE: &str = r##"
    local ref = ui.ref()
    local text = ui.state("text", "one\ntwo")
    local rows = ui.state("rows", {})
    local log = {}
    local function ask(where)
        local el = ref:get()
        local m = if el then el:text_metrics(200) else nil
        table.insert(log, where .. " got " .. (if m then tostring(m.wrapped_height) else "nil"))
    end
    local runs = 0
    ui.effect(function()
        text:get()
        runs += 1
        ask("effect run " .. runs)
    end)
    ui.command { id = "ask", run = function() ask("asked") end }
    ui.command { id = "set", run = function()
        text:set("one\ntwo\nthree\nfour")
        ask("set, same function")
    end }
    ui.command { id = "set_after", run = function()
        text:set("one\ntwo\nthree\nfour")
        ui.after(0, function() ask("set, after(0)") end)
    end }
    ui.command { id = "read", run = function() rows:set(table.clone(log)) end }
    ui.command { id = "clear", run = function() table.clear(log); rows:set({}); runs = 0 end }
    return ui.textarea { id = "note", ref = ref, VALUE
        w = 200, h = 100, p = 0, text_size = 10, line_height = 20, wrap = true }
"##;

/// `1`: controlled (`value` a function).
struct Page(Harness, bool);
impl Page {
    fn new(controlled: bool) -> Self {
        let value = match controlled {
            true => {
                "value = function() return text:get() end, on_change = function(t) text:set(t); ask(\"on_change\") end,"
            }
            false => "value = text,",
        };
        let mut h = Harness::inline(&CODE.replace("VALUE", value));
        h.frames(3);
        assert_eq!(h.error(), None);
        let node = h.find("note").unwrap();
        let editor = h.ui.editor(node).unwrap();
        editor.set_selection(3, 3);
        h.ui.input.focus(&h.ui.scene, Some(node));
        h.frames(3);
        // Asked once before.
        assert!(h.host.run_command("ask"));
        let mut page = Self(h, controlled);
        page.clear();
        page
    }
    fn clear(&mut self) {
        assert!(self.0.host.run_command("clear"));
        self.0.frames(2);
        assert!(self.0.host.run_command("clear"));
    }
    fn rows(&self) -> Vec<String> {
        assert!(self.0.host.run_command("read"));
        let states = self.0.host.states();
        match states.into_iter().find(|(k, _)| k == "rows").unwrap().1 {
            Value::List(list) => (list.iter())
                .filter_map(|v| match v {
                    Value::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    /// What was answered by `act`, asked once more after it, and how many of
    /// those by the next frame's layout, and then by what came later.
    fn step(&mut self, act: impl FnOnce(&mut Harness)) -> (Vec<String>, usize, usize) {
        self.clear();
        act(&mut self.0);
        assert!(self.0.host.run_command("ask"));
        let turn = self.rows().len();
        self.0.ui.prepare_frame();
        let laid = self.rows().len();
        self.0.frames(3);
        let all = self.rows();
        self.clear();
        (all, turn, laid)
    }
    /// An edit's height, `height`, in every place.
    fn edit(&mut self, what: &str, height: u32, act: impl FnOnce(&mut Harness)) {
        let (rows, turn, laid) = self.step(act);
        let mut want = vec![
            format!("effect run 1 got {height}"),
            format!("asked got {height}"),
            format!("effect run 2 got {height}"),
        ];
        if self.1 {
            want.insert(0, format!("on_change got {height}"));
        }
        assert_eq!(rows, want, "{what}");
        assert_eq!((turn, laid), (want.len() - 1, want.len()), "{what}");
    }
    /// A value the app sets: the effect, made before the field, is told the
    /// text `before`, and again once laid out; `asked` besides it.
    fn set(
        &mut self,
        what: &str,
        (before, height): (u32, u32),
        asked: [&str; 2],
        act: impl FnOnce(&mut Harness),
    ) {
        let (rows, _, _) = self.step(act);
        let mut want = vec![
            format!("effect run 1 got {before}"),
            format!("asked got {height}"),
            format!("effect run 2 got {height}"),
        ];
        if !asked[0].is_empty() {
            want.insert(1, format!("{} got {height}", asked[0]));
        }
        if !asked[1].is_empty() {
            want.push(format!("{} got {height}", asked[1]));
        }
        assert_eq!(rows, want, "{what}");
    }
}
#[test]
fn an_edit_is_measured_in_its_own_turn_and_a_set_value_once_taken() {
    for controlled in [false, true] {
        let c = if controlled { "controlled" } else { "bound" };
        let mut page = Page::new(controlled);
        let editor = |h: &mut Harness| {
            let node = h.find("note").unwrap();
            h.ui.editor(node).unwrap()
        };
        page.edit(&format!("{c}, typed"), 60, |h| {
            h.ui.dispatch(InputEvent::Text("\n".into()));
        });
        page.edit(&format!("{c}, undone"), 40, |h| assert!(editor(h).undo()));
        page.edit(&format!("{c}, redone"), 60, |h| assert!(editor(h).redo()));
        page.edit(&format!("{c}, pasted"), 100, |h| {
            editor(h).paste("a\nb\n");
        });
        page.set(
            &format!("{c}, set"),
            (100, 80),
            ["set, same function", ""],
            |h| {
                assert!(h.host.run_command("set"));
            },
        );
        page.set(&format!("{c}, set by the host"), (80, 20), ["", ""], |h| {
            h.host.set_state("text", Value::from("one")).unwrap();
        });
        page.set(
            &format!("{c}, set, later"),
            (20, 80),
            ["", "set, after(0)"],
            |h| {
                assert!(h.host.run_command("set_after"));
            },
        );
    }
}
