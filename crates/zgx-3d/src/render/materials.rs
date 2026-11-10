//! Group 1: a mesh's maps, their placements and its layers' numbers and
//! maps, bound once a set and kept.
use super::Gpu;
use crate::texture::{LAYER_MAPS, MAPS};
use zgx_gpu::wgpu;
/// Whether any of `d`'s maps reads other than its first coordinates as they
/// are: those need the placed pipelines, a wrap alone only a sampler.
pub(super) fn placed(d: &crate::world::Drawn) -> bool {
    d.extras
        .and_then(|e| e.placement())
        .is_some_and(|placement| {
            let mut maps = placement.iter().zip(d.maps);
            maps.any(|(p, map)| map.is_some() && !p.in_place())
        })
}
/// How `d`'s maps repeat: its first map's wrap, its base colour's if it has
/// one (a mesh's maps share one sampler).
pub(super) fn wrap_of(d: &crate::world::Drawn) -> [crate::Wrap; 2] {
    let Some(placement) = d.extras.and_then(|e| e.placement()) else {
        return [crate::Wrap::Repeat; 2];
    };
    let mut maps = placement.iter().zip(d.maps);
    maps.find(|(_, map)| map.is_some())
        .map_or([crate::Wrap::Repeat; 2], |(p, _)| p.wrap)
}
/// The maps `d` draws with, as a key: their ids (`Map::ALL`'s, 0 for none),
/// the first mixed with its placements' numbers, so meshes placed alike share
/// a binding. (No `Hasher`: this runs for every mesh, a frame, and a second
/// `DefaultHasher` caller outlined the hash of the lookups it's for.)
pub(super) fn map_set(d: &crate::world::Drawn) -> [u64; MAPS] {
    let mut maps = d.maps.map(|map| map.map_or(0, crate::Texture::id));
    let Some(extras) = d.extras else {
        return maps;
    };
    if let Some(placement) = extras.placement()
        && maps != [0; MAPS]
    {
        maps[0] ^= placement_key(placement);
    }
    // Light passing through or a coat binds group 1 with maps or without
    // (never 0).
    if extras.layered() {
        // Mixed in whole (a bit set over it made two textures whose ids
        // differ in that bit alone one binding), and never all 0.
        maps[0] ^= layers_key(extras);
        if maps == [0; MAPS] {
            maps[0] = 1;
        }
    }
    maps
}
/// Of a mesh's layer maps (`Extras::layer_maps`), how many are light
/// passing through's; a clear coat's follow.
pub(super) const GLASS_MAPS: usize = 2;
/// A number for how light passes through a mesh and its maps, for its coat
/// and its maps, and for its specular. Few meshes have any: out of
/// `map_set`'s code.
#[cold]
#[inline(never)]
pub(super) fn layers_key(extras: &crate::world::Extras) -> u64 {
    let mut key = 0x8422_2325_cbf2_9ce4_u64;
    let mut mix = |number: u64| key = (key ^ number).wrapping_mul(0x0100_0000_01b3);
    if let Some(transmission) = &extras.transmission {
        transmission
            .numbers()
            .into_iter()
            .for_each(|n| mix(n.into()));
        for map in extras
            .layer_maps()
            .iter()
            .flat_map(|maps| &maps[..GLASS_MAPS])
        {
            mix(map.as_ref().map_or(0, crate::Texture::id));
        }
    }
    if let Some(clearcoat) = extras.clearcoat() {
        // Apart from a transmission's numbers.
        mix(0xc0a7);
        clearcoat.numbers().into_iter().for_each(|n| mix(n.into()));
        for map in extras
            .layer_maps()
            .iter()
            .flat_map(|maps| &maps[GLASS_MAPS..])
        {
            mix(map.as_ref().map_or(0, crate::Texture::id));
        }
    }
    if let Some(specular) = extras.specular() {
        // Apart from both.
        mix(0x5bec);
        specular.numbers().into_iter().for_each(|n| mix(n.into()));
    }
    key
}
/// A number for what `placement` says, never 0.
pub(super) fn placement_key(placement: &[crate::MapPlacement; MAPS]) -> u64 {
    let numbers = placement.iter().flat_map(crate::MapPlacement::numbers);
    numbers.fold(0xcbf2_9ce4_8422_2325, |key, number| {
        (key ^ u64::from(number)).wrapping_mul(0x0100_0000_01b3)
    }) | 1
}

