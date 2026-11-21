//! What a call into a native module costs against the same function linked
//! into the program, headless: a script's loop of `ui.call_sync`, by the
//! fixture module loaded as a library (`ARM=module`, `MODULE=<its library>`)
//! or by the same code as this program's own (`ARM=linked`).
//!
//! `CASE=length`: a string in, its length out, so the call alone.
//! `CASE=echo`: a string in, the same string out: a module is lent its
//! argument, so it counts the string where linked-in code is handed it.
//! `CASE=taking`: the same echo by a function handed its argument
//! (`Host::function_taking`), which counts nothing.
//! `CASE=rows`: 200 rows read a field at a time, so a module's reads.
//!
//! It prints nothing to compare by itself: run each arm under a counter and
//! subtract (`perf stat -e instructions:u`), the script's work the same in
//! both. Under cachegrind, set `ZGX_GATE` too: a round's script runs some
//! fifty times slower there, and its time budget is lengthened to match.
//!
//!   cargo build --release -p zgx-script --features modules --example module_call_bench
//!   cargo build --release -p zgx-module-fixture
//!   ARM=linked CASE=echo perf stat -e instructions:u target/release/examples/module_call_bench
//!   ARM=module MODULE=target/release/libzgx_module_fixture.so CASE=echo perf stat …

use zgx_script::{ScriptHost, Value, testing::TestApp};

#[path = "support/bench_limits.rs"]
mod bench_limits;

fn number(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(default)
}

/// The fixture's `fixture.rows`, as an app's own Rust writes it.
fn rows(rows: &Value) -> Value {
    let mut sum = 0.;
    for row in rows.list() {
        let text = |key: &str| match row.field(key) {
            Some(Value::Str(s)) => s.len(),
            _ => 0,
        };
        if let Some(Value::Num(n)) = row.field("n") {
            sum += n;
        }
        sum += (text("id") + text("text")) as f64;
        sum += f64::from(u8::from(matches!(
            row.field("done"),
            Some(Value::Bool(true))
        )));
    }
    Value::Num(sum)
}

fn main() {
    let module = std::env::var("ARM").is_ok_and(|arm| arm == "module");
    let case = std::env::var("CASE").unwrap_or_else(|_| "echo".into());
    let (calls, rounds) = (
        number("CALLS", if case == "rows" { 2_000 } else { 20_000 }),
        number("ROUNDS", 50),
    );
    let script = format!(
        r#"
        local rows = ui.shared("rows"):get()
        ui.command {{ id = "echo", title = "e", run = function()
            for _ = 1, {calls} do ui.call_sync("fixture.echo", "a string") end
        end }}
        ui.command {{ id = "taking", title = "t", run = function()
            for _ = 1, {calls} do ui.call_sync("fixture.echo_taking", "a string") end
        end }}
        ui.command {{ id = "length", title = "l", run = function()
            for _ = 1, {calls} do ui.call_sync("fixture.length", "a string") end
        end }}
        ui.command {{ id = "rows", title = "r", run = function()
            for _ = 1, {calls} do ui.call_sync("fixture.rows", rows) end
        end }}
        return ui.column {{}}
        "#
    );
    let app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "bench", script).limits(bench_limits::limits());
        let row = |i: usize| {
            Value::record([
                ("id", Value::from(format!("row-{i}").as_str())),
                ("text", Value::from("A row's text, as long as a title is.")),
                ("n", Value::Num(i as f64)),
                ("done", Value::Bool(i.is_multiple_of(3))),
                ("kind", Value::from("row")),
                ("at", Value::Num(1_794_000_000_000.)),
            ])
        };
        host.shared("rows", Value::List((0..200).map(row).collect()));
        if module {
            let path = std::env::var("MODULE").expect("MODULE=<the fixture's library>");
            // SAFETY: This workspace's own fixture, named by who runs this.
            unsafe { host.load_module(path) }.expect("the fixture module");
        } else {
            host.function("fixture.echo", |arg| arg);
            host.function("fixture.echo_taking", |arg| arg);
            host.function("fixture.length", |arg| match arg {
                Value::Str(s) => Value::Num(s.len() as f64),
                _ => Value::Num(0.),
            });
            host.function("fixture.rows", |arg| rows(&arg));
        }
        host
    });
    assert_eq!(app.error(), None);
    for _ in 0..rounds {
        assert!(app.host.run_command(&case));
    }
    assert_eq!(app.error(), None);
    println!(
        "{} {case}: {rounds} rounds of {calls} calls",
        if module { "module" } else { "linked" }
    );
}
