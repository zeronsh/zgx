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
        let gpu = match GpuRenderer::new(32, 32) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let mut ui = Ui::new(32., 32.);
        ui.mount(
            div()
                .size(32., 32.)
                .bg(Color(0, 0, 0, 255))
                .child(gpu_view("World", world.view()).size(32., 32.)),
        );
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
    fn compiled(&mut self, world: &World, id: ComputeId) {
        for _ in 0..500 {
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
    // A small read: copied this frame, mapped and read the next.
    let read = small.read(0..64);
    let frames = window.until(&read, 100);
    assert!(frames <= 3, "{frames} frames");
    assert_eq!(read.try_take().unwrap().unwrap(), vec![0; 64]);
    // 256 KiB at 64 KiB a frame: four frames copying, then mapped.
    let read = large.read(0..256 << 10);
    let frames = window.until(&read, 100);
    assert!((5..=8).contains(&frames), "{frames} frames");
    assert_eq!(read.try_take().unwrap().unwrap().len(), 256 << 10);
    // A budget of the whole: one frame copying.
    world.set_readback_budget(256 << 10);
    let read = large.read(0..256 << 10);
    let frames = window.until(&read, 100);
    assert!(frames <= 3, "{frames} frames");
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
            .at_rate(20.),
        )
        .unwrap();
    window.compiled(&world, pass);
    world.set_compute_enabled(pass, false);
    world.set_compute_enabled(pass, true);
    // Frames 4 ms apart for half a second: 20 a second runs about 10 times,
    // not once a frame.
    let start = std::time::Instant::now();
    let mut frames = 0;
    while start.elapsed().as_secs_f32() < 0.5 {
        window.draw();
        frames += 1;
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    world.set_compute_enabled(pass, false);
    let read = count.read(0..4);
    window.until(&read, 100);
    let runs = words(&read.try_take().unwrap().unwrap())[0];
    assert!((8..=13).contains(&runs), "{runs} runs in {frames} frames");
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
