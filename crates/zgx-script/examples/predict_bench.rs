//! `ui.list { predict = true }`, headless: rows of Markdown and chips, mounted
//! as a list opens, their heights learned and predicted, then scrolled through.
//! CI's instruction gate counts it, so what prediction costs a list that asks
//! for it is guarded, and a cost put in its path shows.
//! Run: cargo run --release -p zgx-script --example predict_bench
//! (`ROWS` sets the rows, `PREDICT=0` leaves it off.)
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
    key = function(r) return r.id end,
    measured = true,
    predict = PREDICT,
    row = function(r)
        local chips = { wrap = true, gap = 4, w_full = true }
        for _, t in r.tags do
            table.insert(chips, ui.div { px = 6, py = 2, rounded = 4, bg = "#eee", ui.text { t, size = 11 } })
        end
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
            ui.text { r.from, size = 13, bold = r.unread },
            if r.flag then ui.text { r.flag, size = 11 } else nil,
            ui.column(blocks),
            if #r.tags > 0 then ui.row(chips) else nil,
        }
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
    let on = std::env::var("PREDICT").map_or(true, |v| v != "0");
    let mut seed = 42_u64;
    let rows: Vec<Value> = (0..count)
        .map(|i| {
            let tags: Vec<Value> = (0..next(&mut seed, 5))
                .map(|_| Value::from(words(&mut seed, 1).as_str()))
                .collect();
            Value::record([
                ("id", Value::from(format!("r{i}").as_str())),
                ("from", Value::from(words(&mut seed, 2).as_str())),
                ("body", Value::from(body(&mut seed).as_str())),
                ("unread", Value::Bool(next(&mut seed, 3) == 0)),
                (
                    "flag",
                    if next(&mut seed, 5) == 0 {
                        Value::from("urgent")
                    } else {
                        Value::Nil
                    },
                ),
                ("tags", Value::List(Rc::from(tags))),
            ])
        })
        .collect();
    let code = SCRIPT.replace("PREDICT", if on { "true" } else { "false" });
    let mut app = TestApp::inline(&code);
    let services = zgx_gpu::TextServices::new();
    services.install(&mut app.ui.scene.borrow_mut());
    app.ui.scene.borrow_mut().fonts_replaced();
    app.ui.scene.borrow_mut().resize(900., 800.);
    app.host
        .shared("bench.rows", Value::Nil)
        .set(Value::List(Rc::from(rows)));
    app.frames(10);
    let step = |app: &mut TestApp| {
        app.frame();
        zgx_script::predict::tick_all(std::time::Duration::from_millis(2));
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    // Open: until all it can say is said.
    let (mut was, mut same) = ((u32::MAX, 0, 0), 0);
    for _ in 0..if on { 3000 } else { 60 } {
        step(&mut app);
        zgx_script::predict::settle();
        let now = zgx_script::predict::counts();
        same = if now == was && now.2 == 0 {
            same + 1
        } else {
            0
        };
        if on && same > 8 {
            break;
        }
        was = now;
        if on {
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    }
    // Through it, then a width changed.
    for _ in 0..300 {
        app.scroll("#bench", 300.);
        step(&mut app);
    }
    app.ui.scene.borrow_mut().resize(700., 800.);
    for _ in 0..20 {
        step(&mut app);
    }
    println!(
        "{} | {}",
        if on { "predicting" } else { "off" },
        zgx_script::predict::take_stats()
    );
}
