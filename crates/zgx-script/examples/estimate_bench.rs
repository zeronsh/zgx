//! `ui.list { estimate = … }`, headless: rows of Markdown and chips, their
//! heights estimated, then scrolled through, the width changed, rows added.
//! CI's instruction gate counts it, so what an estimate costs a list that asks
//! for one is guarded, and a cost put in its path shows.
//! Run: cargo run --release -p zgx-script --example estimate_bench
//! (`ROWS` sets the rows, `ESTIMATE=0` leaves it off.)
use std::rc::Rc;
use zgx_script::Value;
use zgx_script::testing::TestApp;

const SCRIPT: &str = r##"
local rows = ui.shared("bench.rows", {})
return ui.list {
    id = "bench",
    w_full = true,
    h_full = true,
    data = rows,
    key = "id",
    measured = true,
    ESTIMATE
    row = function(r)
        local blocks = { w_full = true, gap = 4 }
        for _, b in ui.markdown(r.body) do
            if b.t == "code" then
                table.insert(blocks, ui.text { b.text, size = 12, w_full = true })
            elseif b.items then
                for _, item in b.items do
                    for _, inner in item do
                        local words = {}
                        for _, run in inner.s or {} do
                            table.insert(words, run.x)
                        end
                        table.insert(blocks, ui.text { table.concat(words), size = 14, wrap = true, w_full = true })
                    end
                end
            elseif b.s then
                local words = {}
                for _, run in b.s do
                    table.insert(words, run.x)
                end
                table.insert(blocks, ui.text { table.concat(words), size = if b.t == "h" then 18 else 14, wrap = true, w_full = true })
            end
        end
        return ui.column {
            w_full = true,
            gap = 6,
            p = 12,
            ui.text { r.from, size = 13 },
            ui.column(blocks),
        }
    end,
}
"##;
const RULE: &str = r#"estimate = {
        height = 48,
        text = {
            field = "body", size = 14, line = 18, char = 0.72, inset = 24,
            markdown = { paragraph = 4, heading = 4, item = 4, code = 16 },
        },
    },"#;
fn next(seed: &mut u64, of: u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 33) % of
}
fn words(seed: &mut u64, count: u64) -> String {
    let mut out = String::new();
    for w in 0..count {
        let len = 2 + next(seed, 9);
        if w > 0 {
            out.push(' ');
        }
        for _ in 0..len {
            out.push((b'a' + next(seed, 26) as u8) as char);
        }
    }
    out
}
fn body(seed: &mut u64) -> String {
    let mut out = String::new();
    for _ in 0..1 + next(seed, 4) {
        let a = 5 + next(seed, 60);
        match next(seed, 5) {
            0 => out.push_str(&format!("## {}\n\n", words(seed, 4))),
            1 => {
                for _ in 0..1 + next(seed, 4) {
                    out.push_str(&format!("- {}\n", words(seed, a / 3 + 1)));
                }
                out.push('\n');
            }
            2 => out.push_str(&format!(
                "```\n{}\n{}\n```\n\n",
                words(seed, 3),
                words(seed, 4)
            )),
            _ => out.push_str(&format!("{}\n\n", words(seed, a))),
        }
    }
    out
}
fn main() {
    let count: usize = std::env::var("ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let on = std::env::var("ESTIMATE").map_or(true, |v| v != "0");
    let mut seed = 42_u64;
    let mut rows: Vec<Value> = (0..count)
        .map(|i| {
            Value::record([
                ("id", Value::from(format!("r{i}").as_str())),
                ("from", Value::from(words(&mut seed, 2).as_str())),
                ("body", Value::from(body(&mut seed).as_str())),
            ])
        })
        .collect();
    let code = SCRIPT.replace("ESTIMATE", if on { RULE } else { "" });
    let mut app = TestApp::inline(&code);
    let services = zgx_gpu::TextServices::new();
    services.install(&mut app.ui.scene.borrow_mut());
    app.ui.scene.borrow_mut().fonts_replaced();
    app.ui.scene.borrow_mut().resize(900., 800.);
    app.host
        .shared("bench.rows", Value::Nil)
        .set(Value::List(Rc::from(rows.clone())));
    app.frames(10);
    for _ in 0..200 {
        app.scroll("#bench", 300.);
        app.frame();
    }
    for width in [700., 900.] {
        app.ui.scene.borrow_mut().resize(width, 800.);
        app.frames(5);
    }
    for i in 0..20 {
        let row = Value::record([
            ("id", Value::from(format!("new{i}").as_str())),
            ("from", Value::from("someone")),
            ("body", Value::from(body(&mut seed).as_str())),
        ]);
        rows.push(row);
        app.host
            .shared("bench.rows", Value::Nil)
            .set(Value::List(Rc::from(rows.clone())));
        app.frames(2);
    }
    println!("{}", if on { "estimating" } else { "off" });
}
