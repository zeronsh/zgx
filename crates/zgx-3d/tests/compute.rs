//! The app's GPU data and compute passes: passes fill what they bind, run in
//! the order their resources say, at their rates, and keep the world drawing
//! only while they're due; reads come back within their budget, never
//! stalling a frame. Needs a GPU (`ZGX_REQUIRE_GPU`); Metal here, others on
//! their own machines.
#![cfg(feature = "compute")]
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

struct Window {
    ui: Ui,
    gpu: GpuRenderer,
    view: zgx::gpu_view::GpuView,
}
impl Window {
    fn new(world: &World) -> Option<Self> {
        Self::with_boxes(world, 1)
    }
    /// With the world's view shown in `boxes` boxes side by side.
    fn with_boxes(world: &World, boxes: usize) -> Option<Self> {
        let gpu = match GpuRenderer::new(32 * boxes as u32, 32) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let width = 32. * boxes as f32;
        let mut ui = Ui::new(width, 32.);
        let mut row = div().size(width, 32.).bg(Color(0, 0, 0, 255)).flex_row();
        for _ in 0..boxes {
            row = row.child(gpu_view("World", world.view()).size(32., 32.));
        }
        ui.mount(row);
        Some(Self {
            ui,
            gpu,
            view: world.view().clone(),
        })
    }
    /// A display refresh: an animating view gets a frame, as a window's
    /// frame clock gives it one (a headless window has none).
    fn draw(&mut self) {
        if self.view.animating() {
            self.view.invalidate();
        }
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap();
    }
    /// Frames drawn until `read` resolves (at most `most`), a moment apart
    /// as a display's would be.
    fn until(&mut self, read: &Readback, most: usize) -> usize {
        for frame in 1..=most {
            self.draw();
            if read.is_ready() {
                return frame;
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        panic!("not read in {most} frames");
    }
    /// Frames drawn until `world`'s pass `id` has compiled.
    /// (Up to 10 s of wall time: a software renderer under load compiles
    /// slowly, and a test isn't about how fast.)
    fn compiled(&mut self, world: &World, id: ComputeId) {
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(10) {
            self.view.invalidate();
            self.draw();
            if world.compute_status(id) == Some(ComputeStatus::Ready) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!(
            "not compiled: {:?} after {} frames, animating {}",
            world.compute_status(id),
            world.frames_drawn(),
            world.view().animating()
        );
    }
}

/// Painters drawn on a device of the test's own at frame times it
/// chooses: what's counted is frame time, however fast this machine draws.
struct TimedFrames {
    device: zgx_gpu::wgpu::Device,
    queue: zgx_gpu::wgpu::Queue,
    view: zgx_gpu::wgpu::TextureView,
    start: std::time::Instant,
}
impl TimedFrames {
    fn device() -> Option<(zgx_gpu::wgpu::Device, zgx_gpu::wgpu::Queue)> {
        use zgx_gpu::wgpu;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let found = block(instance.request_adapter(&Default::default()))
            .ok()
            .and_then(|adapter| block(adapter.request_device(&Default::default())).ok());
        assert!(
            found.is_some() || std::env::var_os("ZGX_REQUIRE_GPU").is_none(),
            "no device"
        );
        found
    }
    fn new((device, queue): (zgx_gpu::wgpu::Device, zgx_gpu::wgpu::Queue)) -> Self {
        use zgx_gpu::wgpu;
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
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            view: target.create_view(&Default::default()),
            device,
            queue,
            start: std::time::Instant::now(),
        }
    }
    /// One frame at `ms` of frame time: each of `painters` renders it, in
    /// turn, as a world's view shown in several boxes is (or as several
    /// viewports of one world are).
    fn draw(&mut self, painters: &mut [&mut dyn zgx_gpu::viewport::Viewport], ms: u64) {
        use zgx_gpu::viewport::ViewportFrame;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for painter in painters {
            let mut frame = ViewportFrame::new(
                &self.device,
                &self.queue,
                &mut encoder,
                &self.view,
                zgx_gpu::wgpu::TextureFormat::Bgra8Unorm,
                (8, 8),
                1.,
            );
            frame.time = self.start + std::time::Duration::from_millis(ms);
            painter.render(&mut frame);
        }
        self.queue.submit([encoder.finish()]);
        let _ = self
            .device
            .poll(zgx_gpu::wgpu::PollType::wait_indefinitely());
    }
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytemuck::pod_collect_to_vec(bytes)
}

const FILL: &str = "
@group(0) @binding(0) var out: texture_storage_2d<r32uint, write>;

@compute @workgroup_size(8, 8)
fn fill(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(out);
    if id.x < size.x && id.y < size.y {
        textureStore(out, id.xy, vec4u(id.x + id.y * 1000u, 0u, 0u, 0u));
    }
}
";

#[test]
fn a_pass_fills_a_texture_that_reads_back_as_written() {
    let world = World::new();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    // 50 wide: rows of 200 bytes, copied padded to 256 and given back packed.
    let texture = world
        .add_texture(TextureDesc::storage(50, 20, Format::R32Uint))
        .unwrap();
    let pass = world
        .add_compute(
            Compute::new(FILL)
                .texture("out", &texture, Access::Write)
                .threads([50, 20, 1]),
        )
        .unwrap();
    window.compiled(&world, pass);
    let read = texture.read();
    window.until(&read, 100);
    let texels = words(&read.try_take().unwrap().unwrap());
    let expected: Vec<u32> = (0..20)
        .flat_map(|y| (0..50).map(move |x| x + y * 1000))
        .collect();
    assert_eq!(texels, expected);
}

#[test]
fn a_buffer_updated_in_place_reads_back_its_values() {
    let world = World::new();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let values: Vec<u32> = (0..256).collect();
    let buffer = world.add_buffer(BufferDesc::from_slice(&values)).unwrap();
    let pass = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> values: array<u32>;
                 @compute @workgroup_size(64)
                 fn double(@builtin(global_invocation_id) id: vec3u) {
                     values[id.x] = values[id.x] * 2u + 1u;
                 }",
            )
            .buffer("values", &buffer, Access::ReadWrite)
            .threads([256, 1, 1]),
        )
        .unwrap();
    window.compiled(&world, pass);
    // On demand: it ran once when ready, and runs again when asked.
    let read = buffer.read(4..16);
    window.until(&read, 100);
    assert_eq!(words(&read.try_take().unwrap().unwrap()), [3, 5, 7]);
    world.run_compute(pass);
    let read = buffer.read(0..1024);
    window.until(&read, 100);
    let doubled = words(&read.try_take().unwrap().unwrap());
    assert_eq!(doubled[..3], [3, 7, 11]);
    assert_eq!(doubled[255], (255 * 2 + 1) * 2 + 1);
    // A write from the CPU lands before the next frame's passes.
    buffer.write(0, &[100u32]).unwrap();
    let read = buffer.read(0..4);
    window.until(&read, 100);
    assert_eq!(words(&read.try_take().unwrap().unwrap()), [100]);
    assert_eq!(buffer.write(1, &[1u32]), Err(WriteError::Unaligned));
    assert_eq!(buffer.write(1024, &[1u32]), Err(WriteError::OutOfRange));
    assert_eq!(
        buffer.read(0..2048).try_take(),
        Some(Err(ReadError::OutOfRange))
    );
}

