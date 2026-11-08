//! One chunk of a streamed Markdown reply, once the reply has grown to
//! `SECTIONS` sections (100 by default): the reply set whole, a section
//! streamed, then `CHUNKS` chunks (9) appended. `scripts/perf/flat_chunks.py`
//! counts it whole with 9 chunks and with 27, a chunk the difference over
//! 18. A chunk should cost the same at 800 sections as at 100
//! (`tests/md_chunk_flat.rs` splits it by phase).
//! Run: cargo run -p zgx-script --release --example md_chunk_bench
#[path = "support/bench_limits.rs"]
mod bench_limits;
use zgx::widgets::Ui;
use zgx_script::{ScriptHost, Value};

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

/// The script's chunk `n` (from 1), as its `append` makes it.
fn piece(n: usize) -> String {
    match n % 3 {
        1 => format!("## Section {}\n\n", n.div_ceil(3)),
        2 => "Words arriving as a reply streams, with **bold** and `code` in them, ".into(),
        _ => "and the rest of the paragraph after it.\n\n".into(),
    }
}

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The chunks counted: appended, laid out and flushed.
#[inline(never)]
fn measured(host: &ScriptHost, ui: &Ui, chunks: usize) {
    for _ in 0..chunks {
        assert!(host.run_command("append"));
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
}

fn main() {
    let sections = env("SECTIONS", 100);
    let chunks = env("CHUNKS", 9);
    let mut ui = Ui::new(640., 900.);
    let host = ScriptHost::inline(&ui.runtime, "md_chunk", SCRIPT).limits(bench_limits::limits());
    host.reload().expect("the bench script loads");
    let _handle = ui.mount(host.view());
    let text: String = (1..=sections * 3).map(piece).collect();
    host.set_state("reply", Value::from(text.as_str())).unwrap();
    host.set_state("chunks", Value::Num((sections * 3) as f64))
        .unwrap();
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    // A section streamed first, uncounted: the stream's own state made.
    for _ in 0..3 {
        assert!(host.run_command("append"));
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
    measured(&host, &ui, chunks);
    println!("{sections} sections, {chunks} chunks counted");
}
