//! A growing paragraph streamed publish by publish, for a profiler to sample:
//! `stream_bench [publishes] [paragraphs-of-text in the last block]`.
use zgx::widgets::Ui;
use zgx_script::{ScriptHost, Value};

const SCRIPT: &str = r###"
local reply = ui.state("reply", "")
local function runs(list: { MarkdownRun }): Node
	local props: RichProps = { wrap = true }
	for _, r in list do
		table.insert(props, ui.span { r.x, literal = true, weight = if r.b then 600 else nil })
	end
	return ui.rich(props)
end
local blocks = ui.markdown(reply, { key = "reply" })
return ui.scroll { w = 640, h = 900, follow = true, ui.column {
	gap = 8,
	w = 600,
	ui.each(blocks, function(b: MarkdownBlock)
		if b.t == "p" or b.t == "h" then
			return runs(b.s)
		end
		return ui.div {}
	end),
} }
"###;

fn main() {
    let publishes: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let long: usize = std::env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(160);
    let gpu = zgx_gpu::GpuRenderer::new(640, 900).expect("a renderer");
    let mut ui = Ui::new(640., 900.);
    gpu.install_text(&mut ui.scene.borrow_mut());
    ui.refresh_text_geometry();
    let host = ScriptHost::inline(&ui.runtime, "stream_bench", SCRIPT);
    host.reload().expect("the script loads");
    let _view = ui.mount(host.view());
    let mut text: String = (0..30)
        .map(|i| format!("Paragraph {i} of an answer, with **bold words** and `inline code` in it, then a longer run of plain text.\n\n"))
        .collect();
    text.push_str(&"The last block with **bold** and `code` words, growing. ".repeat(long));
    for n in 0..publishes {
        text.push_str(&format!(" and more{:>3} words", n % 1000));
        host.set_state("reply", Value::from(text.as_str())).unwrap();
        ui.prepare_frame();
        let _ = ui.scene.borrow_mut().flush();
    }
}
