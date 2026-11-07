//! `ui.list { estimate = … }`: rows out of view stand at what their data says,
//! a list's whole length right before any is built.
use zgx_script::Value;
use zgx_script::testing::TestApp;

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
fn rows(n: usize, mut seed: u64) -> Value {
    let rows: Vec<Value> = (0..n)
        .map(|i| {
            let count = 3 + next(&mut seed, 120);
            let mut fields = vec![
                ("id", Value::from(format!("r{i}").as_str())),
                ("body", Value::from(words(&mut seed, count).as_str())),
            ];
            if next(&mut seed, 3) == 0 {
                fields.push(("note", Value::from("a note")));
            }
            Value::record(fields)
        })
        .collect();
    Value::List(std::rc::Rc::from(rows))
}
/// Rows: 12 of padding each way, a wrapped body, a note if there is one.
fn script(estimate: &str) -> String {
    format!(
        r##"
local rows = ui.shared("list.rows", {{}})
local offset = ui.shared("list.offset", 0)
return ui.list {{
    id = "list",
    w_full = true,
    h_full = true,
    data = rows,
    offset = offset,
    key = "id",
    measured = true,
    {estimate}
    row = function(r)
        return ui.column {{
            w_full = true,
            p = 12,
            ui.text {{ r.body, size = 14, wrap = true, w_full = true }},
            if r.note then ui.text {{ r.note, size = 12 }} else nil,
        }}
    end,
}}
"##
    )
}
const RULE: &str = r#"estimate = {
        height = 24,
        text = { field = "body", size = 14, line = LINE, char = CHAR, inset = 24 },
        has = { note = { height = 16 } },
    },"#;
/// `RULE` for generated words, whose letters are wider than prose's.
fn generated() -> String {
    RULE.replace("LINE", "18").replace("CHAR", "0.72")
}
/// `RULE` for prose.
fn prose() -> String {
    RULE.replace("LINE", "18").replace("CHAR", "0.47")
}
fn mounted(code: &str, data: Value, width: f32) -> (TestApp, zgx_gpu::TextServices) {
    let app = TestApp::inline(code);
    let services = zgx_gpu::TextServices::new();
    services.install(&mut app.ui.scene.borrow_mut());
    app.ui.scene.borrow_mut().fonts_replaced();
    app.ui.scene.borrow_mut().resize(width, 600.);
    app.host.shared("list.rows", Value::Nil).set(data);
    app.frames(4);
    (app, services)
}
/// The list's whole length now: its offset at its end, and the view.
fn length(app: &mut TestApp) -> f32 {
    app.scroll("#list", 1e7);
    app.frames(2);
    let offset = app
        .host
        .shared("list.offset", Value::Nil)
        .signal()
        .with_untracked(Value::clone);
    let Value::Num(offset) = offset else {
        panic!("an offset")
    };
    offset as f32 + 600.
}
/// Its length once every row has been measured: what's true.
fn measured_length(app: &mut TestApp) -> f32 {
    app.scroll("#list", -1e7);
    app.frames(2);
    for _ in 0..600 {
        app.scroll("#list", 500.);
        app.frames(1);
    }
    length(app)
}
fn off_by(estimate: f32, truth: f32) -> f32 {
    (estimate / truth - 1.).abs()
}
#[test]
fn its_length_is_right_before_any_row_is_built() {
    let data = rows(300, 5);
    let (mut app, _services) = mounted(&script(&generated()), data.clone(), 800.);
    let estimated = length(&mut app);
    let truth = measured_length(&mut app);
    assert!(
        off_by(estimated, truth) < 0.1,
        "{estimated} against {truth}"
    );
    // Without one (the control): at the default height, nowhere near.
    let (mut plain, _services) = mounted(&script(""), data, 800.);
    let unmeasured = length(&mut plain);
    assert!(
        off_by(unmeasured, truth) > 0.5,
        "{unmeasured} against {truth}"
    );
}
#[test]
fn a_new_width_estimates_again() {
    let data = rows(300, 6);
    let (mut app, _services) = mounted(&script(&generated()), data.clone(), 800.);
    let wide = length(&mut app);
    app.ui.scene.borrow_mut().resize(400., 600.);
    app.frames(3);
    let narrow = length(&mut app);
    assert!(narrow > wide * 1.4, "{wide} then {narrow}");
    let (mut fresh, _services) = mounted(&script(&generated()), data, 400.);
    let truth = measured_length(&mut fresh);
    assert!(off_by(narrow, truth) < 0.1, "{narrow} against {truth}");
}
#[test]
fn a_row_measured_keeps_its_height_when_others_come() {
    let data = rows(40, 7);
    let (mut app, _services) = mounted(&script(&generated()), data.clone(), 800.);
    let truth = measured_length(&mut app);
    // Every row measured: a new one adds its estimate, and the rest stand.
    let mut more = data.list().to_vec();
    let added = Value::record([("id", Value::from("new")), ("body", Value::from("short"))]);
    more.push(added);
    app.host
        .shared("list.rows", Value::Nil)
        .set(Value::List(std::rc::Rc::from(more)));
    app.frames(3);
    let after = length(&mut app);
    assert!((after - truth - 42.).abs() < 25., "{truth} then {after}");
}
#[test]
#[should_panic(expected = "`estimate` needs `key`")]
fn it_says_it_needs_keys() {
    TestApp::inline(
        r#"return ui.list { data = ui.state("rows", {}), measured = true,
        estimate = { height = 1 }, row = function() return ui.div {} end }"#,
    );
}
/// Reads this repository's own documents as a list's rows, and says how far an
/// estimate is from measured, and what it cost: `--ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn on_real_documents() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files: Vec<_> = std::fs::read_dir(root.join("docs/reference"))
        .unwrap()
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    let mut rows = Vec::new();
    for file in files {
        // Each section a row.
        for part in std::fs::read_to_string(file).unwrap().split("\n## ") {
            let id = format!("r{}", rows.len());
            rows.push(Value::record([
                ("id", Value::from(id.as_str())),
                ("body", Value::from(part)),
            ]));
        }
    }
    let count = rows.len();
    let data = Value::List(std::rc::Rc::from(rows));
    for width in [800., 500.] {
        let t = std::time::Instant::now();
        let (mut app, _services) = mounted(&script(&prose()), data.clone(), width);
        let opened = t.elapsed();
        let estimated = length(&mut app);
        let truth = measured_length(&mut app);
        let (mut plain, _services) = mounted(&script(""), data.clone(), width);
        let unmeasured = length(&mut plain);
        eprintln!(
            "REAL width {width}: {count} rows, measured {truth}, estimate {estimated} ({:+.1}%), without one {unmeasured} ({:+.1}%), opened in {opened:?}",
            100. * (estimated / truth - 1.),
            100. * (unmeasured / truth - 1.),
        );
    }
}
