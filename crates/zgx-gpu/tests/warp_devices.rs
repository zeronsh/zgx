//! wgpu devices of their own on WARP, on threads of one process, drawing and
//! submitting at once: each draws its own colour. WARP 10.0.26100 faulted
//! in d3d10warp.dll, or drew nothing, when their queues executed command
//! lists together; the vendored wgpu-hal takes one lock around WARP's
//! submissions (vendor/wgpu-hal/ZGX_PATCH.md).
#![cfg(windows)]

const THREADS: u8 = 16;
const FRAMES: u32 = 12;
const SIZE: u32 = 64;

/// A device on WARP of an instance of its own, if this machine has it.
fn warp() -> Option<(wgpu::Device, wgpu::Queue)> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::DX12;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }))
    .ok()
    .filter(|adapter| adapter.get_info().device_type == wgpu::DeviceType::Cpu)?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// Draws a triangle over the whole target in `color` `FRAMES` times, each
/// frame its own submission, and reads each frame back.
fn draw(device: &wgpu::Device, queue: &wgpu::Queue, color: [u8; 4]) {
    let [r, g, b, a] = color.map(|c| f32::from(c) / 255.);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
                    let p = vec2<f32>(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0);
                    return vec4<f32>(p, 0.0, 1.0);
                }}
                @fragment fn fs() -> @location(0) vec4<f32> {{
                    return vec4<f32>({r}, {g}, {b}, {a});
                }}"
            )
            .into(),
        ),
    });
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(format.into())],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(SIZE * SIZE * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    for frame in 0..FRAMES {
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
            },
            target.size(),
        );
        queue.submit([encoder.finish()]);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        {
            let pixels = readback.slice(..).get_mapped_range();
            let centre = 4 * (SIZE * SIZE / 2 + SIZE / 2) as usize;
            assert_eq!(pixels[centre..centre + 4], color, "frame {frame}");
        }
        readback.unmap();
    }
}

#[test]
fn devices_of_their_own_on_warp_draw_at_once_on_threads() {
    if warp().is_none() {
        eprintln!("no WARP here");
        return;
    }
    let threads: Vec<_> = (0..THREADS)
        .map(|thread| {
            std::thread::spawn(move || {
                let (device, queue) = warp().unwrap();
                let bits = thread % 7 + 1;
                let channel = |bit: u8| if bits & bit != 0 { 255 } else { 0 };
                draw(&device, &queue, [channel(1), channel(2), channel(4), 255]);
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