#[test]
fn a_pass_writing_a_resource_whole_runs_before_an_earlier_one_reading_it() {
    let world = World::new();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let source = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
    let copy = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
    // Added first: copies `source` into `copy`.
    let reader = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read> source: array<u32>;
                 @group(0) @binding(1) var<storage, read_write> copy: array<u32>;
                 @compute @workgroup_size(1) fn main() { copy[0] = source[0]; }",
            )
            .buffer("source", &source, Access::Read)
            .buffer("copy", &copy, Access::Write),
        )
        .unwrap();
    // Added second, writing `source` whole: it runs first.
    let writer = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> source: array<u32>;
                 @compute @workgroup_size(1) fn main() { source[0] = 7u; }",
            )
            .buffer("source", &source, Access::Write),
        )
        .unwrap();
    // Each runs once, in one frame: held back until both are compiled. In
    // the order added, the copy would be 0.
    world.set_compute_enabled(reader, false);
    world.set_compute_enabled(writer, false);
    window.compiled(&world, reader);
    window.compiled(&world, writer);
    world.set_compute_enabled(reader, true);
    world.set_compute_enabled(writer, true);
    let read = copy.read(0..4);
    window.until(&read, 100);
    assert_eq!(words(&read.try_take().unwrap().unwrap()), [7]);

    // A third reading `copy` and writing `source` whole, where `reader`
    // reads `source` and writes `copy` whole: no order holds.
    let cycle = world.add_compute(
        Compute::new(
            "@group(0) @binding(0) var<storage, read> copy: array<u32>;
             @group(0) @binding(1) var<storage, read_write> source: array<u32>;
             @compute @workgroup_size(1) fn main() { source[0] = copy[0]; }",
        )
        .buffer("copy", &copy, Access::Read)
        .buffer("source", &source, Access::Write),
    );
    assert_eq!(cycle, Err(ComputeError::Cycle));
}

#[test]
fn reads_take_frames_by_the_budget_and_never_stall_one() {
    let world = World::new();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let small = world.add_buffer(BufferDesc::storage::<u32>(16)).unwrap();
    let large = world.add_buffer(BufferDesc::bytes(256 << 10)).unwrap();
    window.draw();
    // Frames counted from below only: the budget sets the fewest a read
    // takes; how many more depends on how fast this GPU finishes them.
    // A small read: copied in a frame, mapped in a later one.
    let read = small.read(0..64);
    let frames = window.until(&read, 1000);
    assert!(frames >= 2, "{frames} frames");
    assert_eq!(read.try_take().unwrap().unwrap(), vec![0; 64]);
    // 256 KiB at 64 KiB a frame: four frames copying, then mapped.
    let read = large.read(0..256 << 10);
    let frames = window.until(&read, 1000);
    assert!(frames >= 5, "{frames} frames");
    assert_eq!(read.try_take().unwrap().unwrap().len(), 256 << 10);
    // A budget of the whole: copied in one frame.
    world.set_readback_budget(256 << 10);
    let read = large.read(0..256 << 10);
    let frames = window.until(&read, 1000);
    assert!(frames >= 2, "{frames} frames");
    // A dropped buffer's read says so.
    let read = small.read(0..4);
    drop(small);
    window.until(&read, 100);
    assert_eq!(read.try_take(), Some(Err(ReadError::Dropped)));
}

#[test]
fn a_world_draws_while_a_pass_is_due_and_settles_when_none_is() {
    let world = World::new();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let count = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
    let pass = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                 @compute @workgroup_size(1) fn main() { count[0] += 1u; }",
            )
            .buffer("count", &count, Access::ReadWrite)
            .every_frame(),
        )
        .unwrap();
    assert!(world.view().animating(), "an every-frame pass is due");
    window.compiled(&world, pass);
    let before = world.frames_drawn();
    for _ in 0..10 {
        window.draw();
    }
    assert_eq!(world.frames_drawn(), before + 10, "it keeps drawing");
    let read = count.read(0..4);
    window.until(&read, 100);
    let runs = words(&read.try_take().unwrap().unwrap())[0];
    assert!(runs >= 10, "ran {runs} times");

    world.set_compute_enabled(pass, false);
    window.draw();
    assert!(!world.view().animating(), "nothing due: settled");
    let before = world.frames_drawn();
    for _ in 0..10 {
        window.draw();
    }
    assert_eq!(
        world.frames_drawn(),
        before,
        "a settled world draws nothing"
    );

    // On demand: one frame for the run, then settled.
    let once = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                 @compute @workgroup_size(1) fn main() { count[0] = 0u; }",
            )
            .buffer("count", &count, Access::ReadWrite)
            .on_demand(),
        )
        .unwrap();
    window.compiled(&world, once);
    window.draw();
    assert!(!world.view().animating());
    let before = world.frames_drawn();
    for _ in 0..10 {
        window.draw();
    }
    assert_eq!(world.frames_drawn(), before);
    let read = count.read(0..4);
    window.until(&read, 100);
    assert_eq!(words(&read.try_take().unwrap().unwrap()), [0]);
}

#[test]
fn a_rated_pass_runs_at_its_rate_of_frame_time() {
    // Frames at times of the test's own: what it counts is frame time,
    // however fast this machine draws.
    let Some(device) = TimedFrames::device() else {
        return;
    };
    let world = World::new();
    let count = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
    let pass = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                 @compute @workgroup_size(1) fn main() { count[0] += 1u; }",
            )
            .buffer("count", &count, Access::ReadWrite)
            .at_rate(20.),
        )
        .unwrap();
    let mut frames = TimedFrames::new(device);
    let mut painter = world.viewport();
    let mut draw = |ms: u64| frames.draw(&mut [&mut painter], ms);
    // Compiled at frame time 0 (it runs once there when ready).
    for _ in 0..1000 {
        draw(0);
        if world.compute_status(pass) == Some(ComputeStatus::Ready) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // It runs once at the next frame time (a frame at the same time only
    // draws).
    draw(1);
    // Counted from none: frames 4 ms of frame time apart for half a
    // second. Due at 0.05, 0.1, ... 0.5: ten runs, not one a frame.
    count.write(0, &[0u32]).unwrap();
    let mut ms = 1;
    while ms < 500 {
        ms += 4;
        draw(ms);
    }
    world.set_compute_enabled(pass, false);
    let read = count.read(0..4);
    for _ in 0..20 {
        ms += 4;
        draw(ms);
        if read.is_ready() {
            break;
        }
    }
    let runs = words(&read.try_take().unwrap().unwrap())[0];
    assert!((9..=10).contains(&runs), "{runs} runs in 125 frames");
}

