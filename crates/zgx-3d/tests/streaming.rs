//! Maps loaded off the frame (`World::loading`): a mesh draws in its
//! material's own colours until its map arrives, its world drawing frames
//! until it has, and frames drawn whole wait for it.
#![cfg(feature = "gltf")]
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
    fn middle(&self) -> [u8; 4] {
        let pixels = self.gpu.readback().unwrap();
        let at = ((SIZE as usize / 2) * SIZE as usize + SIZE as usize / 2) * 4;
        pixels[at..at + 4].try_into().unwrap()
    }
    /// Frames drawn until its world has every map (at most 10 s): one each
    /// refresh, as a host draws a view that animates (this window has no
    /// display to tick it).
    fn draw_until_loaded(&mut self, world: &World) -> usize {
        let started = Instant::now();
        let mut frames = 0;
        while world.loading() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "still loading after {frames} frames"
            );
            assert!(world.view().animating(), "it asks for frames meanwhile");
            std::thread::sleep(Duration::from_millis(2));
            world.view().invalidate();
            self.draw();
            frames += 1;
        }
        frames
    }
}

/// A PNG of `side` squared pixels of `rgba`, its data stored (zlib's stored
/// blocks), or cut short after its header with `broken`.
fn png(side: u32, rgba: [u8; 4], broken: bool) -> Vec<u8> {
    png_of(side, &|_, _| rgba, broken)
}
/// A PNG of `side` squared pixels, each `at(x, y)`'s, as `png` makes one.
fn png_of(side: u32, at: &dyn Fn(u32, u32) -> [u8; 4], broken: bool) -> Vec<u8> {
    let crc = |bytes: &[u8]| {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    0xedb8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    };
    let raw: Vec<u8> = (0..side)
        .flat_map(|y| std::iter::once(0).chain((0..side).flat_map(move |x| at(x, y))))
        .collect();
    let mut zlib = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65_535).enumerate() {
        zlib.push(u8::from((i + 1) * 65_535 >= raw.len()));
        let len = block.len() as u16;
        zlib.extend(len.to_le_bytes());
        zlib.extend((!len).to_le_bytes());
        zlib.extend(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend((b << 16 | a).to_be_bytes());
    if broken {
        zlib.truncate(64);
    }
    let mut file = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        file.extend((data.len() as u32).to_be_bytes());
        let body: Vec<u8> = kind.iter().chain(data).copied().collect();
        file.extend(&body);
        file.extend(crc(&body).to_be_bytes());
    };
    let header: Vec<u8> = [side.to_be_bytes(), side.to_be_bytes()].concat();
    chunk(b"IHDR", &[header, vec![8, 6, 0, 0, 0]].concat());
    chunk(b"IDAT", &zlib);
    chunk(b"IEND", &[]);
    file
}

/// A world of one unlit square of `colour` facing the camera, filling the
/// view, textured with `texture`.
fn square(colour: u32, texture: Texture) -> World {
    let world = World::new();
    let mut material = Material::rgb(colour);
    material.unlit = true;
    let mesh = world.add(Object::mesh(Geometry::cuboid(4., 4., 0.1), material));
    world.set_texture(mesh, Some(texture));
    world.set_camera(Camera::looking_at([0., 0., 3.], [0., 0., 0.]));
    world
}

fn red(pixel: [u8; 4]) -> bool {
    pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40
}
fn white(pixel: [u8; 4]) -> bool {
    pixel[..3].iter().all(|&c| c > 200)
}

#[test]
fn a_mesh_draws_its_own_colour_until_its_map_arrives() {
    // White, its map red: white until the map is on the GPU, then red. Its
    // world draws frames while it's on its way, and none after.
    let world = square(
        0xffffff,
        Texture::decode(&png(256, [255, 0, 0, 255], false)).unwrap(),
    );
    assert!(!world.loading(), "nothing asked for before a frame");
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert!(world.loading(), "its map on its way");
    assert!(world.view().animating(), "drawing frames till it comes");
    assert!(
        white(window.middle()),
        "its own colour: {:?}",
        window.middle()
    );
    window.draw_until_loaded(&world);
    assert!(red(window.middle()), "its map: {:?}", window.middle());
    assert!(!world.view().animating(), "and then at rest");
}

