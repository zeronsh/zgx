//! What loading a textured model costs in memory, from file bytes to the
//! first frame and after the process has rested: a generated glTF (no
//! downloaded assets) of `--textures` quads, each with its own 1024x1024
//! PNG, read by `Model` and drawn once at 512x512 on our own device.
//!
//! Reports, as one `RESULT {json}` line: the heap's peak while loading and
//! what it holds after (a counting allocator), the footprint after reading,
//! after the first frame and after `--rest` seconds idle (macOS gives freed
//! large blocks back to the system lazily, so the footprint right after a
//! load still holds what was freed), Metal's allocated bytes, and the time
//! from the file's bytes to the first frame done. `--pause=S` waits S
//! seconds after the rest, its pid printed, for `footprint -v` from outside.
//!
//! cargo run --release --bin load_memory -- --textures=64
use std::{
    alloc::{GlobalAlloc, Layout, System},
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use zgx_3d::*;
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

#[path = "../../../common/rusage.rs"]
mod rusage;

/// The system allocator (or mimalloc, as zgx-run ships, with
/// `--features mimalloc`), counting live bytes and their peak.
struct Counting;
#[cfg(feature = "mimalloc")]
static INNER: mimalloc::MiMalloc = mimalloc::MiMalloc;
#[cfg(not(feature = "mimalloc"))]
static INNER: System = System;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn grew(by: usize) {
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grew(layout.size());
        unsafe { INNER.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { INNER.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if size > layout.size() {
            grew(size - layout.size());
        } else {
            LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
        }
        unsafe { INNER.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn arg(name: &str, default: usize) -> usize {
    std::env::args()
        .find_map(|a| {
            a.strip_prefix(&format!("--{name}="))
                .map(|v| v.parse().unwrap())
        })
        .unwrap_or(default)
}
fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1048576.
}

/// A 1024² PNG of its own: a pattern that compresses as photos roughly do.
fn png(seed: usize) -> Vec<u8> {
    let side = 1024u32;
    let mut pixels = Vec::with_capacity((side * side * 4) as usize);
    let mut state = seed as u32 * 2654435761 + 1;
    for y in 0..side {
        for x in 0..side {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let grain = (state & 31) as u8;
            let v = ((x / 8 + y / 8) as u8).wrapping_mul(seed as u8 | 1);
            pixels.extend([v.wrapping_add(grain), v ^ 0x5a, 255 - v, 255]);
        }
    }
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, side, side);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    bytes
}

/// Writes the glTF and its PNGs into `dir` (once per texture count).
fn generate(dir: &PathBuf, textures: usize) -> PathBuf {
    let gltf = dir.join("model.gltf");
    if gltf.exists() {
        return gltf;
    }
    std::fs::create_dir_all(dir).unwrap();
    // One quad, shared: positions, normals, coordinates, indices.
    let mut bin = Vec::new();
    for p in [[0f32, 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]] {
        p.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    }
    for _ in 0..4 {
        [0f32, 0., 1.]
            .iter()
            .for_each(|v| bin.extend(v.to_le_bytes()));
    }
    for t in [[0f32, 1.], [1., 1.], [1., 0.], [0., 0.]] {
        t.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    }
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend(i.to_le_bytes());
    }
    std::fs::write(dir.join("quad.bin"), &bin).unwrap();
    let side = (textures as f32).sqrt().ceil() as usize;
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    let mut images = Vec::new();
    let mut texture_list = Vec::new();
    for i in 0..textures {
        std::fs::write(dir.join(format!("t{i}.png")), png(i)).unwrap();
        images.push(format!(r#"{{"uri":"t{i}.png"}}"#));
        texture_list.push(format!(r#"{{"source":{i}}}"#));
        materials.push(format!(
            r#"{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":{i}}},"metallicFactor":0}}}}"#
        ));
        meshes.push(format!(
            r#"{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2}},"indices":3,"material":{i}}}]}}"#
        ));
        let (x, y) = ((i % side) as f32 * 1.1, (i / side) as f32 * 1.1);
        nodes.push(format!(r#"{{"mesh":{i},"translation":[{x},{y},0]}}"#));
    }
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[{roots}]}}],
"nodes":[{nodes}],"meshes":[{meshes}],"materials":[{materials}],"textures":[{textures}],"images":[{images}],
"buffers":[{{"uri":"quad.bin","byteLength":{len}}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":48}},{{"buffer":0,"byteOffset":48,"byteLength":48}},{{"buffer":0,"byteOffset":96,"byteLength":32}},{{"buffer":0,"byteOffset":128,"byteLength":12}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},{{"bufferView":1,"componentType":5126,"count":4,"type":"VEC3"}},{{"bufferView":2,"componentType":5126,"count":4,"type":"VEC2"}},{{"bufferView":3,"componentType":5123,"count":6,"type":"SCALAR"}}]}}"#,
        roots = (0..textures)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(","),
        nodes = nodes.join(","),
        meshes = meshes.join(","),
        materials = materials.join(","),
        textures = texture_list.join(","),
        images = images.join(","),
        len = bin.len(),
    );
    std::fs::write(&gltf, json).unwrap();
    gltf
}

fn allocated(device: &wgpu::Device) -> u64 {
    #[cfg(target_os = "macos")]
    {
        use objc2_metal::MTLDevice;
        // SAFETY: only queried.
        if let Some(metal) = unsafe { device.as_hal::<wgpu::hal::api::Metal>() } {
            return metal.raw_device().currentAllocatedSize() as u64;
        }
    }
    let _ = device;
    0
}

fn main() {
    let textures = arg("textures", 64);
    // `--control=1`: no GPU, no model: 64 heap blocks of 4 MB filled and
    // freed, to see what the allocator itself keeps of freed large blocks.
    if arg("control", 0) == 1 {
        let start = rusage::sample();
        let blocks: Vec<Vec<u8>> = (0..textures).map(|i| vec![i as u8 | 1; 4 << 20]).collect();
        let held = rusage::sample();
        drop(blocks);
        let freed = rusage::sample();
        std::thread::sleep(Duration::from_secs(arg("rest", 10) as u64));
        let rested = rusage::sample();
        println!(
            "RESULT {{\"control\": true, \"footprint_start_mb\": {:.1}, \"held_mb\": {:.1}, \"freed_mb\": {:.1}, \"rested_mb\": {:.1}}}",
            mb(start.footprint),
            mb(held.footprint),
            mb(freed.footprint),
            mb(rested.footprint)
        );
        if let Some(pause) =
            std::env::args().find_map(|a| a.strip_prefix("--pause=").map(str::to_owned))
        {
            eprintln!("paused, pid {}", std::process::id());
            std::thread::sleep(Duration::from_secs(pause.parse().unwrap()));
        }
        return;
    }
    let dir = std::env::temp_dir().join(format!("zgx-load-memory-{textures}"));
    let path = generate(&dir, textures);
    let file_bytes: u64 = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
    let side = 512;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: side,
            height: side,
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

    // From the file's bytes to the first frame.
    let before = rusage::sample();
    let heap_before = LIVE.load(Ordering::Relaxed);
    PEAK.store(heap_before, Ordering::Relaxed);
    let started = Instant::now();
    let folder = dir.clone();
    let model = Model::from_bytes_with(&std::fs::read(&path).unwrap(), |uri| {
        std::fs::read(folder.join(uri)).ok()
    })
    .expect("the model");
    let read_ms = started.elapsed().as_secs_f64() * 1e3;
    let after_read = rusage::sample();
    let heap_after_read = LIVE.load(Ordering::Relaxed);
    let world = World::new();
    world.add_model(&model);
    drop(model);
    let span = (textures as f32).sqrt().ceil() * 1.1;
    world.set_camera(Camera::looking_at(
        [span / 2., span / 2., span * 1.3],
        [span / 2., span / 2., 0.],
    ));
    let mut painter = world.viewport();
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut drawing = ViewportFrame::new(
        &device,
        &queue,
        &mut encoder,
        &view,
        FORMAT,
        (side, side),
        1.,
    );
    drawing.target_srgb = Some(&srgb);
    painter.render(&mut drawing);
    queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let load_ms = started.elapsed().as_secs_f64() * 1e3;
    let first = rusage::sample();
    let heap_first = LIVE.load(Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed);
    let metal = allocated(&device);

    let rest = arg("rest", 10);
    std::thread::sleep(Duration::from_secs(rest as u64));
    let rested = rusage::sample();
    let heap_rested = LIVE.load(Ordering::Relaxed);
    println!(
        "RESULT {{\"textures\": {textures}, \"file_mb\": {:.1}, \"read_ms\": {read_ms:.1}, \"load_ms\": {load_ms:.1}, \
\"heap_peak_mb\": {:.1}, \"heap_after_read_mb\": {:.1}, \"heap_first_frame_mb\": {:.1}, \"heap_rested_mb\": {:.1}, \
\"footprint_start_mb\": {:.1}, \"footprint_after_read_mb\": {:.1}, \"footprint_first_frame_mb\": {:.1}, \
\"footprint_rested_mb\": {:.1}, \"peak_footprint_mb\": {:.1}, \"metal_allocated_mb\": {:.1}, \"rest_s\": {rest}}}",
        mb(file_bytes),
        mb((peak - heap_before) as u64),
        mb((heap_after_read - heap_before) as u64),
        mb((heap_first - heap_before) as u64),
        mb((heap_rested - heap_before) as u64),
        mb(before.footprint),
        mb(after_read.footprint),
        mb(first.footprint),
        mb(rested.footprint),
        mb(rested.peak_footprint),
        mb(metal),
    );
    if let Some(pause) =
        std::env::args().find_map(|a| a.strip_prefix("--pause=").map(str::to_owned))
    {
        eprintln!("paused, pid {}", std::process::id());
        std::thread::sleep(Duration::from_secs(pause.parse().unwrap()));
    }
    drop(painter);
}