#[test]
fn a_pass_is_checked_when_added_with_the_apps_own_lines() {
    let world = World::new();
    let buffer = world.add_buffer(BufferDesc::storage::<u32>(4)).unwrap();
    let texture = world
        .add_texture(TextureDesc::storage(4, 4, Format::Rgba8Unorm))
        .unwrap();
    let Err(ComputeError::Shader(message)) = world.add_compute(Compute::new(
        "@group(0) @binding(0) var<storage, read_write> b: array<u32>;\n\n@compute @workgroup_size(1) fn main() { b[0] = nope; }",
    )) else {
        panic!("an unknown name is an error")
    };
    assert!(message.contains(":3:"), "at the app's line 3: {message}");
    let wrong_access = world.add_compute(
        Compute::new(
            "@group(0) @binding(0) var<storage, read_write> b: array<u32>;
             @compute @workgroup_size(1) fn main() { b[0] = 1u; }",
        )
        .buffer("b", &buffer, Access::Read),
    );
    assert!(matches!(wrong_access, Err(ComputeError::Binding { .. })));
    let unbound = world.add_compute(Compute::new(
        "@group(0) @binding(0) var<storage, read_write> b: array<u32>;
         @compute @workgroup_size(1) fn main() { b[0] = 1u; }",
    ));
    assert!(matches!(unbound, Err(ComputeError::Binding { .. })));
    let format = world.add_compute(
        Compute::new(
            "@group(0) @binding(0) var t: texture_storage_2d<r32float, write>;
             @compute @workgroup_size(1) fn main() { textureStore(t, vec2u(0u), vec4f(1.)); }",
        )
        .texture("t", &texture, Access::Write),
    );
    assert!(matches!(format, Err(ComputeError::Binding { .. })));
    let typo = world.add_compute(
        Compute::new("@compute @workgroup_size(1) fn main() {}").buffer("b", &buffer, Access::Read),
    );
    assert!(matches!(typo, Err(ComputeError::Binding { .. })));
    let large = world.add_compute(
        Compute::new("@compute @workgroup_size(1) fn main() {}").dispatch([70_000, 1, 1]),
    );
    assert!(matches!(large, Err(ComputeError::Limits(_))));
    let wide = world.add_compute(Compute::new("@compute @workgroup_size(512) fn main() {}"));
    assert!(matches!(wide, Err(ComputeError::Limits(_))));
    let two = world.add_compute(Compute::new(
        "@compute @workgroup_size(1) fn a() {} @compute @workgroup_size(1) fn b() {}",
    ));
    assert!(matches!(two, Err(ComputeError::Entry(_))));
    assert!(
        world
            .add_compute(
                Compute::new(
                    "@compute @workgroup_size(1) fn a() {} @compute @workgroup_size(1) fn b() {}",
                )
                .entry("b")
            )
            .is_ok()
    );
}

#[test]
fn the_apps_data_counts_against_its_cap_while_its_handles_live() {
    let world = World::new();
    assert_eq!(world.memory().app(), 0);
    let buffer = world.add_buffer(BufferDesc::bytes(1000)).unwrap();
    let texture = world
        .add_texture(TextureDesc::storage(16, 16, Format::Rgba16Float))
        .unwrap();
    let memory = world.memory();
    assert_eq!(
        (memory.app_buffers, memory.app_textures),
        (1000, 16 * 16 * 8)
    );
    world.set_memory_cap(4096);
    assert_eq!(
        world.add_buffer(BufferDesc::bytes(2048)),
        Err(GpuDataError::OverBudget {
            asked: 2048,
            used: 3048,
            cap: 4096
        })
    );
    drop(texture);
    assert_eq!(world.memory().app(), 1000);
    assert!(world.add_buffer(BufferDesc::bytes(2048)).is_ok());
    // A pass binding it keeps it; removing the pass lets it go.
    let pass = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> b: array<u32>;
                 @compute @workgroup_size(1) fn main() { b[0] = 1u; }",
            )
            .buffer("b", &buffer, Access::Write),
        )
        .unwrap();
    drop(buffer);
    assert_eq!(world.memory().app(), 1000);
    world.remove_compute(pass);
    assert_eq!(world.memory().app(), 0);
    assert_eq!(world.compute_status(pass), None);
    assert!(matches!(
        world.add_buffer(BufferDesc::bytes(0)),
        Err(GpuDataError::Size(0))
    ));
}

#[test]
fn a_buffer_too_large_to_count_is_refused_not_a_panic() {
    let world = World::new();
    assert!(matches!(
        world.add_buffer(BufferDesc::storage::<[f32; 4]>(usize::MAX)),
        Err(GpuDataError::Size(_))
    ));
    assert!(matches!(
        world.add_buffer(BufferDesc::bytes(u64::MAX)),
        Err(GpuDataError::Size(_))
    ));
    assert!(matches!(
        world.add_buffer(BufferDesc::bytes(u64::MAX - 1)),
        Err(GpuDataError::Size(_))
    ));
}

#[test]
fn reads_nobody_holds_are_dropped_and_the_queue_is_bounded() {
    let world = World::new();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let buffer = world.add_buffer(BufferDesc::bytes(1 << 20)).unwrap();
    window.draw();
    // A 1 MiB read a frame at 64 KiB a frame, each dropped for the next:
    // the queue holds the newest, not one more a frame.
    let mut last = None;
    for _ in 0..60 {
        last = Some(buffer.read(0..1 << 20));
        window.draw();
    }
    let (count, bytes) = world.readbacks_queued();
    assert!(
        count <= 2 && bytes <= 2 << 20,
        "{count} reads, {bytes} bytes queued"
    );
    let read = last.unwrap();
    window.until(&read, 100);
    assert_eq!(read.try_take().unwrap().unwrap().len(), 1 << 20);
    // Held, reads past the cap (64 frames' budget: 4 MiB) are refused at
    // once; the queue stays within it.
    let held: Vec<Readback> = (0..10).map(|_| buffer.read(0..1 << 20)).collect();
    let refused = held
        .iter()
        .filter(|r| r.try_take() == Some(Err(ReadError::Busy)))
        .count();
    assert_eq!(refused, 6, "four fit in 4 MiB");
    assert!(world.readbacks_queued().1 <= 4 << 20);
    // A read larger than the cap still goes when nothing is queued.
    drop(held);
    window.draw();
    world.set_readback_budget(1 << 10);
    assert_eq!(world.readbacks_queued().0, 0, "dropped reads are let go");
    let read = buffer.read(0..1 << 20);
    assert!(!read.is_ready());
}

