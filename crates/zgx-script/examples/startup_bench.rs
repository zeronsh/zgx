//! An app's cold start, headless: a chat feed of `N` (10,000) rich messages
//! handed to a script as JSON in its source, as an app bundles its data,
//! decoded into a state, a keyed list of measured rows built over it and
//! the first frame laid out, its text shaped by a window's text services.
//! The feed's shape is Expo Exact's heavy-list benchmark's: paragraphs of
//! styled runs (some Chinese, Japanese, Russian, Hindi and emoji), a quote,
//! photos, a link card and reactions on some messages. CI's instruction gate
//! counts it, so a change that slows the start of an app shows a row; the
//! feed is made here (left out of the count: `gate_marks`), not kept in the
//! repository.
//! Run: cargo run --release -p zgx-script --example startup_bench
#[path = "support/bench_limits.rs"]
mod bench_limits;
#[path = "support/gate_marks.rs"]
mod gate_marks;
use std::{fmt::Write, time::Instant};
use zgx::widgets::Ui;
use zgx_script::ScriptHost;

const SCRIPT: &str = r##"
local source = (ui.json.decode(DATA) :: any).messages
local feed = ui.state("feed", source)

local function ago(minutes: number): string
	if minutes < 60 then
		return `{math.floor(minutes)}m ago`
	elseif minutes < 1440 then
		return `{math.floor(minutes / 60)}h ago`
	end
	return `{math.floor(minutes / 1440)}d ago`
end

local styles = {
	bold = { weight = 600 },
	italic = { italic = true },
	code = { font = "mono", text_size = 15, bg = "#F2F2F7" },
	link = { color = "#007AFF", underline = true },
	mention = { color = "#007AFF", weight = 600 },
	tag = { color = "#5856D6" },
}

local function paragraph(runs)
	local entries = { wrap = true, text_size = 16, color = "#000000" } :: any
	for _, r in runs do
		local style = r.s and styles[r.s]
		if style then
			local span = table.clone(style)
			span[1] = r.t
			table.insert(entries, ui.span(span))
		else
			table.insert(entries, r.t)
		end
	end
	return ui.rich(entries)
end

local function photos(list)
	local boxes = { mt = 8, w_full = true, rounded = 12, overflow = "hidden", gap = 4, flex_wrap = true } :: any
	for _, p in list do
		local single = #list == 1
		table.insert(boxes, ui.div {
			grow = 1,
			basis = 0,
			min_w = if single then nil else 120,
			aspect = if single then p.w / p.h else 1,
			max_h = if single then 320 else nil,
			bg = "#E5E5EA",
		})
	end
	return ui.row(boxes)
end

local function row(m)
	local content = {
		grow = 1,
		basis = 0,
		min_w = 0,
		ui.row {
			items = "baseline",
			gap = 6,
			ui.text { m.author, literal = true, truncate = true, text_size = 15, weight = 600 },
			ui.text { ago(m.minutesAgo), shrink = 0, text_size = 13, color = "#8E8E93" },
		},
	} :: any
	if m.quote then
		table.insert(content, ui.column {
			mt = 6,
			p = 8,
			border_sides = { 0, 0, 0, 3 },
			border_color = "#C7C7CC",
			rounded = 8,
			bg = "#F2F2F7",
			ui.text { m.quote.author, literal = true, text_size = 13, weight = 600 },
			ui.text { m.quote.excerpt, literal = true, wrap = true, lines = 2, truncate = true, text_size = 13 },
		})
	end
	local paras = { mt = 6, gap = 8 } :: any
	for _, runs in m.paragraphs do
		table.insert(paras, paragraph(runs))
	end
	table.insert(content, ui.column(paras))
	if m.photos then
		table.insert(content, photos(m.photos))
	end
	if m.link then
		table.insert(content, ui.column {
			mt = 8,
			border = 0.5,
			border_color = "#D1D1D6",
			rounded = 12,
			overflow = "hidden",
			ui.div { w_full = true, h = 140, bg = "#E5E5EA" },
			ui.column {
				p = 10,
				ui.text { string.upper(m.link.site), literal = true, text_size = 12 },
				ui.text { m.link.title, literal = true, wrap = true, lines = 2, truncate = true, text_size = 15 },
			},
		})
	end
	if m.reactions then
		local chips = { mt = 8, flex_wrap = true, gap = 6 } :: any
		for _, r in m.reactions do
			table.insert(chips, ui.row {
				h = 28,
				px = 10,
				gap = 4,
				items = "center",
				rounded = 14,
				bg = "#F2F2F7",
				ui.text { r.emoji, literal = true, text_size = 14 },
				ui.text { tostring(r.count), text_size = 13, weight = 600 },
			})
		end
		table.insert(content, ui.row(chips))
	end
	return ui.column {
		w_full = true,
		label = `{m.author}, {ago(m.minutesAgo)}`,
		ui.row {
			w_full = true,
			p = 12,
			gap = 12,
			items = "start",
			ui.div { w = 40, h = 40, shrink = 0, rounded = 20, bg = "#E5E5EA" },
			ui.column(content),
		},
		ui.div { h = 0.5, ml = 68, bg = "#E5E5EA" },
	}
end

return ui.column {
	w_full = true,
	h_full = true,
	bg = "#FFFFFF",
	ui.text { "Heavy list", text_size = 17, weight = 600, px = 16, py = 10 },
	ui.list {
		w_full = true,
		grow = 1,
		basis = 0,
		min_h = 0,
		data = feed,
		key = "id",
		measured = true,
		height = 160,
		row = row,
	},
}
"##;

/// A deterministic stream of numbers below `of`.
fn next(seed: &mut u64, of: u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 33) % of
}