#[test]
fn a_frame_drawn_whole_waits_for_its_maps() {
    let world = square(
        0xffffff,
        Texture::decode(&png(256, [255, 0, 0, 255], false)).unwrap(),
    );
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let was = zgx::image::set_whole_frames_on_this_thread(true);
    window.draw();
    zgx::image::set_whole_frames_on_this_thread(was);
    assert!(!world.loading());
    assert!(red(window.middle()), "{:?}", window.middle());
}

#[test]
fn a_small_map_comes_with_its_first_frame() {
    // 128² or less: decoded in the frame, sooner than a frame without it.
    let world = square(
        0xffffff,
        Texture::decode(&png(128, [255, 0, 0, 255], false)).unwrap(),
    );
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert!(!world.loading());
    assert!(red(window.middle()), "{:?}", window.middle());
}

#[test]
fn a_map_that_fails_to_decode_ends_its_wait_in_its_own_colour() {
    let world = square(
        0x0000ff,
        Texture::decode(&png(256, [255, 0, 0, 255], true)).unwrap(),
    );
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    window.draw_until_loaded(&world);
    let [r, g, b, _] = window.middle();
    assert!(
        b > 200 && r < 40 && g < 40,
        "blue, its own: {:?}",
        [r, g, b]
    );
    assert!(!world.view().animating());
}

#[test]
fn a_world_dropped_with_maps_on_their_way_holds_up_no_other() {
    // Many maps asked for, then the world and its window gone: what was
    // asked for is dropped, and another world's map still comes.
    let world = World::new();
    world.set_camera(Camera::looking_at([0., 0., 3.], [0., 0., 0.]));
    for i in 0..32u8 {
        let mesh = world.add(Object::mesh(
            Geometry::cuboid(0.1, 0.1, 0.1),
            Material::rgb(0xffffff),
        ));
        world.set_texture(
            mesh,
            Some(Texture::decode(&png(512, [i, 0, 0, 255], false)).unwrap()),
        );
    }
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert!(world.loading());
    drop(window);
    drop(world);
    let other = square(
        0xffffff,
        Texture::decode(&png(256, [255, 0, 0, 255], false)).unwrap(),
    );
    let Some(mut window) = Window::new(&other) else {
        return;
    };
    window.draw();
    window.draw_until_loaded(&other);
    assert!(red(window.middle()), "{:?}", window.middle());
}

