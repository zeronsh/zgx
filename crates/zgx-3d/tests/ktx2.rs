//! KTX2 textures (the `ktx2` feature): RGBA8 files read with their levels as
//! they're stored, the smaller ones the file's own; what isn't read says
//! why. Every file here is written by the test.
use std::time::{Duration, Instant};
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

const SIZE: f32 = 64.;

struct Window {
    ui: Ui,
    gpu: GpuRenderer,
}
impl Window {
    fn new(world: &World) -> Option<Self> {
        let gpu = match GpuRenderer::new(SIZE as u32, SIZE as u32) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let mut ui = Ui::new(SIZE, SIZE);
        ui.mount(
            div()
                .size(SIZE, SIZE)
                .bg(Color(0, 0, 0, 255))
                .child(gpu_view("World", world.view()).absolute().size(SIZE, SIZE)),
        );
        Some(Self { ui, gpu })
    }
    fn draw(&mut self) {
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap();
    }
    /// Frames until its world has every map, as a display would draw them.
    fn draw_loaded(&mut self, world: &World) {
        self.draw();
        let started = Instant::now();
        while world.loading() {
            assert!(started.elapsed() < Duration::from_secs(10), "still loading");
            std::thread::sleep(Duration::from_millis(2));
            world.view().invalidate();
            self.draw();
        }
    }
    fn middle(&self) -> [u8; 4] {
        let pixels = self.gpu.readback().unwrap();
        let at = ((SIZE as usize / 2) * SIZE as usize + SIZE as usize / 2) * 4;
        pixels[at..at + 4].try_into().unwrap()
    }
}

/// A KTX2 file of RGBA8 texels, `side` square: its first level `first`,
/// every smaller one `rest` (a full chain), in Vulkan `format` (37 RGBA8,
/// 43 its sRGB).
fn ktx2(side: u32, first: [u8; 4], rest: [u8; 4], format: u32) -> Vec<u8> {
    blocks(format, side, (1, 1), &first, &rest, false)
}

/// A KTX2 file in Vulkan `format` of `bw` by `bh` blocks (1 by 1: texels),
/// `side` square, a full chain: its first level all `first`, every smaller
/// one all `rest`; each level a Zstandard frame with `zstd`.
fn blocks(
    format: u32,
    side: u32,
    (bw, bh): (u32, u32),
    first: &[u8],
    rest: &[u8],
    zstd: bool,
) -> Vec<u8> {
    let count = 32 - side.leading_zeros();
    let levels: Vec<Vec<u8>> = (0..count)
        .map(|level| {
            let s = (side >> level).max(1);
            let unit = if level == 0 { first } else { rest };
            unit.repeat((s.div_ceil(bw) * s.div_ceil(bh)) as usize)
        })
        .collect();
    let stored: Vec<Vec<u8>> = if zstd {
        levels.iter().map(|level| zstd_frame(level)).collect()
    } else {
        levels.clone()
    };
    let mut file = b"\xabKTX 20\xbb\r\n\x1a\n".to_vec();
    let scheme = if zstd { 2 } else { 0 };
    for v in [format, 1, side, side, 0, 0, 1, count, scheme] {
        file.extend(v.to_le_bytes());
    }
    file.extend([0u8; 32]);
    let mut at = (80 + 24 * levels.len()) as u64;
    for (level, stored) in levels.iter().zip(&stored) {
        [at, stored.len() as u64, level.len() as u64]
            .iter()
            .for_each(|v| file.extend(v.to_le_bytes()));
        at += stored.len() as u64;
    }
    stored.iter().for_each(|level| file.extend(level));
    file
}

/// `bytes` as a Zstandard frame of raw (stored) blocks.
fn zstd_frame(bytes: &[u8]) -> Vec<u8> {
    // Magic; one segment, an 8-byte content size.
    let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0xe0];
    frame.extend((bytes.len() as u64).to_le_bytes());
    let chunks: Vec<&[u8]> = bytes.chunks(128 << 10).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let last = u32::from(i + 1 == chunks.len());
        frame.extend(&(((chunk.len() as u32) << 3) | last).to_le_bytes()[..3]);
        frame.extend(*chunk);
    }
    frame
}

/// BC1 blocks of one colour (RGB565 at both ends).
const BC1_RED: [u8; 8] = [0x00, 0xf8, 0x00, 0xf8, 0, 0, 0, 0];
const BC1_GREEN: [u8; 8] = [0xe0, 0x07, 0xe0, 0x07, 0, 0, 0, 0];
/// ASTC 4x4 blocks of one colour (void-extent blocks).
const ASTC_RED: [u8; 16] = [
    0xfc, 0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0xff, 0xff,
];
const ASTC_GREEN: [u8; 16] = [
    0xfc, 0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0xff, 0xff, 0, 0, 0xff, 0xff,
];

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

/// An unlit white square of `width` facing the camera 3 away, its map
/// `texture`.
fn square(width: f32, texture: Texture) -> World {
    let world = World::new();
    let mut material = Material::rgb(0xffffff);
    material.unlit = true;
    let mesh = world.add(Object::mesh(Geometry::cuboid(width, width, 0.01), material));
    world.set_texture(mesh, Some(texture));
    world.set_camera(Camera::looking_at([0., 0., 3.], [0., 0., 0.]));
    world
}

fn green(p: [u8; 4]) -> bool {
    p[1] > 200 && p[0] < 60 && p[2] < 60
}
fn red(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}

/// Near, its first level shows (red); far, a smaller one, the file's own
/// (green: one made from the first would be red).
fn levels_are_the_files_own(side: u32) {
    file_levels_show(&ktx2(side, RED, GREEN, 43), side, |_| true);
}