const WORDS: &[&str] = &[
    "launch",
    "layout",
    "coffee",
    "weekend",
    "station",
    "review",
    "benchmark",
    "prototype",
    "frame",
    "draft",
    "picnic",
    "schedule",
    "feedback",
    "platform",
    "album",
    "dinner",
    "deadline",
    "notes",
    "photo",
    "family",
    "design",
    "scroll",
    "render",
    "image",
    "weather",
    "friends",
    "trip",
    "great",
    "sure",
    "soon",
    "maybe",
    "definitely",
    "the",
    "a",
    "we",
    "could",
    "list",
    "text",
    "build",
    "test",
];
const FOREIGN: &[&str] = &[
    "今日は駅で会いましょう。",
    "明天在车站见面吧。",
    "Увидимся завтра на станции.",
    "नमस्ते, कल स्टेशन पर मिलते हैं।",
    "😀 ✨ 👍",
    "🍕",
];
const AUTHORS: &[&str] = &[
    "Ava", "Ben", "Chloé", "Emeka", "Fatima", "Hiro", "Ingrid", "José", "Kofi", "Léa",
];
const EMOJI: &[&str] = &["👍", "❤️", "😂", "😮", "🎉", "🔥", "🙏", "👀"];

/// `n` messages as JSON, in the heavy-list benchmark's shape.
fn feed(n: usize) -> String {
    let mut seed = 49975_u64;
    let mut out = String::from(r#"{"version":1,"messages":["#);
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    for i in 0..n {
        if i > 0 {
            out.push(',');
        }
        let author = AUTHORS[next(&mut seed, AUTHORS.len() as u64) as usize];
        let minutes = 30_000 - i as u64 * 3;
        write!(out, r#"{{"id":"m{i}","author":"{author}","avatar":"avatar-{:02}.jpg","minutesAgo":{minutes},"paragraphs":["#, i % 24).unwrap();
        for p in 0..1 + next(&mut seed, 3) {
            if p > 0 {
                out.push(',');
            }
            out.push('[');
            for r in 0..2 + next(&mut seed, 8) {
                if r > 0 {
                    out.push(',');
                }
                let text = if next(&mut seed, 9) == 0 {
                    FOREIGN[next(&mut seed, FOREIGN.len() as u64) as usize].to_string()
                } else {
                    (0..3 + next(&mut seed, 12))
                        .map(|_| WORDS[next(&mut seed, WORDS.len() as u64) as usize])
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                let style = [
                    "", "", "", "bold", "italic", "code", "link", "mention", "tag",
                ][next(&mut seed, 9) as usize];
                if style.is_empty() {
                    write!(out, r#"{{"t":"{} "}}"#, quote(&text)).unwrap();
                } else {
                    write!(out, r#"{{"t":"{} ","s":"{style}"}}"#, quote(&text)).unwrap();
                }
            }
            out.push(']');
        }
        out.push(']');
        if next(&mut seed, 9) == 0 {
            let count = 1 + next(&mut seed, 4);
            out.push_str(r#","photos":["#);
            for k in 0..count {
                if k > 0 {
                    out.push(',');
                }
                let (w, h) = [(1200, 1600), (1600, 900), (1600, 1600), (900, 1600)]
                    [next(&mut seed, 4) as usize];
                write!(
                    out,
                    r#"{{"src":"photo-{:02}.jpg","w":{w},"h":{h}}}"#,
                    next(&mut seed, 60)
                )
                .unwrap();
            }
            out.push(']');
        }
        if next(&mut seed, 8) == 0 {
            write!(
                out,
                r#","link":{{"thumb":"thumb-{:02}.jpg","title":"A design review of the launch notes","description":"Notes from the weekend's review.","site":"expo.dev"}}"#,
                next(&mut seed, 20)
            )
            .unwrap();
        }
        if next(&mut seed, 7) == 0 {
            let who = AUTHORS[next(&mut seed, AUTHORS.len() as u64) as usize];
            write!(out, r#","quote":{{"id":"m{}","author":"{who}","excerpt":"Sounds good, the station at noon then, and bring the notes from the review."}}"#, i / 2).unwrap();
        }
        if next(&mut seed, 3) == 0 {
            out.push_str(r#","reactions":["#);
            for k in 0..1 + next(&mut seed, 5) {
                if k > 0 {
                    out.push(',');
                }
                let emoji = EMOJI[(k as usize + i) % EMOJI.len()];
                write!(
                    out,
                    r#"{{"emoji":"{emoji}","count":{}}}"#,
                    1 + next(&mut seed, 40)
                )
                .unwrap();
            }
            out.push(']');
        }
        out.push('}');
    }
    out.push_str("]}");
    out
}

fn main() {
    let n: usize = std::env::var("N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    // The app's source with its data in it, made beforehand: the bench's own.
    let code = gate_marks::outside(|| {
        let data = feed(n);
        assert!(!data.contains("]==]"));
        format!("local DATA = [==[{data}]==]\n{SCRIPT}")
    });
    let at = Instant::now();
    let mut ui = Ui::new(402., 874.);
    let services = zgx_gpu::TextServices::new();
    services.install(&mut ui.scene.borrow_mut());
    ui.scene.borrow_mut().fonts_replaced();
    let host = ScriptHost::inline(&ui.runtime, "bench", code).limits(bench_limits::limits());
    host.reload().expect("the bench script loads");
    ui.mount(host.view());
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let elapsed = at.elapsed();
    if let Some(error) = host.error().get() {
        panic!("script error: {error}");
    }
    println!(
        "{n} messages: first frame laid out in {:.1} ms",
        elapsed.as_secs_f64() * 1e3
    );
}
