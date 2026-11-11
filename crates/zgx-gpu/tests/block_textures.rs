//! `viewport::texture_with_block_levels`: a BC and an ASTC texture made
//! with every level, as the device has their features, without a
//! validation error. (A world's tests draw their texels: zgx-3d's KTX2
//! tests.)
use zgx_gpu::viewport::texture_with_block_levels;

/// A device with BC and ASTC textures where the adapter has them.
fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        assert!(
            std::env::var_os("ZGX_REQUIRE_GPU").is_none(),
            "ZGX_REQUIRE_GPU, but no adapter"
        );
        return None;
    };
    let wanted = wgpu::Features::TEXTURE_COMPRESSION_BC | wgpu::Features::TEXTURE_COMPRESSION_ASTC;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wanted,
        ..Default::default()
    }))
    .ok()
}

/// `format`'s texture of `size` from `block` repeated, every level, made
/// in an error scope: its validation error, if any.
fn made(
    (device, queue): &(wgpu::Device, wgpu::Queue),
    format: wgpu::TextureFormat,
    size: (u32, u32),
    block: &[u8],
) -> Option<wgpu::Error> {
    let (bw, bh) = format.block_dimensions();
    let count = 32 - size.0.max(size.1).leading_zeros();
    let levels: Vec<Vec<u8>> = (0..count)
        .map(|level| {
            let (w, h) = ((size.0 >> level).max(1), (size.1 >> level).max(1));
            block.repeat((w.div_ceil(bw) * h.div_ceil(bh)) as usize)
        })
        .collect();
    let levels: Vec<&[u8]> = levels.iter().map(Vec::as_slice).collect();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let texture = texture_with_block_levels(device, queue, size, format, &levels, "test");
    assert_eq!(texture.format(), format);
    assert_eq!(texture.mip_level_count(), count);
    assert_eq!((texture.width(), texture.height()), size);
    queue.submit([]);
    pollster::block_on(scope.pop())
}

/// A BC1 block of one colour, red (RGB565 0xf800 at both ends).
const BC1_RED: [u8; 8] = [0x00, 0xf8, 0x00, 0xf8, 0, 0, 0, 0];
/// An ASTC 4x4 block of one colour (a void-extent block), red.
const ASTC_RED: [u8; 16] = [
    0xfc, 0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0xff, 0xff,
];

#[test]
fn a_bc_texture_is_made_with_every_level() {
    let Some(gpu) = device() else { return };
    if !gpu
        .0
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    {
        eprintln!("SKIPPED: this adapter has no BC textures");
        return;
    }
    for format in [
        wgpu::TextureFormat::Bc1RgbaUnorm,
        wgpu::TextureFormat::Bc1RgbaUnormSrgb,
    ] {
        // 20 by 12: levels 10x6, 5x3, 2x1, 1x1 run past their last blocks.
        assert!(
            made(&gpu, format, (20, 12), &BC1_RED).is_none(),
            "{format:?}"
        );
    }
}

#[test]
fn an_astc_texture_is_made_with_every_level() {
    let Some(gpu) = device() else { return };
    if !gpu
        .0
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_ASTC)
    {
        eprintln!("SKIPPED: this adapter has no ASTC textures");
        return;
    }
    let format = wgpu::TextureFormat::Astc {
        block: wgpu::AstcBlock::B4x4,
        channel: wgpu::AstcChannel::UnormSrgb,
    };
    assert!(made(&gpu, format, (16, 8), &ASTC_RED).is_none());
}

#[test]
#[should_panic(expected = "whole 4 by 4 blocks")]
fn a_size_of_part_blocks_is_refused() {
    let Some(gpu) = device() else {
        panic!("whole 4 by 4 blocks (no adapter: stood in)")
    };
    made(&gpu, wgpu::TextureFormat::Bc1RgbaUnorm, (6, 4), &BC1_RED);
}
