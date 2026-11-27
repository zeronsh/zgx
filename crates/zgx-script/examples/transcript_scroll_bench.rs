//! A long chat transcript scrolled back by a wheel, headless: a measured,
//! following `ui.list` of 2,000 steps (people's messages, replies in
//! Markdown with code blocks and lists, tool calls as cards), each row drawn
//! by the script from `ui.markdown`'s blocks as an app drawing its own
//! Markdown does, its text shaped by zgx-gpu's text cache (`TextServices`,
//! on the CPU, in the bundled font). It opens at the end, as a chat does,
//! then takes `EVENTS` wheel events of `STEP` points up, a frame each: the
//! rows coming into view are parsed, built from Luau, styled, laid out,
//! shaped and measured, and the list keeps the rows in view where they are
//! as those above find their heights. Nothing is drawn.
//! Run: cargo run --release -p zgx-script --example transcript_scroll_bench
//! (`STEPS` sets the transcript's steps, `EVENTS` the wheel events: 120,
//! `STEP` their points: 100; `LINKS=0` leaves links out of its paragraphs,
//! so the rows' own cost reads apart from their links' boxes.) The transcript's data is made here and left out
//! of the gate's count (`gate_marks`).
#[path = "support/gate_marks.rs"]
mod gate_marks;
use zgx_script::Slice;
use zgx_script::Value;
use zgx_script::testing::TestApp;

const SCRIPT: &str = r##"
local steps = ui.shared("bench.steps", {} :: { any })
local ICON = [[<svg viewBox="0 0 16 16"><path d="M3 2h7l3 3v9H3z" fill="none" stroke="currentColor"/></svg>]]
local CHEVRON = [[<svg viewBox="0 0 16 16"><path d="M6 4l4 4-4 4" fill="none" stroke="currentColor"/></svg>]]

local function runs(list: { MarkdownRun }, size: number?): Node
    local props: RichProps = { wrap = true, text_size = size or 14, w_full = true }
    for _, r in list do
        if r.c then
            table.insert(props, ui.span { r.x, literal = true, font = "mono", bg = "#ffffff14", padding = { 4, 0.5 }, radius = 3 })
        elseif r.a then
            table.insert(props, ui.span { r.x, literal = true, color = "#5ea5ff", underline = true, href = r.a })
        else
            table.insert(props, ui.span { r.x, literal = true, weight = if r.b then 600 else nil, italic = r.i })
        end
    end
    return ui.rich(props)
end

local function code(b: any): Node
    local lines: { Child } = {}
    for line in string.gmatch(b.text, "[^\n]+") do
        table.insert(lines, ui.text { line, font = "mono", text_size = 12, lines = 1 })
    end
    return ui.column {
        w_full = true,
        rounded = 6,
        bg = "#1b1b1b",
        border = 1,
        border_color = "#ffffff1a",
        ui.row {
            w_full = true,
            px = 10,
            py = 4,
            justify = "between",
            align = "center",
            ui.text { b.lang, text_size = 11, color = "#999999" },
            ui.button { ui.text { "Copy", text_size = 11 }, on_click = function() end },
        },
        ui.column { px = 10, pb = 8, gap = 1, lines },
    }
end

local function block(b: MarkdownBlock): Node
    if b.t == "p" then
        return runs(b.s)
    elseif b.t == "h" then
        return runs(b.s, if b.l <= 2 then 18 else 15)
    elseif b.t == "code" then
        return code(b)
    elseif b.t == "ul" or b.t == "ol" then
        local items: { Child } = {}
        for n, item in b.items do
            local first = item[1]
            if first and first.t == "p" then
                table.insert(items, ui.row {
                    w_full = true,
                    gap = 6,
                    ui.text { if b.t == "ol" then `{n}.` else "•", text_size = 14, shrink = 0 },
                    runs(first.s),
                })
            end
        end
        return ui.column { w_full = true, gap = 4, items }
    end
    return ui.div {}
end

local function reply(step: any): Node
    local children: { Child } = {}
    for _, b in ui.markdown(step.text) do
        table.insert(children, block(b))
    end
    return ui.column { w_full = true, gap = 10, children }
end

local function message(step: any): Node
    return ui.row {
        w_full = true,
        justify = "end",
        ui.column {
            max_w = 560,
            px = 12,
            py = 8,
            rounded = 12,
            bg = "#2a2a2a",
            ui.text { step.text, text_size = 14, wrap = true },
        },
    }
end

local function tool(step: any): Node
    local preview: { Child } = {}
    for line in string.gmatch(step.output, "[^\n]+") do
        table.insert(preview, ui.text { line, font = "mono", text_size = 12, lines = 1, color = "#aaaaaa" })
    end
    return ui.column {
        w_full = true,
        rounded = 8,
        border = 1,
        border_color = "#ffffff1a",
        hover = { bg = "#ffffff08" },
        ui.row {
            w_full = true,
            gap = 8,
            px = 10,
            py = 6,
            align = "center",
            ui.svg { code = CHEVRON, w = 12, h = 12, shrink = 0 },
            ui.svg { code = ICON, w = 14, h = 14, shrink = 0 },
            ui.text { step.title, text_size = 13, weight = 600, shrink = 0 },
            ui.text { step.path, font = "mono", text_size = 12, lines = 1, color = "#999999", grow = 1 },
            ui.row {
                px = 6,
                py = 2,
                rounded = 8,
                bg = if step.failed then "#5a2020" else "#203a20",
                ui.text { if step.failed then "failed" else "done", text_size = 11 },
            },
        },
        ui.column { w_full = true, px = 10, pb = 8, gap = 1, preview },
    }
end

