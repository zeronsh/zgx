//! One publish of a growing answer: a keyed `ui.each` of 30 markdown blocks
//! whose last block's text grows by about 20 characters, each phase's
//! instructions for a publish (`zgx::profile::take`): the cost should follow
//! the one block that changed, not the 29 beside it. Its own process: it
//! counts with `ZGX_PROFILE`.
use zgx::profile::Phase;
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

fn block(i: usize) -> String {
    format!(
        "Paragraph {i} of an answer, with **bold words** and `inline code` in it, then a longer run of plain text that goes on for a while so that it wraps over a couple of lines in a column of six hundred points.\n\n"
    )
}

fn per_publish(blocks: usize, publishes: usize) -> Vec<(Phase, u64)> {
    let mut ui = Ui::new(640., 900.);
    let host = match std::env::var("STREAM_FIXTURE") {
        Ok(path) => ScriptHost::new(&ui.runtime, std::path::Path::new(&path)),
        Err(_) => ScriptHost::inline(&ui.runtime, "stream_update", SCRIPT),
    };
    let state = std::env::var("STREAM_STATE").unwrap_or("reply".into());
    host.reload().expect("the script loads");
    let _view = ui.mount(host.view());
    let mut text: String = (0..blocks).map(block).collect();
    let long: usize = std::env::var("LASTLEN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    text.push_str(&"The last block with **bold** and `code` words, growing. ".repeat(long));
    text.push_str("The last block, growing");
    host.set_state(&state, Value::from(text.as_str())).unwrap();
    ui.prepare_frame();
    let _ = ui.scene.borrow_mut().flush();
    let mut grow = |n: usize| {
        host.collect_luau_only();
        zgx::profile::take();
        text.push_str(&format!(" and more{n:>3} words"));
        host.set_state(&state, Value::from(text.as_str())).unwrap();
        ui.prepare_frame();
        let _ = ui.scene.borrow_mut().flush();
        zgx::profile::take()
    };
    for n in 0..3 {
        grow(n);
    }
    let counted: Vec<Vec<(Phase, u64, std::time::Duration)>> =
        (0..publishes).map(|n| grow(n + 3)).collect();
    (0..counted[0].len())
        .map(|i| {
            let total: u64 = counted.iter().map(|c| c[i].1).sum();
            (counted[0][i].0, total / publishes as u64)
        })
        .collect()
}

#[test]
fn a_publish_costs_the_changed_block_not_the_answer() {
    let small = per_publish(3, 20);
    let large = per_publish(30, 20);
    for ((phase, a), (_, b)) in small.iter().zip(&large) {
        if std::env::var_os("STREAM_PRINT").is_some() {
            eprintln!("{phase:?}: 3 blocks {a}, 30 blocks {b}");
        }
        if matches!(
            phase,
            Phase::Script | Phase::Update | Phase::Layout | Phase::Damage
        ) {
            assert!(
                *b as f64 <= *a as f64 * 1.35 + 20_000.,
                "{phase:?}: {a} instructions a publish at 3 blocks, {b} at 30"
            );
        }
    }
}
