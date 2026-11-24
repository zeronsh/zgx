//! What a native module's reach into its host costs (stage 2), headless,
//! against the host's own way of doing the same:
//!
//! `CASE=set`: `N` sets of a shared state, each a new number: by a module
//! (`ARM=module`, `Shared::set` through `Api::shared_set`), or by the
//! host's own `Shared::set` (`ARM=host`).
//! `CASE=post`: `N` round trips from a thread to the interface's thread and
//! a job run there: a module's `Ui::post` (`ARM=module`), or a value sent
//! with `zgx::task::latest` to a task that reads it (`ARM=host`); each one
//! waits for the last to run, so neither coalesces.
//! `CASE=upsert`: `N` batches of 500 changed rows into a list of 10,000,
//! keyed by `id`: a module's `Shared::upsert` (`ARM=module`, through
//! `Api::shared_upsert`), or the host setting the whole list with those
//! rows changed (`ARM=host`, what a module that can't patch does). It
//! prints the time a batch takes too.
//! `CASE=place`: `N` batches each moving 500 rows of the 10,000 (every
//! 20th, each to just before the row 10 on), the module keeping the model:
//! posted as the moved rows placed (`ARM=module`, `Shared::upsert_at`), or
//! as the whole reordered list set (`ARM=host`, what a module did before).
//!
//! It prints nothing to compare by itself: count it at two `N`s under
//! cachegrind and take the difference over the difference of `N`
//! (instructions an operation), the setup the same in both:
//!
//!   cargo build --release -p zgx-script --features modules --example module_reach_bench
//!   ARM=module CASE=set N=100000 valgrind --tool=cachegrind --cache-sim=no target/release/examples/module_reach_bench

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zgx_script::{ScriptHost, Value, testing::TestApp};

mod reach {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use zgx_module::{Host, Value};

    zgx_module::register!("bench", |host: &Host| {
        let n = host.shared("n", Value::from(0.)).unwrap();
        host.function("bench.set", move |count| {
            for i in 0..count.num().unwrap_or(0.) as usize {
                n.set(Value::from(i as f64 + 1.)).unwrap();
            }
            Value::NIL
        });
        let rows = host.shared("rows", Value::NIL).unwrap();
        host.function("bench.upsert", move |count| {
            for i in 0..count.num().unwrap_or(0.) as usize {
                rows.upsert(
                    "id",
                    super::batch(i).map(|(id, title)| {
                        Value::record([
                            ("id", Value::from(id.as_str())),
                            ("title", Value::from(title.as_str())),
                        ])
                    }),
                )
                .unwrap();
            }
            Value::NIL
        });
        // The model, kept by the module: ids in order.
        let model = std::cell::RefCell::new(
            (0..super::LIST)
                .map(|i| format!("r{i}"))
                .collect::<Vec<_>>(),
        );
        let placed = host.shared("rows", Value::NIL).unwrap();
        host.function("bench.place", move |arg| {
            let whole = arg.field("whole").and_then(|w| w.bool()).unwrap_or(false);
            for i in 0..arg.field("n").and_then(|n| n.num()).unwrap_or(0.) as usize {
                let moves = super::moves(i);
                let mut model = model.borrow_mut();
                *model = super::moved(&model, &moves);
                if whole {
                    placed
                        .set(Value::list(model.iter().map(|id| super::row_of(id))))
                        .unwrap();
                } else {
                    let rows = moves
                        .iter()
                        .map(|(id, before)| (super::row_of(id), zgx_module::Place::Before(before)));
                    placed.upsert_at("id", rows).unwrap();
                }
            }
            Value::NIL
        });
        let ui = host.ui();
        host.function("bench.ui", move |_| {
            *UI.lock().unwrap() = Some(ui.clone());
            Value::NIL
        });
    });
    pub static UI: std::sync::Mutex<Option<zgx_module::Ui>> = std::sync::Mutex::new(None);
    /// How many posted jobs ran.
    pub static RAN: AtomicUsize = AtomicUsize::new(0);
    /// Posts one job, which counts as it runs.
    pub fn post(ui: &zgx_module::Ui) {
        ui.post(|_| {
            RAN.fetch_add(1, Ordering::Relaxed);
        });
    }
}