#[test]
fn a_large_texture_reads_back_whole_a_band_a_frame() {
    let world = World::new();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    world.set_readback_budget(256 << 10);
    let texture = world
        .add_texture(TextureDesc::storage(1024, 1024, Format::R32Uint))
        .unwrap();
    let pass = world
        .add_compute(
            Compute::new(FILL)
                .texture("out", &texture, Access::Write)
                .threads([1024, 1024, 1]),
        )
        .unwrap();
    window.compiled(&world, pass);
    let read = texture.read();
    let frames = window.until(&read, 200);
    assert!(frames >= 16, "4 MiB at 256 KiB a frame: {frames} frames");
    let texels = words(&read.try_take().unwrap().unwrap());
    assert_eq!(texels.len(), 1024 * 1024);
    for (at, &texel) in texels.iter().enumerate() {
        let (x, y) = ((at % 1024) as u32, (at / 1024) as u32);
        assert_eq!(texel, x + y * 1000, "at {x}, {y}");
    }
}

#[test]
fn writes_waiting_for_a_frame_hold_at_most_the_buffers_bytes() {
    let world = World::new();
    let buffer = world.add_buffer(BufferDesc::bytes(4096)).unwrap();
    // No frame comes (no window): 10,000 writes over one another and
    // beside one another hold no more than the buffer.
    for i in 0..10_000u32 {
        buffer.write(u64::from(i % 1024) * 4, &[i]).unwrap();
        buffer.write(0, &[i; 16]).unwrap();
    }
    assert!(world.writes_queued() <= 4096, "{}", world.writes_queued());
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    // The last write to each place is what the buffer holds.
    let read = buffer.read(0..4096);
    window.until(&read, 100);
    let words = words(&read.try_take().unwrap().unwrap());
    assert_eq!(words[..16], [9999; 16]);
    let last = |place: u32| (0..10_000u32).filter(|i| i % 1024 == place).max().unwrap();
    assert_eq!(words[16], last(16));
    assert_eq!(words[1023], last(1023));
}

/// A frame of `world` drawn on a no-op device of `limits`: its caps read.
fn drawn_on(world: &World, limits: zgx_gpu::wgpu::Limits) {
    use zgx_gpu::{
        viewport::{Viewport, ViewportFrame},
        wgpu,
    };
    let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    });
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
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut frame = ViewportFrame::new(&device, &queue, &mut encoder, &view, format, (8, 8), 1.);
    world.viewport().render(&mut frame);
    queue.submit([encoder.finish()]);
}

#[test]
fn gpu_data_is_held_to_the_devices_own_limits_once_known() {
    let world = World::new();
    drawn_on(
        &world,
        zgx_gpu::wgpu::Limits {
            max_texture_dimension_2d: 256,
            max_buffer_size: 1 << 20,
            ..Default::default()
        },
    );
    assert_eq!(world.caps().unwrap().max_texture, 256);
    assert!(matches!(
        world.add_texture(TextureDesc::storage(512, 16, Format::Rgba8Unorm)),
        Err(GpuDataError::Size(_))
    ));
    assert!(
        world
            .add_texture(TextureDesc::storage(256, 16, Format::Rgba8Unorm))
            .is_ok()
    );
    assert!(matches!(
        world.add_buffer(BufferDesc::bytes(2 << 20)),
        Err(GpuDataError::Size(_))
    ));
    // No compute (tier B): textures only passes write are refused, as the
    // passes are.
    let world = World::new();
    drawn_on(
        &world,
        zgx_gpu::wgpu::Limits {
            max_compute_workgroups_per_dimension: 0,
            max_compute_invocations_per_workgroup: 0,
            ..zgx_gpu::wgpu::Limits::downlevel_defaults()
        },
    );
    assert!(!world.caps().unwrap().compute);
    assert_eq!(
        world.add_texture(TextureDesc::storage(16, 16, Format::Rgba8Unorm)),
        Err(GpuDataError::Unsupported)
    );
    assert_eq!(
        world.add_compute(Compute::new("@compute @workgroup_size(1) fn main() {}")),
        Err(ComputeError::Unsupported)
    );
}

#[test]
fn passes_compile_on_one_thread_however_many_there_are() {
    let world = World::new();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let passes: Vec<ComputeId> = (0..200)
        .map(|i| {
            world
                .add_compute(Compute::new(format!(
                    "@compute @workgroup_size(1) fn main() {{ let x = {i}u; }}"
                )))
                .unwrap()
        })
        .collect();
    for pass in &passes {
        window.compiled(&world, *pass);
    }
    assert!(
        zgx_3d::compile_threads() <= 2,
        "one worker for every pass, and one for sandboxed code"
    );
}

#[test]
fn a_pass_past_the_frames_invocations_is_refused_when_added() {
    let world = World::new();
    // 65,535 cubed workgroups of one: far past 2^26 a frame.
    let huge = world.add_compute(
        Compute::new("@compute @workgroup_size(1) fn main() {}").dispatch([65_535, 65_535, 65_535]),
    );
    assert!(matches!(huge, Err(ComputeError::Limits(_))), "{huge:?}");
    world.set_compute_budget(ComputeBudget {
        invocations: 1 << 10,
        ..ComputeBudget::default()
    });
    let over = world.add_compute(
        Compute::new("@compute @workgroup_size(64) fn main() {}").dispatch([17, 1, 1]),
    );
    assert!(matches!(over, Err(ComputeError::Limits(_))), "{over:?}");
    assert!(
        world
            .add_compute(
                Compute::new("@compute @workgroup_size(64) fn main() {}").dispatch([16, 1, 1])
            )
            .is_ok()
    );
}

#[test]
fn a_sandboxed_pass_needs_a_device_that_times_it() {
    // A device asked for no timestamps (`World::new` asks only devices made
    // by `run` or after it; this one is the test's own): once it's known, a
    // pass a plugin adds is refused rather than run unbounded.
    let Some(device) = TimedFrames::device() else {
        return;
    };
    let mut frames = TimedFrames::new(device);
    let world = World::new();
    let mut painter = world.viewport();
    frames.draw(&mut [&mut painter], 0);
    assert!(!world.caps().unwrap().timestamps);
    let pass = world
        .add_compute(Compute::new("@compute @workgroup_size(1) fn main() {}").sandboxed("plugin"));
    assert_eq!(pass, Err(ComputeError::Untimed));
    assert!(
        world
            .add_compute(Compute::new("@compute @workgroup_size(1) fn main() {}"))
            .is_ok()
    );
}

/// A window whose device times passes (made after `World::new`, which asks
/// for timestamps): on Metal it must; elsewhere, without them, nothing is
/// checked.
fn timed_window(world: &World) -> Option<Window> {
    let mut window = Window::new(world)?;
    window.draw();
    let caps = world.caps().expect("read with the first frame");
    if !caps.timestamps {
        assert!(
            !(cfg!(target_os = "macos") && std::env::var_os("ZGX_REQUIRE_GPU").is_some()),
            "Metal times passes: {caps:?}"
        );
        eprintln!("no timestamps on this device: not checked");
        return None;
    }
    Some(window)
}

