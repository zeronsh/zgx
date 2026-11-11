//! The smallest use of wgpu 29.0.4's ray queries through zgx's vendored
//! wgpu-hal, step by step, for finding where a driver falls over: a cube's
//! BLAS, a TLAS of two instances of it, a ray query from a compute shader
//! and from a fragment shader, each image read back and checked. A
//! `RTPROBE` line on stderr goes before and after each step (stderr isn't
//! buffered), so a crash's last line names the step it died in.
//!
//! Run, from this folder: `cargo run --release`. `RTPROBE_BACKEND=vulkan`
//! (or `dx12`, `metal`) picks a backend; `RTPROBE_ADAPTER=llvmpipe` the
//! adapter whose name holds it.

use wgpu::util::DeviceExt;

const SHADER: &str = include_str!("trace.wgsl");
const N: u32 = 64;

fn mark(step: &str) {
    eprintln!("RTPROBE {step}");
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    size: [u32; 2],
    _pad: [u32; 2],
    eye: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
}

/// A camera at (1, 0, 3) looking down -z with a 60° field, `N` by `N`.
fn camera() -> Params {
    let h = 30f32.to_radians().tan();
    Params {
        size: [N, N],
        _pad: [0; 2],
        eye: [1., 0., 3., 0.],
        forward: [0., 0., -1., 0.],
        right: [h, 0., 0., 0.],
        up: [0., h, 0., 0.],
    }
}

/// A unit cube about the origin: 8 vertices, 12 triangles.
fn cube() -> (Vec<[f32; 3]>, Vec<u32>) {
    let v = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { -0.5 } else { 0.5 },
                if i & 2 == 0 { -0.5 } else { 0.5 },
                if i & 4 == 0 { -0.5 } else { 0.5 },
            ]
        })
        .collect();
    #[rustfmt::skip]
    let i = vec![
        0, 2, 1, 1, 2, 3,
        4, 5, 6, 5, 7, 6,
        0, 1, 4, 1, 5, 4,
        2, 6, 3, 3, 6, 7,
        0, 4, 2, 2, 4, 6,
        1, 3, 5, 3, 7, 5,
    ];
    (v, i)
}

fn translate(t: [f32; 3]) -> [f32; 12] {
    [1., 0., 0., t[0], 0., 1., 0., t[1], 0., 0., 1., t[2]]
}

fn wait(device: &wgpu::Device) {
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}

fn read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u32> {
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
    wait(device);
    let out = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
    buffer.unmap();
    out
}

/// A hit as (custom data + 1) << 16, front face in bit 15, t * 256 below.
fn describe(v: u32) -> String {
    if v == 0 {
        return "miss".into();
    }
    format!(
        "instance data {} t {:.3} {}",
        (v >> 16) - 1,
        (v & 0x7fff) as f32 / 256.,
        if v & 0x8000 != 0 { "front" } else { "back" }
    )
}