return ui.list {
    id = "transcript",
    w_full = true,
    h_full = true,
    data = steps,
    key = "id",
    measured = true,
    follow = true,
    height = 120,
    row = function(step: any)
        return ui.column {
            w_full = true,
            px = 24,
            py = 8,
            selectable = true,
            ui.switch(step.kind, {
                message = function()
                    return message(step)
                end,
                reply = function()
                    return reply(step)
                end,
                tool = function()
                    return tool(step)
                end,
            }),
        }
    end,
}
"##;

const WORDS: &[&str] = &[
    "the", "frame", "layout", "scroll", "reply", "measured", "row", "builds", "ahead", "of",
    "view", "and", "keeps", "what", "it", "made", "cache", "text", "shaped", "list", "state",
    "node", "style", "paint",
];

fn next(seed: &mut u64, of: u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 33) % of
}
fn words(seed: &mut u64, count: u64) -> String {
    let mut out = String::new();
    for w in 0..count {
        if w > 0 {
            out.push(' ');
        }
        out.push_str(WORDS[next(seed, WORDS.len() as u64) as usize]);
    }
    out
}
/// A paragraph with the marks replies carry: bold, a code span, and a link
/// if `links`.
fn paragraph(seed: &mut u64, count: u64, links: bool) -> String {
    let mut out = words(seed, count / 3);
    out.push_str(&format!(" **{}** ", words(seed, 2)));
    out.push_str(&words(seed, count / 3));
    out.push_str(&format!(" `{}_{}()` ", words(seed, 1), next(seed, 100)));
    out.push_str(&words(seed, count / 3));
    if links {
        out.push_str(&format!(
            " [{}](https://example.com/{}).",
            words(seed, 2),
            next(seed, 1000)
        ));
    } else {
        out.push_str(&format!(" {} {}.", words(seed, 2), next(seed, 1000)));
    }
    out
}
fn reply(seed: &mut u64, i: usize, links: bool) -> String {
    let mut parts = Vec::new();
    if i.is_multiple_of(5) {
        parts.push(format!("## {}", words(seed, 4)));
    }
    let n = 30 + next(seed, 60);
    parts.push(paragraph(seed, n, links));
    if i.is_multiple_of(3) {
        let mut code = String::from("```rust\n");
        for line in 0..4 + next(seed, 14) {
            code.push_str(&format!(
                "let {}_{line} = {line} * {i}; // {}\n",
                words(seed, 1),
                words(seed, 4)
            ));
        }
        code.push_str("```");
        parts.push(code);
    }
    if i % 4 == 1 {
        let items: Vec<String> = (0..2 + next(seed, 4))
            .map(|_| {
                let n = 9 + next(seed, 12);
                format!("- {}", paragraph(seed, n, links))
            })
            .collect();
        parts.push(items.join("\n"));
    }
    let n = 15 + next(seed, 40);
    parts.push(paragraph(seed, n, links));
    parts.join("\n\n")
}

fn main() {
    let env = |name: &str, default: usize| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let (count, events) = (env("STEPS", 2000), env("EVENTS", 120));
    let step = env("STEP", 100) as f32;
    let links = env("LINKS", 1) != 0;
    let mut seed = 11_u64;
    // A turn: a message, a reply, a few tool calls, a reply.
    let steps: Vec<Value> = gate_marks::outside(|| {
        (0..count)
            .map(|i| {
                let id = ("id", Value::from(format!("s{i}").as_str()));
                match i % 6 {
                    0 => {
                        let n = 8 + next(&mut seed, 40);
                        Value::record([
                            id,
                            ("kind", Value::from("message")),
                            ("text", Value::from(words(&mut seed, n).as_str())),
                        ])
                    }
                    2..=4 => {
                        let output: Vec<String> = (0..2 + next(&mut seed, 4))
                            .map(|_| {
                                let n = 6 + next(&mut seed, 8);
                                words(&mut seed, n)
                            })
                            .collect();
                        Value::record([
                            id,
                            ("kind", Value::from("tool")),
                            ("title", Value::from(["Read", "Edit", "Bash"][i % 3])),
                            (
                                "path",
                                Value::from(
                                    format!(
                                        "src/{}/{}.rs",
                                        words(&mut seed, 1),
                                        words(&mut seed, 1)
                                    )
                                    .as_str(),
                                ),
                            ),
                            ("output", Value::from(output.join("\n").as_str())),
                            ("failed", Value::Bool(i % 37 == 4)),
                        ])
                    }
                    _ => Value::record([
                        id,
                        ("kind", Value::from("reply")),
                        ("text", Value::from(reply(&mut seed, i, links).as_str())),
                    ]),
                }
            })
            .collect()
    });
    let mut app = TestApp::inline(SCRIPT);
    let services = zgx_gpu::TextServices::new();
    services.install(&mut app.ui.scene.borrow_mut());
    app.ui.scene.borrow_mut().fonts_replaced();
    app.ui.scene.borrow_mut().resize(1000., 800.);
    app.host
        .shared("bench.steps", Value::Nil)
        .set(Value::List(Slice::from(steps)));
    app.frames(5);
    assert_eq!(app.error(), None);
    let built = app.ui.scene.borrow().len();
    if zgx_fingerprint::on() {
        zgx_fingerprint::paint_only(&app.ui.scene.borrow());
    }
    for _ in 0..events {
        app.scroll("#transcript", -step);
        app.frame();
        if zgx_fingerprint::on() {
            zgx_fingerprint::paint_only(&app.ui.scene.borrow());
        }
    }
    assert_eq!(app.error(), None);
    let nodes = app.ui.scene.borrow().len();
    eprintln!(
        "transcript_scroll_bench: {count} steps (links: {links}), {events} wheel events of {step} pt, {built} nodes at the end, {nodes} now"
    );
}
