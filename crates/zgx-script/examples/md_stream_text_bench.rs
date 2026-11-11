//! A Markdown reply streaming in, measured as a renderer measures it: its
//! paragraphs shaped by zgx-gpu's text cache (`TextServices`, on the CPU, no
//! GPU) in the bundled font, as a reply drawn in a window is. Each section is
//! a heading and a long paragraph that grows a chunk at a time, as a reply's
//! text arrives, laid out and flushed after each, undrawn. What a growing
//! paragraph's chunks cost the text cache is what's guarded
//! (`md-stream-text`); `md_stream_bench` measures the same reply's script and
//! layout with zgx's estimate of text.
//! Run: cargo run -p zgx-script --release --example md_stream_text_bench
//! (`SECTIONS` sets the reply's sections, 12 by default; `CHUNKS` each
//! paragraph's chunks, 40.)
#[path = "support/bench_limits.rs"]
mod bench_limits;
use zgx::widgets::Ui;
use zgx_script::ScriptHost;

const SCRIPT: &str = r###"
local reply = ui.state("reply", "")
local chunks = ui.state("chunks", 0)
local per = PER

local function runs(list: { MarkdownRun }): Node
	local props: RichProps = { wrap = true }
	for _, r in list do
		table.insert(props, ui.span { r.x, literal = true, weight = if r.b then 600 else nil })
	end
	return ui.rich(props)
end

local blocks = ui.markdown(reply, { key = "reply" })

ui.command { id = "append", run = function()
	local n = chunks:get()
	chunks:set(n + 1)
	local at = n % (per + 1)
	local piece = if at == 0 then "## Section " .. (n // (per + 1) + 1) .. "\n\n"
		elseif at == per then "and that is the end of this part of the answer.\n\n"
		elseif at % 4 == 1 then "Words arriving as a reply streams, with **bold** in them, "
		else "the paragraph growing a few words at a time as it is written, "
	reply:append(piece)
end }

return ui.column {
	gap = 8,
	w = 600,
	font = "DejaVu Sans",
	ui.each(blocks, function(b: MarkdownBlock)
		if b.t == "p" or b.t == "h" then
			return runs(b.s)
		end
		return ui.div {}
	end),
}
"###;

fn main() {
    let env = |name: &str, default: usize| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let (sections, per) = (env("SECTIONS", 12), env("CHUNKS", 40));
    let mut ui = Ui::new(640., 900.);
    // The renderer's text services, without a renderer, in the font the
    // headless benches share (`zgx_workload::FONT`): counts that follow
    // neither the machine's fonts nor a GPU.
    let text = zgx_gpu::TextServices::new();
    zgx_desktop::FontData::new(std::sync::Arc::<[u8]>::from(zgx_workload::FONT_BYTES))
        .expect("the bundled font")
        .install(&mut text.text_system().borrow_mut());
    text.install(&mut ui.scene.borrow_mut());
    let script = SCRIPT.replace("PER", &per.to_string());
    let host =
        ScriptHost::inline(&ui.runtime, "md_stream_text", script).limits(bench_limits::limits());
    host.reload().expect("the bench script loads");
    let _handle = ui.mount(host.view());
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let chunks = sections * (per + 1);
    for _ in 0..chunks {
        assert!(host.run_command("append"));
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
    // It ran through the text cache: most chunks' paragraphs new texts
    // shaped there (a heading's chunk adds its own), not measured by the
    // estimate.
    let shaped = text.text_cache().borrow().shaped;
    assert!(
        shaped as usize >= chunks / 2,
        "{shaped} texts shaped by the text cache for {chunks} chunks"
    );
    println!(
        "{chunks} chunks, {shaped} texts shaped, scene nodes {}",
        ui.scene.borrow().len()
    );
}
