//! A block-compressed KTX2 map on a device made without compressed
//! textures (its renderer made before any world asks for them, as a world
//! made after `run` gets): decoded to RGBA8, it draws as the same picture
//! given as a PNG. A test binary of its own: the request is the process's.
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

const SIZE: u32 = 64;

/// A 64² picture of four colours, a quarter each, every channel odd (so a
/// BC7 block's 7-bit endpoints and its p-bit hold it exactly).
fn picture(x: u32, y: u32) -> [u8; 4] {
    match (x < 32, y < 32) {
        (true, true) => [255, 1, 1, 255],
        (false, true) => [1, 255, 1, 255],
        (true, false) => [1, 1, 255, 255],
        (false, false) => [255, 255, 255, 255],
    }
}

/// A BC7 mode 6 block of one colour `c` (every channel odd): both
/// endpoints `c >> 1` with p-bits of 1, every index 0.
fn bc7_solid(c: [u8; 4]) -> [u8; 16] {
    let mut bits: u128 = 1 << 6;
    let mut at = 7;
    for channel in c {
        for _ in 0..2 {
            bits |= u128::from(channel >> 1) << at;
            at += 7;
        }
    }
    bits |= 1 << 63 | 1 << 64;
    bits.to_le_bytes()
}

/// The picture as a KTX2 file of BC7 blocks (sRGB), every level its blocks
/// of the picture's colour at that place.
fn ktx2() -> Vec<u8> {
    let count = 32 - SIZE.leading_zeros();
    let levels: Vec<Vec<u8>> = (0..count)
        .map(|level| {
            let side = (SIZE >> level).max(1);
            let blocks = side.div_ceil(4);
            let scale = SIZE / side;
            (0..blocks)
                .flat_map(|by| (0..blocks).map(move |bx| (bx, by)))
                .flat_map(|(bx, by)| bc7_solid(picture(bx * 4 * scale, by * 4 * scale)))
                .collect()
        })
        .collect();
    let mut file = b"\xabKTX 20\xbb\r\n\x1a\n".to_vec();
    for v in [146, 1, SIZE, SIZE, 0, 0, 1, count, 0] {
        file.extend(u32::to_le_bytes(v));
    }
    file.extend([0u8; 32]);
    let mut at = (80 + 24 * levels.len()) as u64;
    for level in &levels {
        let length = level.len() as u64;
        [at, length, length]
            .iter()
            .for_each(|v| file.extend(v.to_le_bytes()));
        at += length;
    }
    levels.iter().for_each(|level| file.extend(level));
    file
}

/// The picture as a PNG.
fn png() -> Vec<u8> {
    let pixels: Vec<u8> = (0..SIZE)
        .flat_map(|y| (0..SIZE).flat_map(move |x| picture(x, y)))
        .collect();
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, SIZE, SIZE);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    bytes
}

/// A frame of an unlit white square filling the view, its map `texture`,
/// drawn by `gpu` (made beforehand); and what its world's device holds.
fn frame(mut gpu: GpuRenderer, texture: Texture) -> (Vec<u8>, Caps) {
    let world = World::new();
    let mut material = Material::rgb(0xffffff);
    material.unlit = true;
    let mesh = world.add(Object::mesh(Geometry::cuboid(2.4, 2.4, 0.01), material));
    world.set_texture(mesh, Some(texture));
    world.set_camera(Camera::looking_at([0., 0., 3.], [0., 0., 0.]));
    let side = SIZE as f32;
    let mut ui = Ui::new(side, side);
    ui.mount(
        div()
            .size(side, side)
            .bg(Color(0, 0, 0, 255))
            .child(gpu_view("World", world.view()).absolute().size(side, side)),
    );
    let mut draw = || {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
    };
    draw();
    let started = std::time::Instant::now();
    while world.loading() {
        assert!(started.elapsed().as_secs() < 10, "still loading");
        std::thread::sleep(std::time::Duration::from_millis(2));
        world.view().invalidate();
        draw();
    }
    (
        gpu.readback().unwrap(),
        world.caps().expect("a frame drawn"),
    )
}

