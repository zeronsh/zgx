//! `SCENE=compute` (with the `compute` feature): `textured`'s world with the
//! app's GPU data beside it, so a frame counts what compute costs the CPU:
//! a 1 MiB buffer and a 256² texture updated every frame, a pass at 30 a
//! second, an on-demand one asked for every fifth frame, and a 4 KiB read
//! each frame (`NO_READS=1`: none). The passes are compiled before the
//! frames counted, as an app's would be by the time it's running.
use super::*;

const UPDATE: &str = "
struct Frame { time: f32, delta: f32, index: u32 }
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read_write> points: array<vec4f>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    points[id.x] += vec4f(frame.delta, 0., 0., 0.);
}
";
const FILL: &str = "
struct Frame { time: f32, delta: f32, index: u32 }
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> points: array<vec4f>;
@group(0) @binding(2) var out: texture_storage_2d<rgba16float, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    textureStore(out, id.xy, points[id.x + id.y * 256u] * frame.time);
}
";
const CLEAR: &str = "
@group(0) @binding(0) var<storage, read_write> points: array<vec4f>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) { points[id.x] = vec4f(0.); }
";

pub(super) fn main() {
    let (meshes, frames) = (setting("MESHES", 2000), setting("FRAMES", 30));
    let (device, queue) = device();
    let (world, root, joint, morphs) = world(false, meshes);
    let points = world
        .add_buffer(BufferDesc::storage::<[f32; 4]>(65536))
        .expect("1 MiB");
    let height = world
        .add_texture(TextureDesc::storage(256, 256, Format::Rgba16Float))
        .expect("512 KiB");
    let passes = [
        world.add_compute(
            Compute::new(UPDATE)
                .buffer("points", &points, Access::ReadWrite)
                .threads([65536, 1, 1])
                .every_frame(),
        ),
        world.add_compute(
            Compute::new(FILL)
                .buffer("points", &points, Access::Read)
                .texture("out", &height, Access::Write)
                .threads([256, 256, 1])
                .at_rate(30.),
        ),
        world.add_compute(
            Compute::new(CLEAR)
                .buffer("points", &points, Access::ReadWrite)
                .threads([65536, 1, 1])
                .on_demand(),
        ),
    ]
    .map(|pass| pass.expect("a valid pass"));
    let mut painter = world.viewport();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("A frame"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[FORMAT.add_srgb_suffix()],
    });
    let view = target.create_view(&Default::default());
    let srgb = target.create_view(&wgpu::TextureViewDescriptor {
        format: Some(FORMAT.add_srgb_suffix()),
        ..Default::default()
    });
    // Frame times a 60 Hz display's, so the rated pass runs as it would.
    let start = std::time::Instant::now();
    let mut draw = |index: usize| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut drawing = ViewportFrame::new(
            &device,
            &queue,
            &mut encoder,
            &view,
            FORMAT,
            (SIZE, SIZE),
            1.,
        );
        drawing.target_srgb = Some(&srgb);
        drawing.time = start + std::time::Duration::from_micros(index as u64 * 16_667);
        painter.render(&mut drawing);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::Poll).expect("the no-op device");
    };
    // Compiled on their threads, then picked up.
    draw(0);
    while passes
        .iter()
        .any(|&pass| world.compute_status(pass) != Some(ComputeStatus::Ready))
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
        draw(0);
    }
    let mut reads = Vec::new();
    for frame in 0..frames {
        moved(&world, (root, joint, &morphs), frame);
        if frame % 5 == 0 {
            world.run_compute(passes[2]);
        }
        if std::env::var_os("NO_READS").is_none() {
            reads.push(points.read(0..4096));
        }
        draw(frame + 1);
        reads.retain(|read| !read.is_ready());
    }
    assert!(world.frames_drawn() > frames as u64);
}
