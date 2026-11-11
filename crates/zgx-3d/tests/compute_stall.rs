//! A driver stuck compiling a sandboxed shader past the deadline: that pass
//! fails, its worker is let go, and other owners' passes compile on a new
//! one. A file of its own: the deadline and the stall are the process's.
#![cfg(feature = "compute")]
use std::time::{Duration, Instant};
use zgx_3d::*;
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

fn block<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_stuck_sandboxed_compile_fails_its_pass_and_holds_no_other_up() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Some(adapter) = block(instance.request_adapter(&Default::default())).ok() else {
        assert!(std::env::var_os("ZGX_REQUIRE_GPU").is_none(), "no adapter");
        return;
    };
    if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        return;
    }
    let (device, queue) = block(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))
    .unwrap();
    // The worker held 5 s by a "driver"; the deadline 300 ms.
    stall_sandboxed_compiles(Duration::from_millis(300), Duration::from_secs(5));
    let world = World::new();
    let pass = || Compute::new("@compute @workgroup_size(1) fn main() {}").every_frame();
    let stuck = world.add_compute(pass().sandboxed("one")).unwrap();
    let mut painter = world.viewport();
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut draw = || {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut frame =
            ViewportFrame::new(&device, &queue, &mut encoder, &view, format, (8, 8), 1.);
        painter.render(&mut frame);
        queue.submit([encoder.finish()]);
        std::thread::sleep(Duration::from_millis(10));
    };
    let started = Instant::now();
    while !matches!(world.compute_status(stuck), Some(ComputeStatus::Failed(_))) {
        draw();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            world.compute_status(stuck)
        );
    }
    // Another owner's compiles at once, on a new worker: well before the
    // stuck one would have let it.
    let other = world.add_compute(pass().sandboxed("two")).unwrap();
    let started = Instant::now();
    while world.compute_status(other) != Some(ComputeStatus::Ready) {
        draw();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            world.compute_status(other)
        );
    }
}
