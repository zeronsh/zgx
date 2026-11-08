//! A long Markdown reply streaming in: 400 sections appended a chunk at a
//! time to a state, a computed parsing it with `ui.markdown { key }` and a
//! `ui.each` drawing its blocks (the docs' own way, text.md), each chunk
//! laid out and flushed, undrawn. What the last chunks of a long reply cost
//! a frame is what's guarded (`md-stream`).
//! Run: cargo run -p zgx-script --release --example md_stream_bench
//! (`SECTIONS` sets the reply's sections, 400 by default.)
#[path = "support/bench_limits.rs"]
mod bench_limits;
use zgx::widgets::Ui;
use zgx_script::ScriptHost;

const SCRIPT: &str = r###"
local reply = ui.state("reply", "")
local chunks = ui.state("chunks", 0)

local function runs(list: { MarkdownRun }): Node
	local props: RichProps = { wrap = true }
	for _, r in list do
		table.insert(props, ui.span { r.x, literal = true, weight = if r.b then 600 else nil })
	end
	return ui.rich(props)
end

local blocks = ui.computed(function()
	return ui.markdown(reply:get(), { key = "reply" })
end)

ui.command { id = "append", run = function()
	local n = chunks:get() + 1
	chunks:set(n)
	local piece = if n % 3 == 1 then "## Section " .. ((n + 2) // 3) .. "\n\n"
		elseif n % 3 == 2 then "Words arriving as a reply streams, with **bold** and `code` in them, "
		else "and the rest of the paragraph after it.\n\n"
	reply:append(piece)
end }

return ui.column {
	gap = 8,
	w = 600,
	ui.each(blocks, function(b: MarkdownBlock)
		if b.t == "p" or b.t == "h" then
			return runs(b.s)
		end
		return ui.div {}
	end),
}
"###;

fn main() {
    let sections: usize = std::env::var("SECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let mut ui = Ui::new(640., 900.);
    let host = ScriptHost::inline(&ui.runtime, "md_stream", SCRIPT).limits(bench_limits::limits());
    host.reload().expect("the bench script loads");
    let _handle = ui.mount(host.view());
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    for _ in 0..sections * 3 {
        assert!(host.run_command("append"));
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
    println!(
        "{} chunks, scene nodes {}",
        sections * 3,
        ui.scene.borrow().len()
    );
}