/// The list's length, and a batch's.
const LIST: usize = 10_000;
const BATCH: usize = 500;
/// Batch `i`'s rows: every 20th row from `i`, titled with `i`.
fn batch(i: usize) -> impl Iterator<Item = (String, String)> {
    (0..BATCH).map(move |j| (format!("r{}", (j * 20 + i) % LIST), format!("b{i}")))
}
/// Batch `i`'s moves: every 20th row from `i`, each to before the row 10
/// on (never one that moves).
fn moves(i: usize) -> Vec<(String, String)> {
    (0..BATCH)
        .map(|j| {
            (
                format!("r{}", (j * 20 + i) % LIST),
                format!("r{}", (j * 20 + i + 10) % LIST),
            )
        })
        .collect()
}
/// `order` with `moves` made: each moved row taken out and put just before
/// its anchor, as `upsert_at` puts it.
fn moved(order: &[String], moves: &[(String, String)]) -> Vec<String> {
    let moving: std::collections::HashSet<&str> = moves.iter().map(|(id, _)| id.as_str()).collect();
    let mut before: std::collections::HashMap<&str, Vec<&str>> = Default::default();
    for (id, anchor) in moves {
        before.entry(anchor.as_str()).or_default().push(id.as_str());
    }
    let mut next = Vec::with_capacity(order.len());
    for id in order {
        if moving.contains(id.as_str()) {
            continue;
        }
        for m in before.get(id.as_str()).into_iter().flatten() {
            next.push((*m).to_owned());
        }
        next.push(id.clone());
    }
    next
}
/// A module's row of the model, by id.
fn row_of(id: &str) -> zgx_module::Value {
    zgx_module::Value::record([
        ("id", zgx_module::Value::from(id)),
        ("title", zgx_module::Value::from("x")),
    ])
}
fn row(id: &str, title: &str) -> Value {
    Value::record([("id", Value::from(id)), ("title", Value::from(title))])
}

fn main() {
    let module = std::env::var("ARM").is_ok_and(|arm| arm == "module");
    let case = std::env::var("CASE").unwrap_or_else(|_| "set".into());
    let n: usize = std::env::var("N")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(100_000);
    let app = TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "bench", "return ui.column {}");
        host.shared("n", Value::Num(0.));
        host.shared(
            "rows",
            Value::List(zgx_script::Slice::from(
                (0..LIST)
                    .map(|i| row(&format!("r{i}"), "x"))
                    .collect::<Vec<_>>(),
            )),
        );
        // SAFETY: This program's own `register!` entry.
        unsafe { host.enter_module(reach::zgx_module_v1, &reach::zgx_module_manifest) }.unwrap();
        host
    });
    let call = |name: &str, arg: Value| app.host.registered_function(name).unwrap()(arg).unwrap();
    let started = std::time::Instant::now();
    match (case.as_str(), module) {
        ("place", _) => {
            call(
                "bench.place",
                Value::record([("n", Value::Num(n as f64)), ("whole", Value::Bool(!module))]),
            );
        }
        ("upsert", true) => {
            call("bench.upsert", Value::Num(n as f64));
        }
        ("upsert", false) => {
            let shared = app.host.shared("rows", Value::Nil);
            for i in 0..n {
                let Value::List(held) = shared.get() else {
                    unreachable!()
                };
                let mut next = held.to_vec();
                for (id, title) in batch(i) {
                    let at: usize = id[1..].parse().unwrap();
                    next[at] = row(&id, &title);
                }
                shared.set(Value::List(zgx_script::Slice::from(next)));
            }
        }
        ("set", true) => {
            call("bench.set", Value::Num(n as f64));
        }
        ("set", false) => {
            let shared = app.host.shared("n", Value::Num(0.));
            for i in 0..n {
                shared.set(Value::Num(i as f64 + 1.));
            }
        }
        ("post", true) => {
            call("bench.ui", Value::Nil);
            let ui = reach::UI.lock().unwrap().take().unwrap();
            let (go, wait) = std::sync::mpsc::channel::<()>();
            let (done, seen) = std::sync::mpsc::channel::<()>();
            let thread = std::thread::spawn(move || {
                for _ in 0..n {
                    wait.recv().unwrap();
                    reach::post(&ui);
                    done.send(()).unwrap();
                }
            });
            for i in 0..n {
                go.send(()).unwrap();
                seen.recv().unwrap();
                app.settle();
                assert_eq!(reach::RAN.load(Ordering::Relaxed), i + 1);
            }
            thread.join().unwrap();
        }
        ("post", false) => {
            let (sender, mut receiver) = zgx::task::latest::<usize>();
            let ran = Arc::new(AtomicUsize::new(0));
            {
                let ran = ran.clone();
                app.executor.borrow_mut().spawn(async move {
                    while receiver.recv().await.is_some() {
                        ran.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
            app.settle();
            let (go, wait) = std::sync::mpsc::channel::<()>();
            let (done, seen) = std::sync::mpsc::channel::<()>();
            let thread = std::thread::spawn(move || {
                for i in 0..n {
                    wait.recv().unwrap();
                    sender.send(i);
                    done.send(()).unwrap();
                }
            });
            for i in 0..n {
                go.send(()).unwrap();
                seen.recv().unwrap();
                app.settle();
                assert_eq!(ran.load(Ordering::Relaxed), i + 1);
            }
            thread.join().unwrap();
        }
        _ => panic!("CASE=set|post|upsert|place"),
    }
    let took = started.elapsed();
    println!(
        "{} {case}: {n}, {:.1} µs each",
        if module { "module" } else { "host" },
        took.as_secs_f64() * 1e6 / n.max(1) as f64
    );
}