/// Group 1 for `d`: its maps, against stand-ins where one is missing, and
/// whatever else its pipeline binds. Made once a set of maps, rarely: out
/// of the frame's code.
#[cold]
#[inline(never)]
pub(super) fn make_binding(
    gpu: &mut Gpu,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    d: &crate::world::Drawn,
) -> wgpu::BindGroup {
    let stand_ins = gpu.stand_ins.get_or_insert_with(|| {
        let flat = |rgba: [u8; 4], srgb| {
            let texture = crate::Texture::from_rgba8(1, 1, rgba.to_vec()).unwrap();
            upload(device, queue, &texture, srgb).expect("its pixels, held")
        };
        [
            flat([255; 4], true),
            flat([255; 4], false),
            flat([128, 128, 255, 255], false),
            flat([255; 4], true),
            flat([255; 4], false),
        ]
    });
    let views = &gpu.views;
    let (layout, sampler) = (&gpu.texture_layout, &gpu.texture_sampler);
    let (placed_layout, wrapped) = (&mut gpu.placed_layout, &mut gpu.wrapped_samplers);
    let (layered_layout, glass_layout) = (&mut gpu.layered_layout, &mut gpu.glass_layout);
    let behind = &gpu.graph.behind;
    let view = |i: usize| {
        d.maps[i]
            .and_then(|t| views.get(&(t.id(), crate::Map::ALL[i].srgb())))
            .map_or(&stand_ins[i], |(view, _)| view)
    };
    let wrap = wrap_of(d);
    let sampler = if wrap == [crate::Wrap::Repeat; 2] {
        sampler
    } else {
        wrapped
            .entry(wrap)
            .or_insert_with(|| texture_sampler(device, wrap))
    };
    use wgpu::util::DeviceExt;
    let uniform = |label, contents: &[u8]| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage: wgpu::BufferUsages::UNIFORM,
        })
    };
    // A mesh light passes through, or with a coat, binds
    // placements too (as they are, if it has none) and their
    // numbers; one light passes through, what is behind it.
    let glass = d
        .extras
        .and_then(|e| e.transmission.as_ref())
        .zip(behind.as_ref());
    let coat = d.extras.and_then(|e| e.clearcoat());
    let specular = d.extras.and_then(|e| e.specular());
    let placement = d.extras.and_then(|e| e.placement()).filter(|_| placed(d));
    let layered = glass.is_some() || coat.is_some() || specular.is_some();
    let rows = (placement.is_some() || layered).then(|| {
        let rows = placement.copied().unwrap_or_default().map(|p| p.rows());
        uniform("3D map placements", bytemuck::cast_slice(&rows))
    });
    let words = layered.then(|| {
        let (a, b) = glass.map_or(([0.; 4], [0.; 4]), |(t, _)| {
            (
                [t.factor, t.ior, t.thickness, t.distance],
                [t.attenuation[0], t.attenuation[1], t.attenuation[2], 0.],
            )
        });
        let c = coat.map_or([0.; 4], |c| [c.factor, c.roughness, 0., 0.]);
        let d = specular.map_or([0.; 4], |s| {
            let [r, g, b] = s.f0();
            [r, g, b, s.factor]
        });
        uniform("3D layers", bytemuck::cast_slice(&[a, b, c, d]))
    });
    // Its layers' maps, uploaded for this binding alone (it keeps them):
    // light passing through's if it does, a coat's if it has one.
    // (A mesh with none, most, looks at none.)
    let mut layer_maps: [Option<wgpu::TextureView>; LAYER_MAPS] = Default::default();
    if let Some(maps) = d.extras.and_then(|e| e.layer_maps()) {
        for (i, map) in maps.iter().enumerate() {
            let used = if i < GLASS_MAPS {
                glass.is_some()
            } else {
                coat.is_some()
            };
            if let Some(texture) = map.as_ref().filter(|_| used) {
                layer_maps[i] = upload(device, queue, texture, false);
            }
        }
    }
    let glass = glass.map(|(_, behind)| (&behind.whole, &behind.sampler));
    let layout = if glass.is_some() {
        glass_layout.get_or_insert_with(|| map_layout(device, Extent::Glass))
    } else if layered {
        layered_layout.get_or_insert_with(|| map_layout(device, Extent::Layered))
    } else if rows.is_some() {
        placed_layout.get_or_insert_with(|| map_layout(device, Extent::Placed))
    } else {
        layout
    };
    // White where it has none (all passing, as thick as its transmission
    // says, as much coat and as rough as it says), a coat's normal flat.
    let layer_view = |i: usize| {
        let stand_in = &stand_ins[if i == LAYER_MAPS - 1 { 2 } else { 1 }];
        layer_maps[i].as_ref().unwrap_or(stand_in)
    };
    let layers = words
        .as_ref()
        .map(|words| (words, [2, 3, 4].map(layer_view)));
    let glass = glass.map(|(view, sampler)| (view, sampler, [0, 1].map(layer_view)));
    map_binding(
        device,
        layout,
        sampler,
        std::array::from_fn(view),
        rows.as_ref(),
        layers,
        glass,
    )
}
/// Group 1's layout as far as `extent`, made if no pipeline has it yet.
pub(super) fn make_layout(gpu: &mut Gpu, device: &wgpu::Device, extent: Option<Extent>) {
    let layout = match extent {
        Some(Extent::Placed) => &mut gpu.placed_layout,
        Some(Extent::Layered) => &mut gpu.layered_layout,
        Some(Extent::Glass) => &mut gpu.glass_layout,
        Some(Extent::Maps) | None => return,
    };
    if let Some(extent) = extent {
        layout.get_or_insert_with(|| map_layout(device, extent));
    }
}
/// Group 1's layout as far as `extent` (`None`: nothing), made
/// (`make_layout`).
pub(super) fn layout_of(gpu: &Gpu, extent: Option<Extent>) -> &wgpu::BindGroupLayout {
    fn made(layout: &Option<wgpu::BindGroupLayout>) -> &wgpu::BindGroupLayout {
        layout.as_ref().expect("made")
    }
    match extent {
        None => &gpu.empty_layout,
        Some(Extent::Maps) => &gpu.texture_layout,
        Some(Extent::Placed) => made(&gpu.placed_layout),
        Some(Extent::Layered) => made(&gpu.layered_layout),
        Some(Extent::Glass) => made(&gpu.glass_layout),
    }
}
/// How much group 1 binds: each extent all of the one before and more.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Extent {
    /// The maps and their sampler.
    Maps,
    /// Their placements as well.
    Placed,
    /// The numbers of a coat or of light passing through as well.
    Layered,
    /// What is behind a mesh light passes through, and its maps, as well.
    Glass,
}
/// Group 1's layout, as far as `extent`.
pub(super) fn map_layout(device: &wgpu::Device, extent: Extent) -> wgpu::BindGroupLayout {
    let fragment = wgpu::ShaderStages::FRAGMENT;
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: fragment,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let sampler = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: fragment,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    };
    let mut entries: Vec<wgpu::BindGroupLayoutEntry> = (0..MAPS as u32).map(texture).collect();
    entries.push(sampler(MAPS as u32));
    if extent >= Extent::Placed {
        entries.push(uniform(
            MAPS as u32 + 1,
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        ));
    }
    if extent >= Extent::Layered {
        // Their numbers, and a coat's maps: how much, how rough, which way
        // it faces.
        entries.extend([
            uniform(MAPS as u32 + 2, fragment),
            texture(MAPS as u32 + 7),
            texture(MAPS as u32 + 8),
            texture(MAPS as u32 + 9),
        ]);
    }
    if extent >= Extent::Glass {
        // What is behind, its sampler, and its maps: how much passes, how
        // thick.
        entries.extend([
            texture(MAPS as u32 + 3),
            sampler(MAPS as u32 + 4),
            texture(MAPS as u32 + 5),
            texture(MAPS as u32 + 6),
        ]);
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("3D maps"),
        entries: &entries,
    })
}
/// A mipmapped, anisotropic sampler wrapping as `wrap` says along u and v.
pub(super) fn texture_sampler(device: &wgpu::Device, wrap: [crate::Wrap; 2]) -> wgpu::Sampler {
    let mode = |wrap| match wrap {
        crate::Wrap::Clamp => wgpu::AddressMode::ClampToEdge,
        crate::Wrap::Mirror => wgpu::AddressMode::MirrorRepeat,
        _ => wgpu::AddressMode::Repeat,
    };
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("3D texture"),
        address_mode_u: mode(wrap[0]),
        address_mode_v: mode(wrap[1]),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: 8,
        ..Default::default()
    })
}

