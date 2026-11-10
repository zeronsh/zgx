//! A frame's backdrop blur timed pass by pass (`debug_time_passes`): each
//! part reported in the order it ran, the pixels the same as untimed, and
//! nothing reported while off.
#![cfg(target_os = "macos")]
use zgx::{compose::prelude::*, style::rgb, widgets::Ui};
use zgx_gpu::GpuRenderer;

fn frame(gpu: &mut GpuRenderer) -> Vec<u8> {
    let mut ui = Ui::new(200., 120.);
    ui.mount(
        overlay()
            .w(200.)
            .h(120.)
            .child(column().children((0..6).map(|i| div().h(20.).bg(rgb(0x203040 + i * 0x101010)))))
            .child(
                div()
                    .w(160.)
                    .h(60.)
                    .translate(20., 30.)
                    .rounded(10.)
                    .bg(zgx::scene::Color(255, 255, 255, 50))
                    .backdrop_blur(12.),
            ),
    );
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    gpu.readback().unwrap()
}

#[test]
fn a_blurred_frame_is_timed_pass_by_pass_and_draws_the_same() {
    let mut gpu = GpuRenderer::new(400, 240).unwrap();
    gpu.set_scale_factor(2.);
    let untimed = frame(&mut gpu);
    assert!(gpu.debug_take_pass_times().is_empty(), "nothing while off");
    gpu.debug_time_passes(true);
    let timed = frame(&mut gpu);
    let parts = gpu.debug_take_pass_times();
    gpu.debug_time_passes(false);
    assert!(timed == untimed, "the pixels are the same timed");
    if gpu.debug_metal_frames() == 0 {
        return; // Drawn through wgpu: nothing to split.
    }
    let ran: Vec<_> = parts.iter().map(|(ran, _)| *ran).collect();
    assert_eq!(
        ran,
        [
            "frame before the blur",
            "blur reduce",
            "blur across",
            "blur down",
            "frame after"
        ]
    );
    assert!(
        parts.iter().all(|(_, us)| us.is_finite() && *us >= 0.),
        "{parts:?}"
    );
}
