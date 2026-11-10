//! Which uploads open the Metal driver's 8 MB blocks: a world of `--meshes`
//! cubes (`--vertices` each, by a sphere's segments), `--textures` distinct
//! 1024² textures over them (decoded from PNGs, as a model's are, unless
//! `--held=1`), an environment if `--environment=1`, drawn `--frames` times
//! at 1920x1080 on our own device; then it prints its pid and waits
//! `--pause` seconds for `footprint -v` from outside.
use std::time::Duration;
use zgx_3d::*;
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

fn arg(name: &str, default: usize) -> usize {
    std::env::args()
        .find_map(|a| {
            a.strip_prefix(&format!("--{name}="))
                .map(|v| v.parse().unwrap())
        })
        .unwrap_or(default)
}

fn png(seed: usize) -> Vec<u8> {
    let (w, h) = (1024u32, 1024u32);
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = ((x ^ y) as usize).wrapping_mul(seed + 7) as u8;
            pixels.extend([v, v.wrapping_add(seed as u8), 255 - v, 255]);
        }
    }
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    bytes
}

/// The same first frames on Metal alone, no wgpu: a clear pass on a
/// 1920x1080 target, and with `blit` a buffer filled by a blit encoder.
#[cfg(target_os = "macos")]
fn raw_metal(frames: usize, blit: bool, pause: usize) {
    use objc2_metal::*;
    let device = MTLCreateSystemDefaultDevice().unwrap();
    let queue = device.newCommandQueue().unwrap();
    let descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::BGRA8Unorm,
            1920,
            1080,
            false,
        )
    };
    descriptor.setUsage(MTLTextureUsage::RenderTarget);
    descriptor.setStorageMode(MTLStorageMode::Private);
    let texture = device.newTextureWithDescriptor(&descriptor).unwrap();
    let buffer = device
        .newBufferWithLength_options(1 << 16, MTLResourceOptions::StorageModePrivate)
        .unwrap();
    for _ in 0..frames {
        let commands = queue.commandBuffer().unwrap();
        if blit {
            let encoder = commands.blitCommandEncoder().unwrap();
            encoder.fillBuffer_range_value(&buffer, objc2_foundation::NSRange::new(0, 1 << 16), 0);
            encoder.endEncoding();
        }
        let pass = MTLRenderPassDescriptor::new();
        let colour = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        colour.setTexture(Some(&texture));
        colour.setLoadAction(MTLLoadAction::Clear);
        colour.setStoreAction(MTLStoreAction::Store);
        let encoder = commands.renderCommandEncoderWithDescriptor(&pass).unwrap();
        encoder.endEncoding();
        commands.commit();
        commands.waitUntilCompleted();
    }
    eprintln!("paused, pid {}", std::process::id());
    std::thread::sleep(Duration::from_secs(pause as u64));
}

fn main() {
    #[cfg(target_os = "macos")]
    if arg("raw", 0) == 1 {
        return raw_metal(arg("frames", 3), arg("blit", 0) == 1, arg("pause", 15));
    }
    let (meshes, segments, textures) =
        (arg("meshes", 100), arg("segments", 16), arg("textures", 0));
    let (frames, pause) = (arg("frames", 3), arg("pause", 15));
    let world = World::new();
    world.set_background(Some(0x303030));
    world.add_light(Light::Directional {
        direction: vec3(-0.3, -1., -0.2),
        color: [1.; 3],
        intensity: 3.,
    });
    if arg("shadows", 0) == 1 {
        world.set_shadows(true);
    }
    if arg("environment", 0) == 1 {
        world.set_environment(Some(Environment::studio()));
    }
    let textures: Vec<Texture> = (0..textures)
        .map(|i| {
            if arg("held", 0) == 1 {
                let bytes = png(i);
                let _ = bytes;
                Texture::from_rgba8(1024, 1024, vec![(i * 13) as u8; 1024 * 1024 * 4]).unwrap()
            } else {
                Texture::decode(&png(i)).unwrap()
            }
        })
        .collect();
    let side = (meshes as f32).sqrt().ceil() as usize;
    for i in 0..meshes {
        let id = world.add(
            Object::mesh(
                Geometry::sphere(0.4, segments as u32),
                Material::rgb(0xffffff),
            )
            .at([
                (i % side) as f32 - side as f32 / 2.,
                0.,
                -((i / side) as f32) - 3.,
            ]),
        );
        if !textures.is_empty() {
            world.set_texture(id, Some(textures[i % textures.len()].clone()));
        }
        // `--variants=N`: meshes spread over N pipelines, by which of five
        // features each has (bits of its index).
        let variants = arg("variants", 0);
        if variants > 0 {
            let v = i % variants;
            let small = Texture::from_rgba8(2, 2, vec![200; 16]).unwrap();
            if v & 1 != 0 {
                world.set_alpha_cutoff(id, Some(0.5));
            }
            if v & 2 != 0 {
                world.set_double_sided(id, true);
            }
            if v & 4 != 0 {
                world.set_map(id, Map::Normal, Some(small.clone()));
            }
            if v & 8 != 0 {
                world.set_map(id, Map::MetalRoughness, Some(small.clone()));
            }
            if v & 16 != 0 {
                world.set_map(id, Map::Emission, Some(small));
            }
        }
    }
    world.set_camera(Camera::looking_at(
        [0., 4., 6.],
        [0., 0., -(side as f32) / 2.],
    ));
    drop(textures);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1920,
            height: 1080,
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
    // Controls without zgx: `--world=0` clears the target in a render pass
    // and nothing else; `--write-buffer=1` also writes 64 bytes to a buffer
    // through the queue; `--clear-buffer=1` clears one in an encoder.
    if arg("world", 1) == 0 {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1 << 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        for _ in 0..frames {
            let mut encoder = device.create_command_encoder(&Default::default());
            if arg("clear-buffer", 0) == 1 {
                encoder.clear_buffer(&buffer, 0, None);
            }
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if arg("write-buffer", 0) == 1 {
                queue.write_buffer(&buffer, 0, &[1u8; 64]);
            }
            queue.submit([encoder.finish()]);
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        }
        eprintln!("paused, pid {}", std::process::id());
        std::thread::sleep(Duration::from_secs(pause as u64));
        return;
    }
    let mut painter = world.viewport();
    for frame in 0..frames {
        world.set_camera(Camera::looking_at(
            [frame as f32 * 0.01, 4., 6.],
            [0., 0., -3.],
        ));
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut drawing = ViewportFrame::new(
            &device,
            &queue,
            &mut encoder,
            &view,
            FORMAT,
            (1920, 1080),
            1.,
        );
        drawing.target_srgb = Some(&srgb);
        painter.render(&mut drawing);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
    eprintln!("paused, pid {}", std::process::id());
    std::thread::sleep(Duration::from_secs(pause as u64));
}