/// The picture as a Basis Universal (UASTC) KTX2 file: each block its
/// colour in UASTC's solid-colour mode, every level, UASTC's data format
/// descriptor (colour model 166).
#[cfg(feature = "ktx2-basis")]
fn uastc() -> Vec<u8> {
    let solid = |[r, g, b, a]: [u8; 4]| -> [u8; 16] {
        let bits: u128 = 0x17
            | u128::from(r) << 5
            | u128::from(g) << 13
            | u128::from(b) << 21
            | u128::from(a) << 29;
        bits.to_le_bytes()
    };
    let count = 32 - SIZE.leading_zeros();
    let levels: Vec<Vec<u8>> = (0..count)
        .map(|level| {
            let side = (SIZE >> level).max(1);
            let (blocks, scale) = (side.div_ceil(4), SIZE / side);
            (0..blocks)
                .flat_map(|by| (0..blocks).map(move |bx| (bx, by)))
                .flat_map(|(bx, by)| solid(picture(bx * 4 * scale, by * 4 * scale)))
                .collect()
        })
        .collect();
    let dfd: Vec<u8> = [
        44u32,
        0,
        2 | 40 << 16,
        166 | 1 << 8 | 2 << 16,
        3 | 3 << 8,
        16,
        0,
        127 << 16 | 3 << 24,
        0,
        0,
        u32::MAX,
    ]
    .iter()
    .flat_map(|v| v.to_le_bytes())
    .collect();
    let mut file = b"\xabKTX 20\xbb\r\n\x1a\n".to_vec();
    for v in [0, 1, SIZE, SIZE, 0, 0, 1, count, 0] {
        file.extend(u32::to_le_bytes(v));
    }
    for v in [80 + 24 * count, dfd.len() as u32, 0, 0] {
        file.extend(v.to_le_bytes());
    }
    file.extend([0u8; 16]);
    let index_at = file.len();
    file.resize(index_at + 24 * count as usize, 0);
    file.extend(&dfd);
    for (i, level) in levels.iter().enumerate() {
        file.resize(file.len().next_multiple_of(16), 0);
        let length = level.len() as u64;
        let (at, here) = (index_at + 24 * i, file.len() as u64);
        file[at..at + 24].copy_from_slice(&[here, length, length].map(u64::to_le_bytes).concat());
        file.extend(level);
    }
    file
}

#[test]
fn a_bc7_map_on_a_device_without_bc_draws_as_its_png() {
    // Every renderer made before any world asks for compressed textures
    // (the first world made asks, for the devices made after it).
    let made = || match GpuRenderer::new(SIZE, SIZE) {
        Ok(gpu) => Some(gpu),
        Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
            panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
        }
        Err(_) => None,
    };
    let (Some(a), Some(b)) = (made(), made()) else {
        return;
    };
    #[cfg(feature = "ktx2-basis")]
    let basis = made();
    let hostile = made();
    let (blocks, caps) = frame(a, Texture::decode(&ktx2()).unwrap());
    assert!(
        !caps.compressed.bc && !caps.compressed.astc,
        "this device holds compressed textures: the test can't see the decode"
    );
    let (pixels, _) = frame(b, Texture::decode(&png()).unwrap());
    let close = |drawn: &[u8], what: &str| {
        let worst = drawn
            .iter()
            .zip(&pixels)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            worst <= 1,
            "the {what} map drew up to {worst} levels from its PNG"
        );
        // And it drew the picture: its four colours.
        let at = |x: usize, y: usize| &drawn[(y * SIZE as usize + x) * 4..][..3];
        assert!(
            at(8, 8)[0] > 200 && at(56, 8)[1] > 200 && at(8, 56)[2] > 200,
            "{what}: {:?}",
            at(8, 8)
        );
    };
    close(&blocks, "BC7");
    // A Basis Universal map there: transcoded to RGBA8, the same picture.
    #[cfg(feature = "ktx2-basis")]
    if let Some(gpu) = basis {
        let (transcoded, _) = frame(gpu, Texture::decode(&uastc()).unwrap());
        close(&transcoded, "Basis Universal");
    }
    // An 8² map (decoded in the frame, as small) of ASTC blocks that panic
    // the decoder (one found by seeded search: texture2ddecoder 0.1.2 reads
    // past its tables): caught, the frame drawn, the square white, as if
    // it had no map.
    if let Some(gpu) = hostile {
        const PANICS: [u8; 16] = [
            173, 118, 54, 116, 236, 121, 207, 234, 139, 142, 21, 3, 253, 158, 31, 255,
        ];
        let file = ktx2_of(8, 157, &PANICS.repeat(4));
        let (drawn, _) = frame(gpu, Texture::decode(&file).unwrap());
        let middle = &drawn[((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize..][..3];
        assert!(
            middle.iter().all(|&c| c > 250),
            "white, as without its map: {middle:?}"
        );
    }
}

/// A KTX2 file `side` square of one level of Vulkan `format`'s `blocks`.
fn ktx2_of(side: u32, format: u32, blocks: &[u8]) -> Vec<u8> {
    let mut file = b"\xabKTX 20\xbb\r\n\x1a\n".to_vec();
    for v in [format, 1, side, side, 0, 0, 1, 1, 0] {
        file.extend(u32::to_le_bytes(v));
    }
    file.extend([0u8; 32]);
    let length = blocks.len() as u64;
    [104, length, length]
        .iter()
        .for_each(|v: &u64| file.extend(v.to_le_bytes()));
    file.extend(blocks);
    file
}