/// `file` (`side` square, its first level red, the rest green) drawn near
/// and far: its levels show whether the world's device holds its format
/// (`has`) or not, where its blocks are decoded to RGBA8 texels. There a
/// square drawn without its map would be white, so red near and green far
/// also say the decode was the path taken.
fn file_levels_show(file: &[u8], side: u32, has: impl Fn(&Caps) -> bool) {
    // Near: far wider than the view, its texels larger than pixels.
    for (width, near) in [(40., true), (0.12, false)] {
        let texture = Texture::decode(file).unwrap();
        assert_eq!((texture.width(), texture.height()), (side, side));
        let world = square(width, texture);
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        window.draw_loaded(&world);
        let p = window.middle();
        let how = match has(&world.caps().expect("a frame drawn")) {
            true => "its blocks on the device",
            false => "decoded, the device lacking its blocks",
        };
        assert!(
            if near { red(p) } else { green(p) },
            "near {near}, {how}: {p:?}"
        );
    }
}

#[test]
fn a_bc_ktx2_maps_blocks_go_to_the_gpu_as_they_are() {
    // BC1 (Vulkan 134, sRGB): 64² in its first frame, 256² by the texture
    // threads.
    for side in [64, 256] {
        let file = blocks(134, side, (4, 4), &BC1_RED, &BC1_GREEN, false);
        file_levels_show(&file, side, |caps| caps.compressed.bc);
    }
}

#[test]
fn an_astc_ktx2_maps_blocks_go_to_the_gpu_as_they_are() {
    // ASTC 4x4 (Vulkan 158, sRGB).
    let file = blocks(158, 256, (4, 4), &ASTC_RED, &ASTC_GREEN, false);
    file_levels_show(&file, 256, |caps| caps.compressed.astc);
}

#[test]
fn a_zstd_supercompressed_ktx2_maps_levels_are_decompressed() {
    // RGBA8 and BC1, each level a Zstandard frame.
    file_levels_show(&blocks(43, 256, (1, 1), &RED, &GREEN, true), 256, |_| true);
    let file = blocks(134, 64, (4, 4), &BC1_RED, &BC1_GREEN, true);
    file_levels_show(&file, 64, |caps| caps.compressed.bc);
}

#[test]
fn a_small_ktx2_maps_smaller_levels_are_the_files_own() {
    // 64²: put on the GPU in its first frame.
    levels_are_the_files_own(64);
}

#[test]
fn a_ktx2_map_loaded_off_the_frame_keeps_the_files_levels() {
    // 256²: by the texture threads, its smaller levels not made there.
    levels_are_the_files_own(256);
}

#[test]
fn a_ktx2_file_that_isnt_read_says_why() {
    let basis = ktx2(4, RED, GREEN, 0);
    let error = Texture::decode(&basis).unwrap_err();
    assert!(error.contains("Basis Universal"), "{error}");
    // BC1 of a size that isn't whole blocks.
    let error = Texture::decode(&blocks(134, 6, (4, 4), &BC1_RED, &BC1_GREEN, false)).unwrap_err();
    assert!(error.contains("whole blocks"), "{error}");
    let mut cut = ktx2(4, RED, GREEN, 43);
    cut.truncate(cut.len() - 1);
    assert!(Texture::decode(&cut).is_err());
}

#[test]
fn a_gltf_texture_names_its_ktx2_image_by_khr_texture_basisu() {
    // A quad whose texture's KTX2 image is given by the extension, with a
    // PNG-less fallback `source` past the images (as a file made for viewers
    // without it may not be): the extension's is read.
    let base64 = |bytes: &[u8]| {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
            for i in 0..4 {
                out.push(if i <= chunk.len() {
                    A[(n >> (18 - 6 * i) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        out
    };
    let mut bin = Vec::new();
    for p in [[-2f32, -2., 0.], [2., -2., 0.], [2., 2., 0.], [-2., 2., 0.]] {
        p.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    }
    for t in [[0f32, 1.], [1., 1.], [1., 0.], [0., 0.]] {
        t.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    }
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend(i.to_le_bytes());
    }
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"extensionsUsed":["KHR_texture_basisu"],"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[{{"mesh":0}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},"indices":2,"material":0}}]}}],
"materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}},"metallicFactor":0}},"extensions":{{"KHR_materials_unlit":{{}}}}}}],
"textures":[{{"source":5,"extensions":{{"KHR_texture_basisu":{{"source":0}}}}}}],
"images":[{{"uri":"data:image/ktx2;base64,{image}"}}],
"buffers":[{{"uri":"data:application/octet-stream;base64,{bin}","byteLength":{len}}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":48}},{{"buffer":0,"byteOffset":48,"byteLength":32}},{{"buffer":0,"byteOffset":80,"byteLength":12}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[-2,-2,0],"max":[2,2,0]}},{{"bufferView":1,"componentType":5126,"count":4,"type":"VEC2"}},{{"bufferView":2,"componentType":5123,"count":6,"type":"SCALAR"}}]}}"#,
        image = base64(&ktx2(64, RED, GREEN, 43)),
        bin = base64(&bin),
        len = bin.len(),
    );
    let model = Model::from_bytes(json.as_bytes()).expect("the model");
    let world = World::new();
    world.add_model(&model);
    world.add_light(Light::Hemisphere {
        sky: [1.; 3],
        ground: [1.; 3],
        intensity: 1.,
    });
    world.set_camera(Camera::looking_at([0., 0., 3.], [0., 0., 0.]));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw_loaded(&world);
    let p = window.middle();
    assert!(
        p[0] > 2 * p[1].max(p[2]) && p[0] > 60,
        "its map's red: {p:?}"
    );
}
