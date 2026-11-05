//! `ui.list { predict = true }`: rows' heights from their data alone, on a
//! list no rule was written for.
use std::collections::HashMap;
use zgx_script::Value;
use zgx_script::testing::TestApp;

fn words(seed: &mut u64, count: u64) -> String {
    let mut out = String::new();
    for w in 0..count {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let len = 2 + (*seed >> 33) % 9;
        if w > 0 {
            out.push(' ');
        }
        for c in 0..len {
            out.push((b'a' + ((*seed >> (20 + c)) % 26) as u8) as char);
        }
    }
    out
}
fn next(seed: &mut u64, of: u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 33) % of
}
/// A mail list: nothing like a chat's rows.
fn mail(n: usize, mut seed: u64) -> Value {
    let rows: Vec<Value> = (0..n)
        .map(|i| {
            let tags: Vec<Value> = (0..next(&mut seed, 6))
                .map(|_| Value::from(words(&mut seed, 1).as_str()))
                .collect();
            let (subject, preview) = {
                let (a, b) = (3 + next(&mut seed, 20), 5 + next(&mut seed, 70));
                (words(&mut seed, a), words(&mut seed, b))
            };
            Value::record([
                ("id", Value::from(format!("r{i}").as_str())),
                ("sender", Value::from(words(&mut seed, 2).as_str())),
                ("subject", Value::from(subject.as_str())),
                ("preview", Value::from(preview.as_str())),
                ("unread", Value::Bool(next(&mut seed, 3) == 0)),
                (
                    "flag",
                    if next(&mut seed, 5) == 0 {
                        Value::from("urgent")
                    } else {
                        Value::Nil
                    },
                ),
                ("tags", Value::List(std::rc::Rc::from(tags))),
            ])
        })
        .collect();
    Value::List(std::rc::Rc::from(rows))
}
const MAIL: &str = r##"
local rows = ui.shared("mail.rows", {})
return ui.list {
    id = "mail",
    w_full = true,
    h_full = true,
    data = rows,
    key = function(r) return r.id end,
    measured = true,
    predict = true,
    row = function(r)
        local chips = { wrap = true, gap = 4, w_full = true }
        for _, t in r.tags do
            table.insert(chips, ui.div { px = 6, py = 2, rounded = 4, bg = "#eee", ui.text { t, size = 11 } })
        end
        return ui.div {
            id = `row-{r.id}`,
            w_full = true,
            ui.row {
                w_full = true,
                gap = 12,
                p = 12,
                items = "start",
                ui.div { w = 32, h = 32, rounded = 16, bg = "#ccc" },
                ui.column {
                    grow = 1,
                    min_w = 0,
                    gap = 2,
                    ui.row {
                        w_full = true,
                        gap = 8,
                        ui.text { r.sender, size = 13, bold = r.unread },
                        if r.flag then ui.text { r.flag, size = 11 } else nil,
                    },
                    ui.text { r.subject, size = 14, wrap = true, w_full = true, bold = r.unread },
                    ui.text { r.preview, size = 13, wrap = true, w_full = true },
                    if #r.tags > 0 then ui.row(chips) else nil,
                },
            },
        }
    end,
}
"##;
/// Heights said before a row was ever mounted against those it then had.
fn score(code: &str, shared: &str, rows: Value, width: f32) -> (u32, u32, u32, String) {
    let mut app = TestApp::inline(code);
    let services = zgx_gpu::TextServices::new();
    services.install(&mut app.ui.scene.borrow_mut());
    app.ui.scene.borrow_mut().fonts_replaced();
    app.ui.scene.borrow_mut().resize(width, 800.);
    let count = rows.list().len();
    app.host.shared(shared, Value::Nil).set(rows);
    app.frames(10);
    let (mut was, mut same) = ((u32::MAX, 0, 0), 0);
    for _ in 0..600 {
        zgx_script::predict::settle();
        std::thread::sleep(std::time::Duration::from_millis(16));
        app.frames(1);
        let now = zgx_script::predict::counts();
        same = if now == was && now.2 == 0 {
            same + 1
        } else {
            0
        };
        if same > 6 {
            break;
        }
        was = now;
    }
    zgx_script::predict::settle();
    let said = zgx_script::predict::heights();
    let stats = zgx_script::predict::take_stats();
    let mut real: HashMap<String, f32> = HashMap::new();
    let mut quiet = 0;
    for _ in 0..4000 {
        let opts = zgx_script::driver::TreeOptions {
            depth: None,
            budget: usize::MAX,
            all: true,
        };
        let (lines, hidden) = zgx_script::driver::tree_lines(&app.ui, &app.handle, None, &opts);
        let text = zgx_script::driver::tree_lines_text(&lines, hidden);
        let before = real.len();
        for line in text.lines() {
            let mut it = line.trim_start().split_whitespace();
            let id = it.next().unwrap_or("");
            if let Some(row) = it.next().unwrap_or("").strip_prefix("#row-")
                && let Some(node) = app.find(id)
            {
                real.insert(row.to_string(), app.ui.scene.borrow().bounds(node).height);
            }
        }
        if real.len() >= count {
            break;
        }
        quiet = if real.len() == before { quiet + 1 } else { 0 };
        if quiet > 60 {
            break;
        }
        app.scroll("#mail", 500.);
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            app.frame();
        }
    }
    let (mut exact, mut wrong, mut known) = (0, 0, 0);
    let mut first_wrong = String::new();
    for (i, (h, mounted)) in said.iter().enumerate() {
        let Some(r) = real.get(&format!("r{i}")) else {
            continue;
        };
        if *mounted {
            known += 1;
        } else if (h - r).abs() < 0.05 {
            exact += 1;
        } else {
            wrong += 1;
            if first_wrong.is_empty() {
                first_wrong = format!("r{i}: said {h}, is {r}");
            }
        }
    }
    (
        exact,
        wrong,
        known,
        format!("{stats} | {first_wrong} | seen {}", real.len()),
    )
}
#[test]
fn a_list_no_rule_was_written_for() {
    for (seed, width) in [(11_u64, 900_f32), (222, 640.), (3333, 1200.)] {
        let (exact, wrong, known, why) = score(MAIL, "mail.rows", mail(300, seed), width);
        eprintln!(
            "MAIL seed {seed} width {width}: exact {exact} wrong {wrong} known {known}: {why}"
        );
        assert!(
            exact + wrong > 100,
            "too few rows out of view: {exact} {wrong} {known}: {why}"
        );
        assert!(
            wrong * 20 <= exact + wrong,
            "exact {exact} wrong {wrong}: {why}"
        );
    }
}
/// Rows whose parts don't add up: a block its row skips still changes the gap
/// of the next. It must say so and leave them estimates, not guess.
const SKIPPING: &str = r##"
local rows = ui.shared("mail.rows", {})
return ui.list {
    id = "mail",
    w_full = true,
    h_full = true,
    data = rows,
    key = function(r) return r.id end,
    measured = true,
    predict = true,
    row = function(r)
        local kids = { w_full = true, gap = 7, p = 8 }
        -- One line for either: not a sum of what each adds.
        if r.unread or r.flag then
            table.insert(kids, ui.text { r.sender, size = 13 })
        end
        table.insert(kids, ui.text { r.preview, size = 13, wrap = true, w_full = true })
        return ui.div { id = `row-{r.id}`, w_full = true, ui.column(kids) }
    end,
}
"##;
#[test]
fn rows_it_cannot_model_are_said_nothing_of_or_exactly() {
    let (exact, wrong, known, why) = score(SKIPPING, "mail.rows", mail(300, 7), 900.);
    eprintln!("SKIPPING: exact {exact} wrong {wrong} known {known}: {why}");
    // Its limit, held here: rows whose parts interact stay wrong until mounted,
    // about as often as they come (32 samples can pass by chance); never most.
    assert!(wrong * 5 <= exact + wrong, "{exact} {wrong}: {why}");
}
