//! What a video's frames cost the CPU: `examples/bench` played on its stage,
//! each of `FRAMES` frames (860) stepped on the virtual clock and drawn whole,
//! nothing read back, `PASSES` times (3), each on a stage of its own. The instruction gate's `video` cell runs it on wgpu's
//! no-op backend (`ZGX_GPU_NOOP`): the clock, the script, layout and the
//! renderer's own work, no driver's.
//! Run: ZGX_GPU_NOOP=1 cargo run --release -p zgx-video --example video_bench
fn main() {
    let frames: u64 = std::env::var("FRAMES")
        .ok()
        .and_then(|frames| frames.parse().ok())
        .unwrap_or(860);
    let passes: u64 = std::env::var("PASSES")
        .ok()
        .and_then(|passes| passes.parse().ok())
        .unwrap_or(3);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/bench");
    for _ in 0..passes {
        let mut stage = zgx_video::ScriptStage::new(
            &dir,
            None,
            &serde_json::json!({}),
            1.,
            zgx::scene::Color(0, 0, 0, 255),
            zgx_video::Timing::default(),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let last = frames.min(stage.output_frames());
        for frame in 0..last {
            stage.step_to(frame).expect("a frame");
            stage.stage.render().expect("drawn");
        }
    }
}
