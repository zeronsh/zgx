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

#[test]
fn a_bc7_map_on_a_device_without_bc_draws_as_its_png() {
    // Both renderers made before any world asks for compressed textures.
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
    let (blocks, caps) = frame(a, Texture::decode(&ktx2()).unwrap());
    assert!(
        !caps.compressed.bc,
        "this device holds BC textures: the test can't see the decode"
    );
    let (pixels, _) = frame(b, Texture::decode(&png()).unwrap());
    let worst = blocks
        .iter()
        .zip(&pixels)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(
        worst <= 1,
        "the BC7 map drew up to {worst} levels from its PNG"
    );
    // And it drew the picture: its four colours.
    let at = |x: usize, y: usize| &blocks[(y * SIZE as usize + x) * 4..][..3];
    assert!(
        at(8, 8)[0] > 200 && at(56, 8)[1] > 200 && at(8, 56)[2] > 200,
        "{:?}",
        at(8, 8)
    );
}
