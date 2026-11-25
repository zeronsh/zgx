//! A 300-line note in Luau with a code block (`highlight`, `line_insets`,
//! `line_marks`), as a composer's. `SCENE=letters` (the default) types in its
//! prose and in its block, scrolls the block sideways and the note down, no
//! edit changing a fence (`note-letters`); `SCENE=fence` types a fence at its
//! top and undoes it, over and over (`note-fence`), which puts every line
//! below in a block and back. Laid out and flushed, undrawn. The note's text
//! and where in it to type are the bench's own and left uncounted
//! (`gate_marks`).
//! Run: cargo run -p zgx-script --release --example note_bench
//! (`REPS` sets the letters, or the fences, 200 and 20 by default.)
#[path = "support/bench_limits.rs"]
mod bench_limits;
#[path = "support/gate_marks.rs"]
mod gate_marks;
use zgx::{input::InputEvent, widgets::Ui};
use zgx_script::{ScriptHost, Value};

const SCRIPT: &str = r###"
local text = ui.state("text", "")
return ui.textarea {
	id = "note",
	value = text,
	w = 400,
	h = 600,
	p = 0,
	text_size = 14,
	line_height = 20,
	wrap = true,
	group_scrollbar = true,
	highlight = function(line, state)
		if line == "```" then
			return {}, if state == "code" then 0 else "code"
		end
		return {}, state
	end,
	line_insets = function(line, state)
		if state == "code" and line ~= "```" then
			return { left = 12, group = 1, nowrap = true }
		end
		return nil
	end,
	line_marks = function(line, state)
		if state == "code" and line ~= "```" then
			return { { w = "fill", h = "fill", fill = "#eeeeee", pinned = true } }
		end
		return nil
	end,
}
"###;

/// An intro, an empty line, 100 of prose, 40 of code in a fence, 160 of prose.
fn text() -> String {
    let mut lines = vec!["intro".to_string(), String::new()];
    for i in 0..100 {
        lines.push(format!(
            "line {i} of a note, with words enough to fill a row"
        ));
    }
    lines.push("```".into());
    for i in 0..40 {
        lines.push(format!(
            "local value{i} = compute({i}, 'a line of code wider than the note')"
        ));
    }
    lines.push("```".into());
    for i in 0..160 {
        lines.push(format!("line {i} after the block, with words for a row"));
    }
    lines.join("\n")
}

fn frame(ui: &Ui) {
    ui.prepare_frame();
    let flushed = ui.scene.borrow_mut().flush();
    if zgx_fingerprint::on() {
        zgx_fingerprint::frame(&ui.scene.borrow(), &flushed);
    }
}

fn main() {
    let fence = std::env::var("SCENE").is_ok_and(|scene| scene == "fence");
    let reps: usize = std::env::var("REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if fence { 20 } else { 200 });
    let (text, block) = gate_marks::outside(|| {
        let text = text();
        let block = text.find("value7 ").expect("its block");
        (text, block)
    });
    let mut ui = Ui::new(640., 700.);
    let host = ScriptHost::inline(&ui.runtime, "note", SCRIPT).limits(bench_limits::limits());
    host.reload().expect("the bench script loads");
    host.set_state("text", Value::from(text.as_str()))
        .expect("its text set");
    let handle = ui.mount(host.view());
    frame(&ui);
    let node = handle.find("note").expect("the note");
    let editor = ui.editor(node).expect("its editor");
    ui.input.focus(&ui.scene, Some(node));
    if fence {
        for _ in 0..reps {
            editor.set_selection(6, 6);
            for _ in 0..3 {
                ui.dispatch(InputEvent::Text("`".into()));
                frame(&ui);
            }
            while editor.editor.borrow().len() > text.len() && editor.undo() {
                frame(&ui);
            }
        }
        assert_eq!(editor.editor.borrow().len(), text.len());
        return;
    }
    editor.set_selection(5, 5);
    frame(&ui);
    for _ in 0..reps {
        ui.dispatch(InputEvent::Text("a".into()));
        frame(&ui);
    }
    editor.set_selection(block + reps, block + reps);
    frame(&ui);
    for _ in 0..reps {
        ui.dispatch(InputEvent::Text("b".into()));
        frame(&ui);
    }
    for i in 0..100 {
        editor.set_group_scroll(1, (i % 50) as f32 * 4.);
        frame(&ui);
    }
    for i in 0..100 {
        editor.set_scroll_offset(i as f32 * 20.);
        frame(&ui);
    }
}
