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
                n.set(Value::from(i as f64 + 1.));
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
        // SAFETY: This program's own `register!` entry.
        unsafe { host.enter_module(reach::zgx_module_v1, &reach::zgx_module_manifest) }.unwrap();
        host
    });
    let call = |name: &str, arg: Value| app.host.registered_function(name).unwrap()(arg).unwrap();
    match (case.as_str(), module) {
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
        _ => panic!("CASE=set|post"),
    }
    println!("{} {case}: {n}", if module { "module" } else { "host" });
}