/// A sandboxed pass of `owner` adding 1 to its own counter each run, with
/// a loop (so its work is the sandbox's whole: `shader_sandbox::Limits`).
fn counting_sandboxed(world: &World, owner: &str) -> (ComputeId, GpuBuffer) {
    let count = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
    let pass = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                 @compute @workgroup_size(64)
                 fn main(@builtin(global_invocation_id) id: vec3u) {
                     var n = 0u;
                     loop { if n >= 4u { break; } n += 1u; }
                     if id.x == 0u { count[0] += n / 4u; }
                 }",
            )
            .buffer("count", &count, Access::ReadWrite)
            .threads([64, 1, 1])
            .sandboxed(owner)
            .every_frame(),
        )
        .unwrap();
    (pass, count)
}

#[test]
fn on_a_device_that_times_them_an_owner_runs_two_passes_worth_a_frame() {
    let world = World::new();
    let Some(mut window) = timed_window(&world) else {
        return;
    };
    // Time no limit: the work is what's shared.
    world.set_compute_budget(ComputeBudget {
        sandboxed_gpu_ms: 1e9,
        owner_gpu_ms: 1e9,
        ..ComputeBudget::default()
    });
    // Each pass's work is the sandbox's whole: an owner runs two a frame,
    // the frame four. Three of one owner's and one of another's.
    let passes: Vec<_> = ["a", "a", "a", "b"]
        .into_iter()
        .map(|owner| counting_sandboxed(&world, owner))
        .collect();
    for (pass, _) in &passes {
        window.compiled(&world, *pass);
    }
    for (_, count) in &passes {
        count.write(0, &[0u32]).unwrap();
    }
    for _ in 0..12 {
        window.draw();
    }
    let reads: Vec<_> = passes.iter().map(|(_, count)| count.read(0..4)).collect();
    window.until(reads.last().unwrap(), 100);
    let runs: Vec<u32> = reads
        .iter()
        .map(|read| words(&read.try_take().unwrap().unwrap())[0])
        .collect();
    let (a, b) = (runs[0] + runs[1] + runs[2], runs[3]);
    assert!(b >= 12, "{runs:?}: the other owner ran each frame");
    assert_eq!(a, 2 * b, "{runs:?}: two of the first owner's a frame");
    // In turn: none of its three starved.
    assert!(runs[..3].iter().all(|&n| n + 1 >= a / 3), "{runs:?}");
    for (pass, _) in &passes {
        assert_eq!(world.compute_status(*pass), Some(ComputeStatus::Ready));
    }
}

#[test]
fn on_a_device_that_times_them_passes_over_their_budgets_are_turned_off() {
    // A budget of a nanosecond: no pass that does anything is within it,
    // however fast the GPU (only that the time is measured is relied on).
    let world = World::new();
    let Some(mut window) = timed_window(&world) else {
        return;
    };
    world.set_compute_budget(ComputeBudget {
        sandboxed_gpu_ms: 1e-6,
        owner_gpu_ms: 1e9,
        ..ComputeBudget::default()
    });
    let (sandboxed, _) = counting_sandboxed(&world, "plugin");
    // The app's own pass beside it: its budget is the app's, not this.
    let (own, _) = {
        let count = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
        let pass = world
            .add_compute(
                Compute::new(
                    "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                     @compute @workgroup_size(1) fn main() { count[0] += 1u; }",
                )
                .buffer("count", &count, Access::ReadWrite)
                .every_frame(),
            )
            .unwrap();
        (pass, count)
    };
    window.compiled(&world, sandboxed);
    let off = |world: &World, window: &mut Window, pass| {
        for _ in 0..100 {
            window.draw();
            if matches!(
                world.compute_status(pass),
                Some(ComputeStatus::OverBudget { .. })
            ) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        false
    };
    assert!(
        off(&world, &mut window, sandboxed),
        "{:?}",
        world.compute_status(sandboxed)
    );
    assert_eq!(world.compute_status(own), Some(ComputeStatus::Ready));
    // An owner over its budget: all its passes turned off.
    let world = World::new();
    let Some(mut window) = timed_window(&world) else {
        return;
    };
    world.set_compute_budget(ComputeBudget {
        sandboxed_gpu_ms: 1e9,
        owner_gpu_ms: 1e-6,
        ..ComputeBudget::default()
    });
    let (first, _) = counting_sandboxed(&world, "plugin");
    let (second, _) = counting_sandboxed(&world, "plugin");
    window.compiled(&world, first);
    window.compiled(&world, second);
    assert!(
        off(&world, &mut window, first),
        "{:?}",
        world.compute_status(first)
    );
    assert!(
        matches!(
            world.compute_status(second),
            Some(ComputeStatus::OverBudget { .. })
        ),
        "{:?}",
        world.compute_status(second)
    );
}

/// A device of this machine's with timestamps, if it has them.
fn timed_device() -> Option<(zgx_gpu::wgpu::Device, zgx_gpu::wgpu::Queue)> {
    use zgx_gpu::wgpu;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = block(instance.request_adapter(&Default::default())).ok()?;
    if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        eprintln!("no timestamps on {}: not checked", adapter.get_info().name);
        return None;
    }
    block(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))
    .ok()
}

fn block<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn a_pass_over_its_gpu_budget_is_turned_off_and_one_within_isnt() {
    // Frames' times charged as the device's timestamps would charge them:
    // judged on these numbers, not on how busy this machine's GPU is.
    let world = World::new();
    world.set_compute_budget(ComputeBudget {
        gpu_ms: 0.5,
        frames: 3,
        ..ComputeBudget::default()
    });
    let pass = || Compute::new("@compute @workgroup_size(1) fn main() {}").every_frame();
    let (heavy, light, alternating) = (
        world.add_compute(pass()).unwrap(),
        world.add_compute(pass()).unwrap(),
        world.add_compute(pass()).unwrap(),
    );
    let over = |id| {
        matches!(
            world.compute_status(id),
            Some(ComputeStatus::OverBudget { .. })
        )
    };
    for frame in 0..5 {
        let slow = if frame % 2 == 0 { 2.0 } else { 0.1 };
        world.time_compute_frame(&[(heavy, 2.0), (light, 0.1), (alternating, slow)]);
        assert_eq!(over(heavy), frame >= 2, "frame {frame}");
        assert_eq!(over(alternating), frame >= 4, "frame {frame}");
    }
    assert!(!over(light));
    assert_eq!(
        world.compute_status(heavy),
        Some(ComputeStatus::OverBudget { gpu_ms: 2.0 })
    );
}

