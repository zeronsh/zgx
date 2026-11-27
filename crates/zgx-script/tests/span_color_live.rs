//! A span's colour that a function or state says (`color = function() … end`):
//! read on its own, so a change recolours the paragraph, shaped as it was,
//! rather than make its spans again (`TextSpan::text_color_live`): an app
//! fading a streamed reply's words in from script pays for the colours
//! alone, not a paragraph's lowering a frame.

use std::{cell::Cell, rc::Rc};

use zgx::scene::{Color, NodeKind};
use zgx_script::{ScriptHost, Value, testing::TestApp};

const SCRIPT: &str = r##"
local tick = ui.state("tick", 0)
ui.command { id = "tick", run = function()
	tick:set(tick:peek() + 1)
end }
return ui.rich {
	id = "reply",
	ui.span { function()
		ui.call_sync("made", nil)
		return "Settled words, "
	end },
	ui.span { "fading in.", color = function()
		return if tick:get() % 2 == 0 then "#ff0000" else "#0000ff"
	end },
}
"##;

/// The colours of the runs of the paragraph under `reply`.
fn colours(app: &mut TestApp) -> Vec<Color> {
    let node = app.find("#reply").expect("the paragraph");
    let scene = app.ui.scene.borrow();
    let mut stack = vec![node];
    while let Some(id) = stack.pop() {
        if let NodeKind::RichText { text } = scene.kind(id) {
            return text.runs().iter().map(|run| run.color).collect();
        }
        stack.extend(scene.children(id).iter().rev().copied());
    }
    panic!("no rich text under #reply");
}

#[test]
fn a_colour_function_recolours_the_paragraph_without_making_its_spans_again() {
    let made = Rc::new(Cell::new(0));
    let counted = made.clone();
    let mut app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", SCRIPT);
        host.function("made", move |_| {
            counted.set(counted.get() + 1);
            Value::Nil
        });
        host
    });
    app.frames(1);
    assert_eq!(app.error(), None);
    assert_eq!(colours(&mut app)[1], Color(255, 0, 0, 255));
    let before = made.get();
    for (step, want) in [
        Color(0, 0, 255, 255),
        Color(255, 0, 0, 255),
        Color(0, 0, 255, 255),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(app.host.run_command("tick"));
        app.frames(1);
        assert_eq!(colours(&mut app)[1], want, "step {step}");
    }
    assert_eq!(app.error(), None);
    assert_eq!(
        made.get(),
        before,
        "the paragraph's spans weren't made again"
    );
}

const REBUILT: &str = r##"
local on = ui.state("on", false)
ui.command { id = "flip", run = function()
	on:set(not on:peek())
end }
return ui.rich {
	id = "reply",
	ui.span { "Row ", bold = on },
	ui.span { "coloured", color = function()
		ui.call_sync("read", nil)
		return if on:get() then "#0000ff" else "#ff0000"
	end },
}
"##;

/// A paragraph made again (its bold follows the state) as its colour
/// function's state changes too: the colour is right, and the function runs
/// once a change, as it did when the paragraph read it itself (not again for
/// an effect made anew each time).
#[test]
fn a_paragraph_made_again_reads_its_colour_function_once_a_change() {
    let reads = Rc::new(Cell::new(0));
    let counted = reads.clone();
    let mut app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", REBUILT);
        host.function("read", move |_| {
            counted.set(counted.get() + 1);
            Value::Nil
        });
        host
    });
    app.frames(1);
    assert_eq!(app.error(), None);
    assert_eq!(colours(&mut app)[1], Color(255, 0, 0, 255));
    let before = reads.get();
    for (step, want) in [
        Color(0, 0, 255, 255),
        Color(255, 0, 0, 255),
        Color(0, 0, 255, 255),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(app.host.run_command("flip"));
        app.frames(1);
        assert_eq!(colours(&mut app)[1], want, "step {step}");
        assert_eq!(
            reads.get(),
            before + step + 1,
            "step {step}: read once a change"
        );
    }
    assert_eq!(app.error(), None);
}
