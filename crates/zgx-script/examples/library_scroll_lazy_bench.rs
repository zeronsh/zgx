//! `library_scroll_bench`'s table written with its branches as functions
//! (`ui.switch`'s cases, `ui.show`'s rarely shown content): built only
//! when they show, so a row that comes into view builds the paper it
//! shows and not the placeholder, the loading row, the rename field or the
//! retry button it doesn't. The same 2,000 papers, scrolled 600 points at a
//! time; CI's instruction gate counts it beside the eager table, so the
//! difference shows what a lazy branch saves.
//! Run: cargo run --release -p zgx-script --example library_scroll_lazy_bench
//! (`ROWS` sets the rows, `STEPS` the 600-point scrolls: 10, each about 17
//! rows in.)
use std::rc::Rc;
use zgx_script::Value;
use zgx_script::testing::TestApp;

const SCRIPT: &str = r##"
local rows = ui.shared("bench.rows", {} :: { any })
local renaming = ui.state("renaming", "")
local ICON = [[<svg viewBox="0 0 16 16"><path d="M2 3h5l2 2h5v8H2z" fill="currentColor"/></svg>]]
local function icon()
    return ui.svg { code = ICON, w = 14, h = 14, shrink = 0 }
end
local function cell(w, ...)
    return ui.row { w = w, h_full = true, align = "center", gap = 6, px = 8, shrink = 0, ... }
end
local function chip(label)
    return ui.row { px = 6, py = 2, gap = 4, rounded = 8, bg = "#2a2a2a", icon(), ui.text { label, text_size = 11 } }
end
local function title(r)
    return ui.column {
        grow = 1,
        gap = 2,
        ui.row {
            gap = 4,
            align = "center",
            ui.show(function()
                return renaming:get() == r.id
            end, function() return ui.input { value = ui.use_state(r.title), w = 220 } end, ui.text {
                r.title,
                text_size = 13,
                lines = 1,
            }),
            ui.show(r.failed, function() return ui.button { ui.text { "Retry import", text_size = 11 }, on_click = function() end } end),
        },
        ui.text { r.authors, text_size = 11, lines = 1, color = "#999999" },
    }
end
local function paper(r)
    return ui.row {
        w_full = true,
        h = 36,
        hover = { bg = "#ffffff10" },
        cell(28, icon()),
        cell(320, title(r)),
        cell(140, ui.text { r.venue, text_size = 12, lines = 1 }),
        cell(60, ui.text { tostring(r.year), text_size = 12 }),
        cell(100, ui.text { r.added, text_size = 12 }),
        cell(80, ui.show(r.notes > 0, function() return ui.row { gap = 3, icon(), ui.text { tostring(r.notes), text_size = 11 } } end)),
        cell(180, chip(r.tag), chip("pdf")),
        cell(28, ui.button { icon(), on_click = function() end }),
    }
end
local function loading_more()
    return ui.row { w_full = true, h = 36, align = "center", px = 12, ui.text { "Loading more…", text_size = 12 } }
end
local function placeholder()
    return ui.row {
        w_full = true,
        h = 36,
        gap = 12,
        px = 12,
        align = "center",
        ui.div { w = 14, h = 14, rounded = 3, bg = "#2a2a2a" },
        ui.div { w = 220, h = 10, rounded = 3, bg = "#2a2a2a" },
        ui.div { w = 120, h = 10, rounded = 3, bg = "#2a2a2a" },
        ui.div { w = 60, h = 10, rounded = 3, bg = "#2a2a2a" },
    }
end
return ui.list {
    id = "library",
    w_full = true,
    h_full = true,
    data = rows,
    key = "id",
    height = 36,
    row = function(r: any)
        return ui.switch(r.kind, {
            more = loading_more,
            placeholder = placeholder,
            row = function() return paper(r) end,
        })
    end,
}
"##;

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
        for _ in 0..2 + next(seed, 9) {
            out.push((b'a' + next(seed, 26) as u8) as char);
        }
    }
    out
}
fn main() {
    let env = |name: &str, default: usize| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let (count, steps) = (env("ROWS", 2000), env("STEPS", 10));
    let mut seed = 7_u64;
    let rows: Vec<Value> = (0..count)
        .map(|i| {
            let words = |seed: &mut u64, n| Value::from(words(seed, n).as_str());
            Value::record([
                ("id", Value::from(format!("p{i}").as_str())),
                ("kind", Value::from("row")),
                ("title", words(&mut seed, 6)),
                ("authors", words(&mut seed, 4)),
                ("venue", words(&mut seed, 2)),
                ("year", Value::Num((1990 + next(&mut seed, 35)) as f64)),
                ("added", Value::from("12 Nov 2026")),
                ("notes", Value::Num(next(&mut seed, 3) as f64)),
                ("tag", words(&mut seed, 1)),
                // One failed import, near the top.
                ("failed", Value::Bool(i == 3)),
            ])
        })
        .collect();
    let mut app = TestApp::inline(SCRIPT);
    let services = zgx_gpu::TextServices::new();
    services.install(&mut app.ui.scene.borrow_mut());
    app.ui.scene.borrow_mut().fonts_replaced();
    app.ui.scene.borrow_mut().resize(1000., 700.);
    app.host
        .shared("bench.rows", Value::Nil)
        .set(Value::List(Rc::from(rows)));
    app.frames(5);
    assert_eq!(app.error(), None);
    if zgx_fingerprint::on() {
        zgx_fingerprint::paint_only(&app.ui.scene.borrow());
    }
    for _ in 0..steps {
        app.scroll("#library", 600.);
        app.frame();
        if zgx_fingerprint::on() {
            zgx_fingerprint::paint_only(&app.ui.scene.borrow());
        }
    }
    let nodes = app.ui.scene.borrow().len();
    eprintln!(
        "library_scroll_lazy_bench: {count} rows, {steps} scrolls of 600 pt, {nodes} nodes now"
    );
}