/// The same on a GPU's own timestamps: a pass of milliseconds against a
/// half-millisecond budget. Needs a GPU with timestamps and quiet enough
/// that an empty pass stays under 0.5 ms (not llvmpipe or WARP under load):
/// `ZGX_REQUIRE_GPU=1 cargo test -p zgx-3d --features compute --test compute -- --ignored`.
#[test]
#[ignore = "real GPU timestamps: run on a quiet GPU with --ignored"]
fn a_pass_over_its_gpu_budget_frames_running_is_turned_off() {
    use zgx_gpu::{
        viewport::{Viewport, ViewportFrame},
        wgpu,
    };
    let Some((device, queue)) = timed_device() else {
        return;
    };
    let world = World::new();
    world.set_compute_budget(ComputeBudget {
        gpu_ms: 0.5,
        frames: 3,
        ..ComputeBudget::default()
    });
    let sink = world
        .add_buffer(BufferDesc::storage::<f32>(65_536))
        .unwrap();
    // About 1.3 billion multiply-adds: milliseconds on any GPU.
    let heavy = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> sink: array<f32>;
                 @compute @workgroup_size(64)
                 fn main(@builtin(global_invocation_id) id: vec3u) {
                     var x = f32(id.x);
                     for (var i = 0u; i < 20000u; i++) { x = x * 1.0001 + 0.5; }
                     sink[id.x] = x;
                 }",
            )
            .buffer("sink", &sink, Access::Write)
            .threads([65_536, 1, 1])
            .every_frame(),
        )
        .unwrap();
    let light = world
        .add_compute(Compute::new("@compute @workgroup_size(1) fn main() {}").every_frame())
        .unwrap();
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
    for _ in 0..200 {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut frame =
            ViewportFrame::new(&device, &queue, &mut encoder, &view, format, (8, 8), 1.);
        painter.render(&mut frame);
        queue.submit([encoder.finish()]);
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if matches!(
            world.compute_status(heavy),
            Some(ComputeStatus::OverBudget { .. })
        ) {
            break;
        }
    }
    let Some(ComputeStatus::OverBudget { gpu_ms }) = world.compute_status(heavy) else {
        panic!("still {:?}", world.compute_status(heavy));
    };
    assert!(gpu_ms > 0.5, "{gpu_ms} ms");
    assert_eq!(world.compute_status(light), Some(ComputeStatus::Ready));
    assert!(world.view().animating(), "the light pass still runs");
}

#[test]
fn passes_past_a_frames_invocations_take_turns() {
    let world = World::new();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    // Room for one of the two 64-invocation passes a frame: neither starves.
    world.set_compute_budget(ComputeBudget {
        invocations: 100,
        ..ComputeBudget::default()
    });
    let counts = world.add_buffer(BufferDesc::storage::<u32>(2)).unwrap();
    let passes: Vec<ComputeId> = (0..2)
        .map(|i| {
            world
                .add_compute(
                    Compute::new(format!(
                        "@group(0) @binding(0) var<storage, read_write> counts: array<atomic<u32>>;
                         @compute @workgroup_size(64)
                         fn main(@builtin(local_invocation_index) i: u32) {{
                             if i == 0u {{ atomicAdd(&counts[{i}], 1u); }}
                         }}"
                    ))
                    .buffer("counts", &counts, Access::ReadWrite)
                    .every_frame(),
                )
                .unwrap()
        })
        .collect();
    for pass in &passes {
        window.compiled(&world, *pass);
    }
    for _ in 0..20 {
        window.draw();
    }
    let read = counts.read(0..8);
    window.until(&read, 100);
    let ran = words(&read.try_take().unwrap().unwrap());
    assert!(
        ran[0] >= 8 && ran[1] >= 8,
        "each ran about half the frames: {ran:?}"
    );
    assert!(ran[0] + ran[1] <= 30, "one a frame: {ran:?}");
}

#[test]
fn budgets_past_reason_are_held_to_it() {
    let world = World::new();
    // A readback budget of everything is 64 MiB a frame; the queue 64
    // frames of that.
    world.set_readback_budget(u64::MAX);
    let buffer = world.add_buffer(BufferDesc::bytes(64 << 20)).unwrap();
    let held: Vec<Readback> = (0..100).map(|_| buffer.read(0..64 << 20)).collect();
    let queued = world.readbacks_queued().1;
    assert!(queued <= 64 * (64 << 20), "{queued} bytes queued");
    assert!(
        held.iter()
            .any(|r| r.try_take() == Some(Err(ReadError::Busy)))
    );
    // A budget that isn't a number is the default.
    world.set_compute_budget(ComputeBudget {
        gpu_ms: f32::NAN,
        sandboxed_gpu_ms: -1.,
        ..ComputeBudget::default()
    });
    let budget = world.compute_budget();
    assert_eq!(budget.gpu_ms, ComputeBudget::default().gpu_ms);
    assert_eq!(
        budget.sandboxed_gpu_ms,
        ComputeBudget::default().sandboxed_gpu_ms
    );
}

#[test]
fn a_sandboxed_pass_that_never_ends_ends() {
    use zgx_gpu::{
        viewport::{Viewport, ViewportFrame},
        wgpu,
    };
    let Some((device, queue)) = timed_device() else {
        return;
    };
    let world = World::new();
    // No time budget a slow or busy GPU could reach: the cap, not the
    // timing, is what's tested.
    world.set_compute_budget(ComputeBudget {
        sandboxed_gpu_ms: 1e9,
        owner_gpu_ms: 1e9,
        ..ComputeBudget::default()
    });
    let out = world.add_buffer(BufferDesc::storage::<u32>(256)).unwrap();
    let spin = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> out: array<u32>;
                 @compute @workgroup_size(64)
                 fn main(@builtin(global_invocation_id) id: vec3u) {
                     var n = 0u;
                     loop { n += 1u; }
                     out[id.x] = n;
                 }",
            )
            .buffer("out", &out, Access::Write)
            .threads([256, 1, 1])
            .sandboxed("plugin")
            .every_frame(),
        )
        .unwrap();
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
        // Waits for the device: a pass that didn't end would hang here.
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    };
    for _ in 0..200 {
        draw();
        if world.compute_status(spin) == Some(ComputeStatus::Ready) {
            break;
        }
    }
    assert_eq!(world.compute_status(spin), Some(ComputeStatus::Ready));
    // It has run: read what it wrote.
    draw();
    let read = out.read(0..1024);
    for _ in 0..20 {
        draw();
        if read.is_ready() {
            break;
        }
    }
    assert_eq!(world.compute_status(spin), Some(ComputeStatus::Ready));
    // Each invocation looped the same capped count: at least 64, and the
    // pass's work within the sandbox's.
    let looped = words(&read.try_take().unwrap().unwrap());
    let n = looped[0];
    assert!(looped.iter().all(|&m| m == n), "{:?}", &looped[..4]);
    assert!(
        n >= 64 && u64::from(n) * 256 <= 1 << 25,
        "{n} iterations each"
    );
}