fn main() {
    let backends = match std::env::var("RTPROBE_BACKEND").as_deref() {
        Ok("vulkan") => wgpu::Backends::VULKAN,
        Ok("dx12") => wgpu::Backends::DX12,
        Ok("metal") => wgpu::Backends::METAL,
        _ => wgpu::Backends::PRIMARY,
    };
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    let instance = wgpu::Instance::new(descriptor);
    let wanted = std::env::var("RTPROBE_ADAPTER").unwrap_or_default().to_lowercase();
    let adapters = pollster::block_on(instance.enumerate_adapters(backends));
    for a in &adapters {
        let info = a.get_info();
        mark(&format!(
            "adapter seen: {} ({:?}, {:?}, driver {:?} {:?})",
            info.name, info.backend, info.device_type, info.driver, info.driver_info
        ));
    }
    let adapter = adapters
        .into_iter()
        .find(|a| a.get_info().name.to_lowercase().contains(&wanted))
        .expect("an adapter (RTPROBE_ADAPTER names part of one)");
    let info = adapter.get_info();
    let limits = adapter.limits();
    mark(&format!(
        "adapter: {} ({:?}, driver {:?} {:?}); EXPERIMENTAL_RAY_QUERY {}; max_tlas_instance_count {} max_blas_primitive_count {}",
        info.name,
        info.backend,
        info.driver,
        info.driver_info,
        adapter.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
        limits.max_tlas_instance_count,
        limits.max_blas_primitive_count,
    ));
    if !adapter.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) {
        mark("RESULT: no ray queries on this adapter");
        return;
    }

    mark("device: requesting");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rtprobe"),
        required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY,
        required_limits: wgpu::Limits::default().using_acceleration_structure_values(limits),
        // SAFETY: a probe of the experimental API; its faults are what
        // it's looking for.
        experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
        ..Default::default()
    }))
    .expect("a device with ray queries");
    device.on_uncaptured_error(std::sync::Arc::new(|e| panic!("wgpu error: {e}")));
    mark("device: made");

    mark("shader module (ray_query): creating");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("trace.wgsl"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    mark("shader module (ray_query): created");
    let visibility = wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility,
                ty: wgpu::BindingType::AccelerationStructure {
                    vertex_return: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    mark("compute pipeline (ray_query): creating");
    let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("cs_trace"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("cs_trace"),
        compilation_options: Default::default(),
        cache: None,
    });
    mark("compute pipeline (ray_query): created");
    mark("render pipeline (ray_query in fragment): creating");
    let fragment = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fs_trace"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_trace"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::R32Uint.into())],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    mark("render pipeline (ray_query in fragment): created");

    let (v, i) = cube();
    let size = wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: v.len() as u32,
        index_format: Some(wgpu::IndexFormat::Uint32),
        index_count: Some(i.len() as u32),
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
    };
    mark("BLAS: creating");
    let blas = device.create_blas(
        &wgpu::CreateBlasDescriptor {
            label: Some("cube"),
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        },
        wgpu::BlasGeometrySizeDescriptors::Triangles {
            descriptors: vec![size.clone()],
        },
    );
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&v),
        usage: wgpu::BufferUsages::BLAS_INPUT,
    });
    let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&i),
        usage: wgpu::BufferUsages::BLAS_INPUT,
    });
    mark("BLAS: created; build submitting");
    let mut e = device.create_command_encoder(&Default::default());
    e.build_acceleration_structures(
        [&wgpu::BlasBuildEntry {
            blas: &blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                size: &size,
                vertex_buffer: &vertices,
                first_vertex: 0,
                vertex_stride: 12,
                index_buffer: Some(&indices),
                first_index: Some(0),
                transform_buffer: None,
                transform_buffer_offset: None,
            }]),
        }],
        [],
    );
    queue.submit([e.finish()]);
    mark("BLAS build: submitted; waiting");
    wait(&device);
    mark("BLAS build: done");

    mark("TLAS (2 instances): creating");
    let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("two cubes"),
        max_instances: 2,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    });
    tlas[0] = Some(wgpu::TlasInstance::new(&blas, translate([0., 0., 0.]), 0, 0xff));
    tlas[1] = Some(wgpu::TlasInstance::new(&blas, translate([2., 0., 0.]), 7, 0xff));
    mark("TLAS: created; build submitting");
    let mut e = device.create_command_encoder(&Default::default());
    e.build_acceleration_structures([], [&tlas]);
    queue.submit([e.finish()]);
    mark("TLAS build: submitted; waiting");
    wait(&device);
    mark("TLAS build: done");

    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&camera()),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(N * N * 4),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    mark("bind group with the TLAS: creating");
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: tlas.as_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: out.as_entire_binding(),
            },
        ],
    });
    mark("bind group with the TLAS: created");
    let staging = |label| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: u64::from(N * N * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };

    mark("compute dispatch: submitting");
    let compute_read = staging("compute read");
    let mut e = device.create_command_encoder(&Default::default());
    {
        let mut pass = e.begin_compute_pass(&Default::default());
        pass.set_pipeline(&compute);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(N / 8, N / 8, 1);
    }
    e.copy_buffer_to_buffer(&out, 0, &compute_read, 0, u64::from(N * N * 4));
    queue.submit([e.finish()]);
    mark("compute dispatch: submitted; waiting and reading back");
    let from_compute = read(&device, &compute_read);
    mark("compute readback: done");

    mark("fragment draw: submitting");
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: N,
            height: N,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Uint,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let fragment_read = staging("fragment read");
    let mut e = device.create_command_encoder(&Default::default());
    {
        let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
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
        pass.set_pipeline(&fragment);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    e.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &fragment_read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(N * 4),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    queue.submit([e.finish()]);
    mark("fragment draw: submitted; waiting and reading back");
    let from_fragment = read(&device, &fragment_read);
    mark("fragment readback: done");

    // Each cube's centre, projected: the camera at x = 1 looking down -z.
    let project = |x: f32, z: f32| -> u32 {
        let ndc = (x - 1.) / ((3. - z) * 30f32.to_radians().tan());
        (((ndc + 1.) * 0.5 * N as f32) - 0.5).round() as u32
    };
    let (left, right, mid) = (project(0., 0.5), project(2., 0.5), N / 2);
    let at = |img: &[u32], x: u32, y: u32| img[(y * N + x) as usize];
    let mut ok = true;
    for (name, img) in [("compute", &from_compute), ("fragment", &from_fragment)] {
        let (l, r, m, corner) = (
            at(img, left, mid),
            at(img, right, mid),
            at(img, mid, mid),
            at(img, 0, 0),
        );
        mark(&format!(
            "{name}: left cube {} | right cube {} | between {} | corner {}",
            describe(l),
            describe(r),
            describe(m),
            describe(corner)
        ));
        let t_left = (l & 0x7fff) as f32 / 256.;
        ok &= (l >> 16) == 1
            && (r >> 16) == 8
            && m == 0
            && corner == 0
            && l & 0x8000 != 0
            && (t_left - 2.5).abs() < 0.25;
    }
    let same = from_compute == from_fragment;
    mark(&format!("compute and fragment images identical: {same}"));
    mark(&format!(
        "RESULT: {}",
        if ok && same { "PASS" } else { "FAIL" }
    ));
    if !(ok && same) {
        std::process::exit(1);
    }
}