/// `texture` and its mipmaps on the GPU (written by the CPU where memory
/// is shared: no copy, see `texture_with_levels`), read as sRGB colours or
/// as data.
pub(super) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &crate::Texture,
    srgb: bool,
) -> Option<wgpu::TextureView> {
    let decoded = decoded(texture)?;
    Some(upload_decoded(device, queue, texture, srgb, decoded))
}
/// A texture's pixels and their smaller levels, ready to upload: decoding
/// them is most of an upload's time, and needs no device.
type Decoded = (
    std::sync::Arc<zgx::image::ImageData>,
    Vec<(u32, u32, Vec<u8>)>,
);
/// `texture`'s pixels (decoded from its file if it keeps one) and their
/// smaller levels; `None` if they don't decode.
fn decoded(texture: &crate::Texture) -> Option<Decoded> {
    let image = texture.pixels()?;
    let smaller = crate::texture::mipmaps(texture.width(), texture.height(), image.pixels());
    Some((image, smaller))
}
/// `texture`, `decoded`, on the GPU (`upload`).
fn upload_decoded(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &crate::Texture,
    srgb: bool,
    (image, smaller): Decoded,
) -> wgpu::TextureView {
    let levels: Vec<&[u8]> = std::iter::once(image.pixels())
        .chain(smaller.iter().map(|(_, _, pixels)| pixels.as_slice()))
        .collect();
    let made = zgx_gpu::viewport::texture_with_levels(
        device,
        queue,
        (texture.width(), texture.height()),
        if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        },
        &levels,
        "3D map",
    );
    texture.uploaded();
    made.create_view(&Default::default())
}
/// Threads decoding a frame's textures (`upload_missing`), the machine's
/// cores up to 4. Each holds one decoded texture and the channel as many
/// again, so about twice this many are decoded at once. Measured loading 64
/// textures of 1024² (M5 Max, file to first frame): 1 thread 427 ms, 2
/// 214, 4 141, 8 99; each kept about 11 MB more in the footprint at rest.
fn decoders() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().min(4))
}
/// `missing`, maps a frame draws that `gpu` hasn't uploaded (each read as
/// sRGB or not; one may come twice), uploaded and marked used this frame:
/// those whose files decode, more than `SMALL` pixels, decoded with their
/// smaller levels on `decoders()` threads and each put on the GPU here as
/// it's ready; the rest here, in turn. A map that doesn't decode draws as if
/// it had none (`Texture::pixels`). Once a frame with any new map: out of
/// the frame's code.
#[cold]
#[inline(never)]
pub(super) fn upload_missing(
    gpu: &mut Gpu,
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    mut missing: Vec<(&crate::Texture, bool)>,
) {
    let mut seen = std::collections::HashSet::new();
    missing.retain(|(texture, srgb)| seen.insert((texture.id(), *srgb)));
    let wanted = missing;
    let now = gpu.frame;
    // Pixels in hand, or few of them: here, in turn. A thread costs more
    // than they do.
    let (wanted, here): (Vec<_>, Vec<_>) = wanted.into_iter().partition(|(texture, _)| {
        !texture.holds_pixels() && texture.width() * texture.height() > crate::texture::SMALL
    });
    let workers = decoders().min(wanted.len());
    for (texture, srgb) in here
        .into_iter()
        .chain(wanted.iter().copied().filter(|_| workers <= 1))
    {
        if let Some(view) = upload(device, queue, texture, srgb) {
            gpu.views.insert((texture.id(), srgb), (view, now));
        }
    }
    if workers <= 1 {
        return;
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (sent, received) = std::sync::mpsc::sync_channel(workers);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (sent, next, wanted) = (sent.clone(), &next, &wanted);
            scope.spawn(move || {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&(texture, _)) = wanted.get(at) else {
                        break;
                    };
                    if sent.send((at, decoded(texture))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sent);
        for (at, decoded) in received {
            let (texture, srgb) = wanted[at];
            if let Some(decoded) = decoded {
                let view = upload_decoded(device, queue, texture, srgb, decoded);
                gpu.views.insert((texture.id(), srgb), (view, now));
            }
        }
    });
}
/// A mesh's maps (`Map::ALL`'s order) bound for sampling.
pub(super) fn map_binding(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    maps: [&wgpu::TextureView; MAPS],
    placements: Option<&wgpu::Buffer>,
    layers: Option<(&wgpu::Buffer, [&wgpu::TextureView; LAYER_MAPS - GLASS_MAPS])>,
    glass: Option<(
        &wgpu::TextureView,
        &wgpu::Sampler,
        [&wgpu::TextureView; GLASS_MAPS],
    )>,
) -> wgpu::BindGroup {
    let entries: Vec<wgpu::BindGroupEntry> = maps
        .iter()
        .enumerate()
        .map(|(binding, view)| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: wgpu::BindingResource::TextureView(view),
        })
        .chain(std::iter::once(wgpu::BindGroupEntry {
            binding: MAPS as u32,
            resource: wgpu::BindingResource::Sampler(sampler),
        }))
        .chain(placements.map(|buffer| wgpu::BindGroupEntry {
            binding: MAPS as u32 + 1,
            resource: buffer.as_entire_binding(),
        }))
        .chain(
            layers
                .into_iter()
                .flat_map(|(words, [coat, rough, normal])| {
                    [
                        wgpu::BindGroupEntry {
                            binding: MAPS as u32 + 2,
                            resource: words.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: MAPS as u32 + 7,
                            resource: wgpu::BindingResource::TextureView(coat),
                        },
                        wgpu::BindGroupEntry {
                            binding: MAPS as u32 + 8,
                            resource: wgpu::BindingResource::TextureView(rough),
                        },
                        wgpu::BindGroupEntry {
                            binding: MAPS as u32 + 9,
                            resource: wgpu::BindingResource::TextureView(normal),
                        },
                    ]
                }),
        )
        .chain(
            glass
                .into_iter()
                .flat_map(|(behind, sampler, [passing, thick])| {
                    [
                        wgpu::BindGroupEntry {
                            binding: MAPS as u32 + 3,
                            resource: wgpu::BindingResource::TextureView(behind),
                        },
                        wgpu::BindGroupEntry {
                            binding: MAPS as u32 + 4,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: MAPS as u32 + 5,
                            resource: wgpu::BindingResource::TextureView(passing),
                        },
                        wgpu::BindGroupEntry {
                            binding: MAPS as u32 + 6,
                            resource: wgpu::BindingResource::TextureView(thick),
                        },
                    ]
                }),
        )
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("3D maps"),
        layout,
        entries: &entries,
    })
}