#[test]
fn a_world_holds_at_most_1024_passes_and_a_sandboxed_shader_1_mib() {
    let world = World::new();
    let passes: Vec<ComputeId> = (0..1024)
        .map(|_| {
            world
                .add_compute(Compute::new("@compute @workgroup_size(1) fn main() {}"))
                .unwrap()
        })
        .collect();
    assert!(matches!(
        world.add_compute(Compute::new("@compute @workgroup_size(1) fn main() {}")),
        Err(ComputeError::Limits(_))
    ));
    world.remove_compute(passes[0]);
    assert!(
        world
            .add_compute(Compute::new("@compute @workgroup_size(1) fn main() {}"))
            .is_ok()
    );
    // A sandboxed shader past 1 MiB is refused before it's parsed.
    let world = World::new();
    let long = format!(
        "{}@compute @workgroup_size(1) fn main() {{}}",
        "//\n".repeat(400_000)
    );
    assert!(matches!(
        world.add_compute(Compute::new(long.as_str()).sandboxed("plugin")),
        Err(ComputeError::Limits(_))
    ));
}

#[test]
fn a_rated_pass_lets_its_world_rest_between_its_runs() {
    let world = World::new();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let count = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
    let pass = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                 @compute @workgroup_size(1) fn main() { count[0] += 1u; }",
            )
            .buffer("count", &count, Access::ReadWrite)
            .at_rate(1.),
        )
        .unwrap();
    assert!(world.view().animating(), "due at once");
    window.compiled(&world, pass);
    window.draw();
    // Run once; its next run a second away: the world draws nothing
    // between.
    assert!(!world.view().animating(), "at rest until it's due");
    let before = world.frames_drawn();
    for _ in 0..30 {
        window.draw();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(world.frames_drawn(), before, "no frames between runs");
    // A pass at 120 a second is due every frame: it keeps drawing.
    let fast = world
        .add_compute(Compute::new("@compute @workgroup_size(1) fn main() {}").at_rate(120.))
        .unwrap();
    window.compiled(&world, fast);
    world.view().invalidate();
    for _ in 0..5 {
        window.draw();
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
    assert!(world.view().animating());
}

#[test]
fn sandboxed_passes_are_counted_by_world_and_owner() {
    let world = World::new();
    let pass = || Compute::new("@compute @workgroup_size(1) fn main() {}");
    for _ in 0..16 {
        world.add_compute(pass().sandboxed("one")).unwrap();
    }
    assert!(matches!(
        world.add_compute(pass().sandboxed("one")),
        Err(ComputeError::Limits(_))
    ));
    for owner in ["two", "three", "four"] {
        for _ in 0..16 {
            world.add_compute(pass().sandboxed(owner)).unwrap();
        }
    }
    assert!(
        matches!(
            world.add_compute(pass().sandboxed("five")),
            Err(ComputeError::Limits(_))
        ),
        "64 a world"
    );
    assert!(
        world.add_compute(pass()).is_ok(),
        "the app's own aren't counted"
    );
    // More invocations than a sandboxed pass runs.
    let world = World::new();
    assert!(matches!(
        world.add_compute(pass().threads([1 << 17, 1, 1]).sandboxed("one")),
        Err(ComputeError::Limits(_))
    ));
}

#[test]
fn sandboxed_passes_share_a_frames_work_and_an_owners() {
    use zgx_gpu::{
        viewport::{Viewport, ViewportFrame},
        wgpu,
    };
    let Some((device, queue)) = timed_device() else {
        return;
    };
    let world = World::new();
    world.set_compute_budget(ComputeBudget {
        // No time budget that a busy GPU could reach: only the frame's
        // and the owners' work decide what runs.
        sandboxed_gpu_ms: 1e9,
        owner_gpu_ms: 1e9,
        ..ComputeBudget::default()
    });
    let counts = world.add_buffer(BufferDesc::storage::<u32>(8)).unwrap();
    // Eight looping passes, four an owner: each a whole run's work, so an
    // owner's frame holds two and the frame four.
    let passes: Vec<ComputeId> = (0..8)
        .map(|i| {
            world
                .add_compute(
                    Compute::new(format!(
                        "@group(0) @binding(0) var<storage, read_write> counts: array<atomic<u32>>;
                         @compute @workgroup_size(64)
                         fn main(@builtin(local_invocation_index) i: u32) {{
                             var n = 0u;
                             loop {{ n += 1u; }}
                             if i == 0u {{ atomicAdd(&counts[{i}], 1u + (n & 0u)); }}
                         }}"
                    ))
                    .buffer("counts", &counts, Access::ReadWrite)
                    .threads([1024, 1, 1])
                    .every_frame()
                    .sandboxed(if i < 4 { "one" } else { "two" }),
                )
                .unwrap()
        })
        .collect();
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
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    };
    let ready = || {
        passes
            .iter()
            .all(|&p| world.compute_status(p) == Some(ComputeStatus::Ready))
    };
    for _ in 0..2000 {
        draw();
        if ready() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Every pass compiled (on a busy machine, a compile's deadline runs from
    // when the worker starts it, not from when it was queued behind others).
    let statuses: Vec<_> = passes.iter().map(|&p| world.compute_status(p)).collect();
    assert!(ready(), "{statuses:?}");
    // From none, 40 frames, and the one the read is copied after.
    counts.write(0, &[0u32; 8]).unwrap();
    for _ in 0..40 {
        draw();
    }
    let read = counts.read(0..32);
    draw();
    for _ in 0..20 {
        if read.is_ready() {
            break;
        }
        draw();
    }
    // A run counts once a workgroup: 16 of them.
    let ran: Vec<u32> = words(&read.try_take().unwrap().unwrap())
        .iter()
        .map(|n| n / 16)
        .collect();
    let (one, two): (u32, u32) = (ran[..4].iter().sum(), ran[4..].iter().sum());
    // Two an owner a frame, four a frame in all; every pass had turns.
    assert!(one <= 2 * 41 && two <= 2 * 41, "{ran:?}");
    assert!(one + two >= 4 * 30, "the budget used: {ran:?}");
    assert!(ran.iter().all(|&n| n >= 10), "each had turns: {ran:?}");
}

#[test]
fn an_owners_passes_together_over_its_budget_are_all_turned_off() {
    // Each within its own budget, the owner's two together not: judged on
    // these times.
    let world = World::new();
    world.set_compute_budget(ComputeBudget {
        sandboxed_gpu_ms: 100.,
        owner_gpu_ms: 1.0,
        frames: 3,
        ..ComputeBudget::default()
    });
    let pass = |owner: &str| {
        world
            .add_compute(
                Compute::new("@compute @workgroup_size(1) fn main() {}")
                    .every_frame()
                    .sandboxed(owner),
            )
            .unwrap()
    };
    let (a, b, other) = (pass("greedy"), pass("greedy"), pass("modest"));
    for _ in 0..3 {
        world.time_compute_frame(&[(a, 0.6), (b, 0.6), (other, 0.6)]);
    }
    for id in [a, b] {
        assert!(matches!(
            world.compute_status(id),
            Some(ComputeStatus::OverBudget { .. })
        ));
    }
    assert!(!matches!(
        world.compute_status(other),
        Some(ComputeStatus::OverBudget { .. })
    ));
}

/// The same on a GPU's own timestamps (see the test above for running it).
#[test]
#[ignore = "real GPU timestamps: run on a quiet GPU with --ignored"]
fn an_owners_passes_over_its_budget_together_are_all_turned_off() {
    use zgx_gpu::{
        viewport::{Viewport, ViewportFrame},
        wgpu,
    };
    let Some((device, queue)) = timed_device() else {
        return;
    };
    let world = World::new();
    // Each pass alone within its own budget; the owner's two together not.
    world.set_compute_budget(ComputeBudget {
        sandboxed_gpu_ms: 100.,
        owner_gpu_ms: 0.01,
        ..ComputeBudget::default()
    });
    let sink = world.add_buffer(BufferDesc::storage::<u32>(1024)).unwrap();
    let spin = |owner: &str| {
        world
            .add_compute(
                Compute::new(
                    "@group(0) @binding(0) var<storage, read_write> sink: array<u32>;
                     @compute @workgroup_size(64)
                     fn main(@builtin(global_invocation_id) id: vec3u) {
                         var n = 0u;
                         loop { n += 1u; }
                         sink[id.x] = n;
                     }",
                )
                .buffer("sink", &sink, Access::ReadWrite)
                .threads([1024, 1, 1])
                .every_frame()
                .sandboxed(owner),
            )
            .unwrap()
    };
    let (a, b) = (spin("greedy"), spin("greedy"));
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
    for _ in 0..300 {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut frame =
            ViewportFrame::new(&device, &queue, &mut encoder, &view, format, (8, 8), 1.);
        painter.render(&mut frame);
        queue.submit([encoder.finish()]);
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if matches!(
            world.compute_status(a),
            Some(ComputeStatus::OverBudget { .. })
        ) {
            break;
        }
    }
    for pass in [a, b] {
        assert!(
            matches!(
                world.compute_status(pass),
                Some(ComputeStatus::OverBudget { .. })
            ),
            "{:?}",
            world.compute_status(pass)
        );
    }
}

type Painters<'a, 'b> = &'a mut [&'b mut dyn zgx_gpu::viewport::Viewport];

/// A buffer holding 1000 and a pass adding 1 to it every frame, made
/// ready by frames at time 0 (each of `painters` drawing each `boxes`
/// times).
fn counting(
    world: &World,
    frames: &mut TimedFrames,
    painters: Painters,
    boxes: usize,
) -> GpuBuffer {
    let count = world
        .add_buffer(BufferDesc::from_slice(&[1000u32]))
        .unwrap();
    let pass = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                 @compute @workgroup_size(1) fn main() { count[0] += 1u; }",
            )
            .buffer("count", &count, Access::ReadWrite)
            .every_frame(),
        )
        .unwrap();
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(10) {
        for _ in 0..boxes {
            frames.draw(painters, 0);
        }
        if world.compute_status(pass) == Some(ComputeStatus::Ready) {
            return count;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("not compiled");
}

/// `count` after frames 16 ms apart to 320 ms (each of `painters` drawing
/// each `boxes` times), and then until it reads back.
fn count_after(
    count: &GpuBuffer,
    frames: &mut TimedFrames,
    painters: Painters,
    boxes: usize,
) -> u32 {
    let frame = |ms: u64, frames: &mut TimedFrames, painters: Painters| {
        for _ in 0..boxes {
            frames.draw(painters, ms);
        }
    };
    let mut ms = 0;
    while ms < 320 {
        ms += 16;
        frame(ms, frames, painters);
    }
    // Copied after the next frame's passes: what later frames run isn't
    // in it.
    let read = count.read(0..4);
    for _ in 0..100 {
        ms += 16;
        frame(ms, frames, painters);
        if read.is_ready() {
            return words(&read.try_take().unwrap().unwrap())[0];
        }
    }
    panic!("not read");
}

/// What a world's every-frame pass counts drawn once a frame by one painter
/// (`count_after`): the control.
fn counted_once(frames: &mut TimedFrames) -> u32 {
    let world = World::new();
    let mut painter = world.viewport();
    let count = counting(&world, frames, &mut [&mut painter], 1);
    count_after(&count, frames, &mut [&mut painter], 1)
}

#[test]
fn a_world_shown_in_two_boxes_runs_its_passes_once_a_frame() {
    let Some(device) = TimedFrames::device() else {
        return;
    };
    let mut frames = TimedFrames::new(device);
    let once = counted_once(&mut frames);
    assert!(once > 1000 + 15, "{once}: it ran");
    let world = World::new();
    // One painter drawing each frame twice, as a view in two boxes does.
    let mut painter = world.viewport();
    let count = counting(&world, &mut frames, &mut [&mut painter], 2);
    let twice = count_after(&count, &mut frames, &mut [&mut painter], 2);
    assert_eq!(twice, once, "shown twice a frame, as shown once");
}

#[test]
fn two_viewports_of_one_world_share_its_gpu_data_and_step_it_once_a_frame() {
    let Some(device) = TimedFrames::device() else {
        return;
    };
    let mut frames = TimedFrames::new(device);
    let once = counted_once(&mut frames);
    let world = World::new();
    // Its own view's painter and a second (a thumbnail beside it), on
    // one device, taking turns each frame: its buffer not made again by
    // each in turn (from none), nor its pass run by each.
    let (mut first, mut second) = (world.viewport(), world.viewport());
    let count = counting(&world, &mut frames, &mut [&mut first, &mut second], 1);
    let shared = count_after(&count, &mut frames, &mut [&mut first, &mut second], 1);
    assert_eq!(shared, once, "two painters, as one");
}

/// Through a real renderer, which stamps each window frame's time itself:
/// a world's view in two boxes runs its every-frame pass once a frame.
#[test]
fn a_world_shown_in_two_boxes_of_a_window_runs_its_passes_once_a_window_frame() {
    let world = World::new();
    let Some(mut window) = Window::with_boxes(&world, 2) else {
        return;
    };
    let count = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
    let pass = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                 @compute @workgroup_size(1) fn main() { count[0] += 1u; }",
            )
            .buffer("count", &count, Access::ReadWrite)
            .every_frame(),
        )
        .unwrap();
    window.compiled(&world, pass);
    count.write(0, &[0u32]).unwrap();
    let rendered = world.frames_drawn();
    for _ in 0..20 {
        window.draw();
    }
    // Each window frame rendered the view twice.
    assert_eq!(world.frames_drawn() - rendered, 40, "two boxes");
    let read = count.read(0..4);
    let more = window.until(&read, 100) as u32;
    let runs = words(&read.try_take().unwrap().unwrap())[0];
    // A run a window frame (the read copied within the frames after).
    assert!(
        (20..=20 + more).contains(&runs),
        "{runs} runs in {} window frames",
        20 + more
    );
}
