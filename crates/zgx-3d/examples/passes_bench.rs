//! fx_bench's `passes` scene, a binary of its own: a shader with a feedback
//! pass and tagged boxes as inputs. Its own binary, so the gate counts it
//! as a cell new where fx_bench has no such scene.
//! Run: cargo run --release -p zgx-3d --example passes_bench
#[allow(dead_code)]
#[path = "fx_bench.rs"]
mod fx_bench;

fn main() {
    fx_bench::run("passes".into());
}