/// `bytes` as base64, for a data URI.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// A glTF of `count` lit quads side by side, each with a base colour and a
/// normal map of its own (256², so each loads on the pool's threads), all
/// inline.
fn model(count: usize) -> Model {
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
    let list = |f: &dyn Fn(usize) -> String| (0..count).map(f).collect::<Vec<_>>().join(",");
    let image = |i: usize| {
        let (colour, normal) = (
            png(256, [40 + i as u8 * 30, 200 - i as u8 * 20, 90, 255], false),
            png(
                256,
                [128 + i as u8 * 10, 128, 255 - i as u8 * 5, 255],
                false,
            ),
        );
        format!(
            r#"{{"uri":"data:image/png;base64,{}"}},{{"uri":"data:image/png;base64,{}"}}"#,
            base64(&colour),
            base64(&normal)
        )
    };
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[{roots}]}}],
"nodes":[{nodes}],"meshes":[{meshes}],"materials":[{materials}],"textures":[{textures}],"images":[{images}],
"buffers":[{{"uri":"data:application/octet-stream;base64,{bin}","byteLength":{len}}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":48}},{{"buffer":0,"byteOffset":48,"byteLength":48}},{{"buffer":0,"byteOffset":96,"byteLength":32}},{{"buffer":0,"byteOffset":128,"byteLength":12}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},{{"bufferView":1,"componentType":5126,"count":4,"type":"VEC3"}},{{"bufferView":2,"componentType":5126,"count":4,"type":"VEC2"}},{{"bufferView":3,"componentType":5123,"count":6,"type":"SCALAR"}}]}}"#,
        roots = list(&|i| i.to_string()),
        nodes = list(&|i| format!(
            r#"{{"mesh":{i},"translation":[{},{},0]}}"#,
            (i % 3) as f32 - 1.5,
            (i / 3) as f32 - 1.
        )),
        meshes = list(&|i| format!(
            r#"{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2}},"indices":3,"material":{i}}}]}}"#
        )),
        materials = list(&|i| format!(
            r#"{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":{}}},"metallicFactor":0}},"normalTexture":{{"index":{}}}}}"#,
            2 * i,
            2 * i + 1
        )),
        textures = (0..2 * count)
            .map(|i| format!(r#"{{"source":{i}}}"#))
            .collect::<Vec<_>>()
            .join(","),
        images = list(&image),
        bin = base64(&bin),
        len = bin.len(),
    );
    Model::from_bytes(json.as_bytes()).expect("the model")
}

#[test]
fn a_model_whose_maps_arrive_over_frames_ends_as_one_loaded_whole() {
    // The same model twice, lit: in one window its maps arrive over several
    // frames (drawn as they come), in the other a frame drawn whole waits
    // for every one. The last frames are the same pixels.
    let world = |model: &Model| {
        let world = World::new();
        world.add_model(model);
        world.add_light(Light::Directional {
            direction: vec3(-0.3, -0.5, -1.),
            color: [1.; 3],
            intensity: 3.,
        });
        world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
        world
    };
    let model = model(6);
    let (streamed, whole) = (world(&model), world(&model));
    let (Some(mut a), Some(mut b)) = (Window::new(&streamed), Window::new(&whole)) else {
        return;
    };
    a.draw();
    assert!(streamed.loading(), "its maps on their way");
    let frames = a.draw_until_loaded(&streamed);
    assert!(frames >= 1, "they came in later frames");
    let was = zgx::image::set_whole_frames_on_this_thread(true);
    b.draw();
    zgx::image::set_whole_frames_on_this_thread(was);
    assert!(!whole.loading());
    let (a, b) = (a.gpu.readback().unwrap(), b.gpu.readback().unwrap());
    let drawn = a.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
    assert!(drawn > a.len() / 16, "it drew: {drawn} pixels");
    assert!(a == b, "the same pixels");
}

#[test]
fn a_cut_out_casts_the_shadow_of_its_map_once_it_arrives() {
    // A square cut out by its base colour's alpha (its left half clear)
    // over a floor, in the sun: its shadow is drawn before its map arrives,
    // whole; once the map is in, the shadow is cut as it is, as in a frame
    // that waited for it.
    cut_out_shadow(false);
}

#[cfg(feature = "lighting")]
#[test]
fn a_cut_out_casts_the_shadow_of_its_map_into_cascades_once_it_arrives() {
    cut_out_shadow(true);
}

/// `a_cut_out_casts_the_shadow_of_its_map_once_it_arrives`, the sun's
/// shadow in one map or in `cascades`.
fn cut_out_shadow(cascades: bool) {
    #[cfg(not(feature = "lighting"))]
    let _ = cascades;
    let world = || {
        let world = World::new();
        world.set_shadows(true);
        #[cfg(feature = "lighting")]
        if cascades {
            world.set_cascades(Some(Cascades::new(2).distance(12.)));
        }
        world.add_light(Light::Directional {
            direction: vec3(0., -1., -0.2),
            color: [1.; 3],
            intensity: 3.,
        });
        world.add(
            Object::mesh(Geometry::cuboid(6., 0.1, 6.), Material::rgb(0xffffff)).at([0., -1., 0.]),
        );
        let half = |x: u32, _| {
            if x < 128 {
                [255, 255, 255, 0]
            } else {
                [255, 255, 255, 255]
            }
        };
        let leaf = world.add(
            Object::mesh(Geometry::cuboid(2., 0.05, 2.), Material::rgb(0x00ff00)).at([0., 0.5, 0.]),
        );
        world.set_alpha_cutoff(leaf, Some(0.5));
        world.set_texture(
            leaf,
            Some(Texture::decode(&png_of(256, &half, false)).unwrap()),
        );
        world.set_camera(Camera::looking_at([0., 4., 3.], [0., -1., 0.]));
        world
    };
    let (streamed, whole) = (world(), world());
    let (Some(mut a), Some(mut b)) = (Window::new(&streamed), Window::new(&whole)) else {
        return;
    };
    a.draw();
    assert!(streamed.loading(), "its map on its way");
    a.draw_until_loaded(&streamed);
    let was = zgx::image::set_whole_frames_on_this_thread(true);
    b.draw();
    zgx::image::set_whole_frames_on_this_thread(was);
    assert!(
        a.gpu.readback().unwrap() == b.gpu.readback().unwrap(),
        "the same shadow"
    );
}
