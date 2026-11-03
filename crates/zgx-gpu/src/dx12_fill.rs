//! An app's shader fill or layer shader on Direct3D 12, its fragment stage
//! compiled by FXC in a process of its own (`fxc_child`) and handed to wgpu
//! as bytecode (`create_shader_module_passthrough`, `PASSTHROUGH_SHADERS`).
//!
//! A passthrough module skips naga, so the HLSL must use the registers
//! wgpu-hal's root signature gives zgx's pipeline layouts: `options` writes
//! them as wgpu-hal's `create_pipeline_layout` assigns them (each group's
//! views in order, a CBV per uniform buffer and an SRV per texture or
//! read-only storage buffer; its samplers by index into the group's sampler
//! index buffer, an SRV after its views; the sampler heaps at s0 and s2048),
//! and naga writes the HLSL with wgpu-hal's other options. Checked against the
//! wgpu-hal named in `DX12_FILL_CHECKED_WGPU_HAL` (lib.rs; its test runs on
//! every platform's CI),
//! and by `app_shaders_d3d12`: the same shaders built by wgpu (naga) and
//! through here draw the same pixels, byte for byte.
use wgpu::naga;
use wgpu::naga::back::hlsl;

/// zgx's pipeline layouts an app's shader is drawn with.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Layout {
    /// The quads': the texture and its sampler; the viewport and regions.
    Quads,
    /// A blended layer's: the quads' groups and the backdrop texture.
    Blend,
}

/// What a bind group entry is, as wgpu-hal maps it.
#[derive(Clone, Copy)]
enum Entry {
    Texture(u32),
    Sampler(u32),
    Uniform(u32),
    StorageRead(u32),
}

/// `layout`'s groups, entries in the order the layouts declare them
/// (`create_quad_pipelines`, `blend_layouts`).
fn groups(layout: Layout) -> &'static [&'static [Entry]] {
    const QUADS: &[&[Entry]] = &[
        &[Entry::Texture(1), Entry::Sampler(2)],
        &[Entry::Uniform(0), Entry::StorageRead(1)],
    ];
    const BLEND: &[&[Entry]] = &[
        &[Entry::Texture(1), Entry::Sampler(2)],
        &[Entry::Uniform(0), Entry::StorageRead(1)],
        &[Entry::Texture(0)],
    ];
    match layout {
        Layout::Quads => QUADS,
        Layout::Blend => BLEND,
    }
}

/// naga's HLSL options for `layout`, as wgpu-hal writes them for its root
/// signature (`wgpu-hal/src/dx12/device.rs`, `create_pipeline_layout`), at
/// shader model 5.1, FXC's.
pub(crate) fn options(layout: Layout) -> hlsl::Options {
    let target = |space: u8, register: u32| hlsl::BindTarget {
        space,
        register,
        binding_array_size: None,
        dynamic_storage_buffer_offsets_index: None,
        restrict_indexing: false,
    };
    let mut binding_map = hlsl::BindingMap::default();
    let mut sampler_buffer_binding_map = hlsl::SamplerIndexBufferBindingMap::default();
    let (mut cbv, mut srv) = (0u32, 0u32);
    for (group, entries) in groups(layout).iter().enumerate() {
        let group = group as u32;
        let binding = |binding| naga::ResourceBinding { group, binding };
        // Views, in order.
        for entry in *entries {
            match *entry {
                Entry::Uniform(b) => {
                    binding_map.insert(binding(b), target(0, cbv));
                    cbv += 1;
                }
                Entry::Texture(b) | Entry::StorageRead(b) => {
                    binding_map.insert(binding(b), target(0, srv));
                    srv += 1;
                }
                Entry::Sampler(_) => {}
            }
        }
        // Samplers, by index into the group's sampler index buffer.
        let mut samplers = 0;
        for entry in *entries {
            if let Entry::Sampler(b) = *entry {
                binding_map.insert(binding(b), target(255, samplers));
                samplers += 1;
            }
        }
        if samplers != 0 {
            sampler_buffer_binding_map
                .insert(hlsl::SamplerIndexBufferKey { group }, target(0, srv));
            srv += 1;
        }
    }
    hlsl::Options {
        shader_model: hlsl::ShaderModel::V5_1,
        binding_map,
        fake_missing_bindings: false,
        special_constants_binding: None,
        immediates_target: None,
        dynamic_storage_buffer_offsets_targets: Default::default(),
        zero_initialize_workgroup_memory: true,
        restrict_indexing: true,
        sampler_heap_target: hlsl::SamplerHeapBindTargets {
            standard_samplers: target(0, 0),
            comparison_samplers: target(0, 2048),
        },
        sampler_buffer_binding_map,
        external_texture_binding_map: Default::default(),
        force_loop_bounding: true,
        ray_query_initialization_tracking: true,
    }
}

/// `source`'s fragment entry `entry` as HLSL for `layout`, and its name in
/// the HLSL.
pub(crate) fn fragment_hlsl(
    source: &str,
    entry: &str,
    layout: Layout,
) -> Result<(String, String), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.to_string())?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| e.to_string())?;
    let pipeline = hlsl::PipelineOptions {
        entry_point: Some((naga::ShaderStage::Fragment, entry.to_owned())),
    };
    let options = options(layout);
    let mut hlsl = String::new();
    let mut reflection = hlsl::Writer::new(&mut hlsl, &options, &pipeline)
        .write(&module, &info, None)
        .map_err(|e| e.to_string())?;
    let name = reflection
        .entry_point_names
        .pop()
        .ok_or("no entry point")?
        .map_err(|e| e.to_string())?;
    Ok((hlsl, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quads' registers as wgpu-hal gives them: the texture t0, its
    /// sampler's index buffer t1, the viewport b0, the regions t2; the
    /// blend's backdrop t3.
    #[test]
    fn the_registers_follow_wgpu_hals_order() {
        let at = |options: &hlsl::Options, group, binding| {
            let t = options.binding_map[&naga::ResourceBinding { group, binding }];
            (t.space, t.register)
        };
        let quads = options(Layout::Quads);
        assert_eq!(at(&quads, 0, 1), (0, 0));
        assert_eq!(at(&quads, 0, 2), (255, 0));
        assert_eq!(at(&quads, 1, 0), (0, 0));
        assert_eq!(at(&quads, 1, 1), (0, 2));
        let buffer = quads.sampler_buffer_binding_map[&hlsl::SamplerIndexBufferKey { group: 0 }];
        assert_eq!((buffer.space, buffer.register), (0, 1));
        assert_eq!(at(&options(Layout::Blend), 2, 0), (0, 3));
    }

    /// A fill's fragment stage translates for both layouts.
    #[test]
    fn a_fill_translates() {
        let source = crate::fill_source(
            "fn fill(input: FillInput) -> vec4<f32> { return vec4(input.p / input.size, 0.0, 1.0); }",
        );
        let (hlsl, name) = fragment_hlsl(&source, "zgx_fill", Layout::Quads).unwrap();
        assert!(hlsl.contains(&name), "{name}");
        assert!(hlsl.contains("register(t2)"), "the regions at t2");
    }
}
