//! Reference images of worlds, two ways.
//!
//! **Fresh against retained:** each scene drawn by a window made for it, and
//! by one that reached the same state the long way round, by each of
//! `SEQUENCES` in turn: other sizes, settings turned off and on, materials
//! changed and changed back, an object added and removed, and what each kept
//! structure adds. Stale caches or history show as a difference: none at
//! all on a software adapter, at most one level a channel on hardware. A
//! sequence may also check what pixels can't show (a count) against the
//! fresh world's.
//!
//! **Goldens:** each scene against a PNG in `tests/reference/<family>/`, one
//! family an adapter kind (`llvmpipe`, `warp`, `metal`, …: software and
//! hardware rasterise differently), with a tolerance per image: how far a
//! channel may differ, and the fraction of pixels allowed past that.
//! `ZGX_BLESS=1` writes the running family's images. A family with no
//! goldens is reported, and fails where a GPU is required
//! (`ZGX_REQUIRE_GPU`, as in CI) or `ZGX_REQUIRE_GOLDENS` is set. A failing
//! comparison writes the actual, expected and difference images to a
//! temporary folder and prints their paths; where a GPU is required it also
//! prints each actual image as a `ZGX-GOLDEN <path> <base64 PNG>` line, so a
//! CI run's log can bless a family no machine at hand has
//! (`scripts/goldens_from_log.py`). The folder holds at most 5 MB, every
//! image 256² at most.
use std::{
    f32::consts::PI,
    path::{Path, PathBuf},
};
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

/// Each image's side: small, so three families fit the budget.
const SIDE: u32 = 128;
const BUDGET: u64 = 5 << 20;

struct Window {
    ui: Ui,
    gpu: GpuRenderer,
    size: (u32, u32),
    /// Its world, to wait for custom materials' compiles (`draw`).
    #[cfg(feature = "shaders")]
    world: World,
}
impl Window {
    fn new(world: &World, (w, h): (u32, u32)) -> Option<Self> {
        // Every window on one device (a thread's first window's context),
        // as an app's windows are: on the M1 Max, each new device drew a
        // scene's silhouette edges as one of two images (`APPLE7`).
        thread_local! {
            static SHARED: std::cell::OnceCell<std::mem::ManuallyDrop<zgx_gpu::GpuContext>> =
                const { std::cell::OnceCell::new() };
        }
        let made = SHARED.with(|shared| match shared.get() {
            Some(context) => GpuRenderer::new_with_context(w, h, context),
            None => GpuRenderer::new(w, h).inspect(|gpu| {
                let _ = shared.set(std::mem::ManuallyDrop::new(gpu.context()));
            }),
        });
        let gpu = match made {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let mut ui = Ui::new(w as f32, h as f32);
        ui.mount(
            div()
                .w_full()
                .h_full()
                .bg(Color(0, 0, 0, 255))
                .child(gpu_view("World", world.view()).w_full().h_full()),
        );
        Some(Self {
            ui,
            gpu,
            size: (w, h),
            #[cfg(feature = "shaders")]
            world: world.clone(),
        })
    }
    /// A frame, and with custom materials more while one's pipeline
    /// compiles off the frame, until it's drawn by it: what a window's
    /// frame clock draws.
    fn draw(&mut self) {
        self.frame();
        #[cfg(feature = "shaders")]
        {
            let start = std::time::Instant::now();
            while self.world.compiling() && start.elapsed() < std::time::Duration::from_secs(20) {
                std::thread::sleep(std::time::Duration::from_millis(2));
                self.world.view().invalidate();
                self.frame();
            }
        }
    }
    fn frame(&mut self) {
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap();
    }
    fn resize(&mut self, (w, h): (u32, u32)) {
        self.size = (w, h);
        self.gpu.resize(w, h);
        self.ui.scene.borrow_mut().resize(w as f32, h as f32);
        self.draw();
    }
    fn pixels(&self) -> Vec<u8> {
        self.gpu.readback().unwrap()
    }
    /// Which goldens this adapter is compared with.
    fn family(&self) -> String {
        let info = self.gpu.adapter_info();
        let name = info.name.to_lowercase();
        if name.contains("llvmpipe") {
            "llvmpipe".into()
        } else if name.contains("basic render") || name.contains("warp") {
            "warp".into()
        } else if info.backend == zgx_gpu::wgpu::Backend::Metal {
            "metal".into()
        } else {
            let name: String = name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect();
            format!("{:?}-{name}", info.backend).to_lowercase()
        }
    }
    fn software(&self) -> bool {
        self.gpu.adapter_info().device_type == zgx_gpu::wgpu::DeviceType::Cpu
    }
}

/// How close an image must be to its golden.
#[derive(Clone, Copy)]
struct Tolerance {
    /// Levels (of 255) a channel may differ by and still match.
    level: u8,
    /// The fraction of pixels allowed to differ by more.
    fraction: f64,
}
const CLOSE: Tolerance = Tolerance {
    level: 2,
    fraction: 0.002,
};
/// Shadow edges and glass, which move a little with rasterisation.
const EDGES: Tolerance = Tolerance {
    level: 3,
    fraction: 0.01,
};

/// A scene: what it holds and how it's seen. `settle` sets every world-wide
/// setting the retained window changes, so both end alike.
struct Scene {
    name: &'static str,
    tolerance: Tolerance,
    build: fn(&World) -> Vec<ObjectId>,
    shadows: bool,
    tone: ToneMapping,
    environment: fn() -> Option<Environment>,
    camera: Camera,
}
impl Scene {
    fn settle(&self, world: &World) {
        world.set_shadows(self.shadows);
        world.set_tone_mapping(self.tone);
        world.set_environment((self.environment)());
        world.set_camera(self.camera);
        world.set_samples(4);
        world.set_exposure(0.);
        world.set_background(Some(0x203040));
    }
    fn world(&self) -> (World, Vec<ObjectId>) {
        let world = World::new();
        let objects = (self.build)(&world);
        self.settle(&world);
        (world, objects)
    }
}

fn camera(position: [f32; 3], target: [f32; 3]) -> Camera {
    Camera::looking_at(position, target)
}
fn sun(world: &World, direction: [f32; 3], intensity: f32) {
    world.add_light(Light::Directional {
        direction: direction.into(),
        color: [1., 0.97, 0.92],
        intensity,
    });
}
fn floor(world: &World) -> ObjectId {
    world.add(Object::mesh(
        Geometry::plane(6., 6.),
        Material::rgb(0x9a948a).with(0., 0.8),
    ))
}
/// A sky bright above, a warm horizon, dark below: made here, not read.
fn sky() -> Option<Environment> {
    let (w, h) = (64u32, 32u32);
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let lat = (y as f32 + 0.5) / h as f32 * PI;
            let lon = (x as f32 + 0.5) / w as f32 * 2. * PI;
            let up = lat.cos();
            let sun = if up > 0.3 && (lon - 1.).abs() < 0.3 {
                6.
            } else {
                0.
            };
            let c = if up > 0. {
                [0.4 + 0.6 * up + sun, 0.55 + 0.4 * up + sun, 0.9 + sun]
            } else {
                [0.15, 0.12, 0.1]
            };
            rgb.extend(c);
        }
    }
    Some(Environment::from_equirect(w, h, &rgb).unwrap())
}

fn scenes() -> Vec<Scene> {
    #[cfg_attr(not(feature = "lighting"), allow(unused_mut))]
    let mut scenes = vec![
        Scene {
            name: "lighting",
            tolerance: CLOSE,
            build: |world| {
                let ball = world.add(
                    Object::mesh(
                        Geometry::sphere(0.6, 48),
                        Material::rgb(0xd04030).with(0., 0.4),
                    )
                    .at([-0.8, 0.6, 0.]),
                );
                let cube = world.add(
                    Object::mesh(
                        Geometry::cuboid(0.9, 0.9, 0.9),
                        Material::rgb(0x3060c0).with(0.5, 0.3),
                    )
                    .at([0.8, 0.45, 0.]),
                );
                let f = floor(world);
                sun(world, [-0.4, -1., -0.3], 2.);
                world.add_light(Light::Point {
                    position: vec3(0., 1.2, 1.2),
                    color: [1., 0.6, 0.2],
                    intensity: 3.,
                    range: 4.,
                });
                let spot = world.add_light(Light::Point {
                    position: vec3(1.5, 2., 1.),
                    color: [0.3, 0.6, 1.],
                    intensity: 6.,
                    range: 5.,
                });
                world.set_spot(spot, Some(Spot::new([-0.6, -1., -0.4], 0.2, 0.4)));
                world.add_light(Light::Hemisphere {
                    sky: [0.3, 0.35, 0.45],
                    ground: [0.1, 0.08, 0.06],
                    intensity: 1.,
                });
                vec![ball, cube, f]
            },
            shadows: false,
            tone: ToneMapping::Neutral,
            environment: || None,
            camera: camera([0., 1.9, 4.4], [0., 0.4, 0.]),
        },
        Scene {
            name: "shadow",
            tolerance: EDGES,
            build: |world| {
                let mut ids = vec![floor(world)];
                for (i, x) in [-1.2f32, 0., 1.2].into_iter().enumerate() {
                    let h = 0.5 + i as f32 * 0.4;
                    ids.push(world.add(
                        Object::mesh(Geometry::cuboid(0.5, h, 0.5), Material::rgb(0xe0e0e0)).at([
                            x,
                            h / 2.,
                            0.,
                        ]),
                    ));
                }
                ids.push(
                    world.add(
                        Object::mesh(Geometry::torus(0.35, 0.1, 32), Material::rgb(0xc08040))
                            .at([0., 1.6, 0.8]),
                    ),
                );
                sun(world, [-0.6, -1., -0.5], 3.);
                world.add_light(Light::Hemisphere {
                    sky: [0.25, 0.3, 0.4],
                    ground: [0.1, 0.1, 0.1],
                    intensity: 1.,
                });
                ids
            },
            shadows: true,
            tone: ToneMapping::Neutral,
            environment: || None,
            camera: camera([1.5, 2.8, 3.6], [0., 0.4, 0.]),
        },
        Scene {
            name: "glass",
            tolerance: EDGES,
            build: |world| {
                let back = world.add(
                    Object::mesh(Geometry::cuboid(3., 2., 0.1), Material::rgb(0xffffff))
                        .at([0., 1., -1.2]),
                );
                let stripes: Vec<ObjectId> = (0..4)
                    .map(|i| {
                        let colour = [0xd03030, 0x30a040, 0x3050d0, 0xe0c020][i];
                        world.add(
                            Object::mesh(Geometry::cuboid(0.25, 1.8, 0.12), Material::rgb(colour))
                                .at([-0.75 + i as f32 * 0.5, 1., -1.1]),
                        )
                    })
                    .collect();
                let ball = world.add(
                    Object::mesh(
                        Geometry::sphere(0.6, 48),
                        Material::rgb(0xffffff).with(0., 0.05),
                    )
                    .at([-0.45, 0.8, 0.2]),
                );
                world.set_transmission(
                    ball,
                    Some(
                        Transmission::new(1.)
                            .ior(1.5)
                            .volume(0.8, [0.6, 0.9, 1.], 1.),
                    ),
                );
                let pane = world.add(
                    Object::mesh(
                        Geometry::cuboid(0.7, 1.2, 0.05),
                        Material::rgb(0xffffff).with(0., 0.3),
                    )
                    .at([0.6, 0.8, 0.2]),
                );
                world.set_transmission(pane, Some(Transmission::new(0.9)));
                sun(world, [-0.3, -1., -0.6], 2.5);
                let mut ids = vec![back, ball, pane, floor(world)];
                ids.extend(stripes);
                ids
            },
            shadows: true,
            tone: ToneMapping::Neutral,
            environment: sky,
            camera: camera([0., 1.1, 2.8], [0., 0.8, 0.]),
        },
        Scene {
            name: "clearcoat",
            tolerance: CLOSE,
            build: |world| {
                let rough = world.add(
                    Object::mesh(
                        Geometry::sphere(0.55, 48),
                        Material::rgb(0xa01818).with(0., 0.9),
                    )
                    .at([-0.65, 0.6, 0.]),
                );
                world.set_clearcoat(rough, Some(Clearcoat::new(1.).roughness(0.05)));
                let half = world.add(
                    Object::mesh(
                        Geometry::sphere(0.55, 48),
                        Material::rgb(0x1840a0).with(0.3, 0.7),
                    )
                    .at([0.65, 0.6, 0.]),
                );
                world.set_clearcoat(half, Some(Clearcoat::new(0.5).roughness(0.4)));
                world.set_specular(half, Some(Specular::new(0.8).color([1., 0.9, 0.7])));
                sun(world, [-0.2, -0.6, -1.], 3.);
                vec![rough, half, floor(world)]
            },
            shadows: false,
            tone: ToneMapping::Neutral,
            environment: sky,
            camera: camera([0., 1.2, 3.4], [0., 0.55, 0.]),
        },
        Scene {
            name: "environment",
            tolerance: CLOSE,
            build: |world| {
                (0..4)
                    .map(|i| {
                        let roughness = i as f32 / 3.;
                        world.add(
                            Object::mesh(
                                Geometry::sphere(0.4, 48),
                                Material::rgb(0xe8c070).with(1. - (i % 2) as f32, roughness),
                            )
                            .at([-1.05 + i as f32 * 0.7, 0.5, 0.]),
                        )
                    })
                    .collect()
            },
            shadows: false,
            tone: ToneMapping::Neutral,
            environment: || Some(Environment::studio().rotated(0.6)),
            camera: camera([0., 1.1, 3.8], [0., 0.45, 0.]),
        },
        // One scene of bright and dim emitters, under each tone map.
        tone("tone_neutral", ToneMapping::Neutral),
        tone("tone_aces", ToneMapping::Aces),
        tone("tone_linear", ToneMapping::Linear),
    ];
    #[cfg(feature = "lighting")]
    scenes.push(clustered_lights());
    #[cfg(feature = "lighting")]
    scenes.push(cascaded_shadow());
    #[cfg(feature = "shaders")]
    scenes.push(materials());
    scenes
}

/// A row of posts receding from the camera, their shadows in three
/// cascades (the `lighting` feature) reaching 12 m.
#[cfg(feature = "lighting")]
fn cascaded_shadow() -> Scene {
    Scene {
        name: "shadow_cascades",
        tolerance: EDGES,
        build: |world| {
            let mut ids = vec![world.add(Object::mesh(
                Geometry::plane(30., 30.),
                Material::rgb(0x9a948a).with(0., 0.8),
            ))];
            for i in 0..8 {
                let z = 1.5 - i as f32 * 1.6;
                ids.push(world.add(
                    Object::mesh(Geometry::cuboid(0.25, 1.4, 0.25), Material::rgb(0xe0e0e0)).at([
                        0.6 * (i % 2) as f32 - 0.3,
                        0.7,
                        z,
                    ]),
                ));
            }
            sun(world, [-0.7, -1., 0.2], 3.);
            world.add_light(Light::Hemisphere {
                sky: [0.25, 0.3, 0.4],
                ground: [0.1, 0.1, 0.1],
                intensity: 1.,
            });
            world.set_cascades(Some(Cascades::new(3).distance(12.).size(512)));
            ids
        },
        shadows: true,
        tone: ToneMapping::Neutral,
        environment: || None,
        camera: camera([1.2, 1.6, 3.5], [-0.3, 0.4, -4.]),
    }
}

/// 48 point and spot lights over a floor and two meshes, shaded through
/// clusters (the `lighting` feature), a sun casting shadows.
#[cfg(feature = "lighting")]
fn clustered_lights() -> Scene {
    Scene {
        name: "lights_clustered",
        tolerance: EDGES,
        build: |world| {
            let ball = world.add(
                Object::mesh(
                    Geometry::sphere(0.5, 48),
                    Material::rgb(0xe0e0e0).with(0., 0.35),
                )
                .at([-0.7, 0.5, 0.]),
            );
            let cube = world.add(
                Object::mesh(
                    Geometry::cuboid(0.8, 0.8, 0.8),
                    Material::rgb(0xc0c0c0).with(0.9, 0.3),
                )
                .at([0.8, 0.4, -0.3]),
            );
            let f = floor(world);
            sun(world, [-0.4, -1., -0.6], 0.4);
            for i in 0..48 {
                let (x, z) = ((i % 8) as f32 * 0.8 - 2.8, (i / 8) as f32 * 0.9 - 2.4);
                let hue = i as f32 * 0.37;
                let light = world.add_light(Light::Point {
                    position: vec3(x, 0.25 + (i % 3) as f32 * 0.3, z),
                    color: [
                        0.5 + 0.5 * hue.cos(),
                        0.5 + 0.5 * (hue + 2.1).cos(),
                        0.5 + 0.5 * (hue + 4.2).cos(),
                    ],
                    intensity: 0.8,
                    range: 0.6 + (i % 5) as f32 * 0.2,
                });
                if i % 7 == 0 {
                    world.set_spot(light, Some(Spot::new([0.3, -1., 0.2], 0.2, 0.5)));
                }
            }
            vec![ball, cube, f]
        },
        shadows: true,
        tone: ToneMapping::Neutral,
        environment: || None,
        camera: camera([0., 2.6, 3.6], [0., 0.2, -0.3]),
    }
}

/// Custom materials (`shaders`): a sphere its shader tints and roughens by
/// noise, a plane its shader waves, a cut-out casting its holes' shadow,
/// and a surface layer darkening every lit mesh's lower half.
#[cfg(feature = "shaders")]
fn materials() -> Scene {
    Scene {
        name: "material",
        tolerance: EDGES,
        build: |world| {
            let shader = |source: &str| MaterialShader::new(format!("#zgx material 1\n{source}"));
            let noisy = world
                .add_shader(
                    shader(
                        "fn surface(s: SurfaceIn) -> Surface {
                             var m = surface_default(s);
                             m.base_color *= params.tint.rgb;
                             m.roughness = 0.15 + 0.7 * noise3(s.world_position * 6.0);
                             return m;
                         }",
                    )
                    .param("tint", Param::Color),
                )
                .unwrap();
            world.set_param(noisy, "tint", [0.4, 0.7, 1., 1.]).unwrap();
            let wave = world
                .add_shader(
                    shader(
                        "fn displace(v: VertexIn) -> Displaced {
                             var d: Displaced;
                             let h = params.height * sin(v.world.x * 8.0);
                             let slope = params.height * 8.0 * cos(v.world.x * 8.0);
                             d.position = v.position + v.normal * h;
                             d.normal = normalize(v.normal - vec3(slope, 0.0, 0.0));
                             return d;
                         }",
                    )
                    .param("height", Param::Float),
                )
                .unwrap();
            world.set_param(wave, "height", 0.08).unwrap();
            let holes = world
                .add_shader(shader(
                    "fn opacity(s: SurfaceIn) -> f32 {
                         let cell = fract(s.uv * 4.0) - 0.5;
                         return select(1.0, 0.0, dot(cell, cell) < 0.1);
                     }",
                ))
                .unwrap();
            let wet = world
                .add_surface_layer(shader(
                    "fn surface(s: SurfaceIn) -> Surface {
                         var m = surface_default(s);
                         let low = 1.0 - smoothstep(0.0, 0.3, s.world_position.y);
                         m.base_color *= 1.0 - 0.5 * low;
                         m.roughness = mix(m.roughness, 0.1, low);
                         return m;
                     }",
                ))
                .unwrap();
            let _ = wet;
            let ball = world.add(
                Object::mesh(
                    Geometry::sphere(0.5, 48),
                    Material::rgb(0xffffff).with(0., 0.5),
                )
                .at([-0.9, 0.55, 0.]),
            );
            world.set_shader(ball, Some(noisy));
            let sheet = world.add(
                Object::mesh(
                    Geometry::plane(1.2, 1.2),
                    Material::rgb(0xd0a060).with(0., 0.6),
                )
                .transformed(Transform {
                    position: vec3(0.6, 0.7, -0.3),
                    rotation: Quat::from_euler(PI / 2., 0., 0.),
                    ..Transform::default()
                }),
            );
            world.set_shader(sheet, Some(wave));
            let card = world.add(
                Object::mesh(
                    Geometry::cuboid(0.8, 0.8, 0.04),
                    Material::rgb(0x40a060).with(0., 0.5),
                )
                .at([0.2, 1.3, 0.6]),
            );
            world.set_shader(card, Some(holes));
            sun(world, [-0.3, -1., -0.4], 3.);
            world.add_light(Light::Hemisphere {
                sky: [0.3, 0.35, 0.45],
                ground: [0.1, 0.08, 0.06],
                intensity: 1.,
            });
            vec![ball, sheet, card, floor(world)]
        },
        shadows: true,
        tone: ToneMapping::Neutral,
        environment: || None,
        camera: camera([0., 1.6, 3.6], [0., 0.6, 0.]),
    }
}

fn tone(name: &'static str, tone: ToneMapping) -> Scene {
    Scene {
        name,
        tolerance: CLOSE,
        build: |world| {
            (0..5)
                .map(|i| {
                    let mut m =
                        Material::rgb([0x101010, 0x402010, 0x204060, 0x606060, 0x808080][i]);
                    let glow = [0.05, 0.3, 1., 3., 10.][i];
                    m.emissive = [glow, glow * 0.8, glow * 0.6];
                    world.add(Object::mesh(Geometry::sphere(0.3, 32), m).at([
                        -1.2 + i as f32 * 0.6,
                        0.4,
                        0.,
                    ]))
                })
                .chain([floor(world)])
                .collect()
        },
        shadows: false,
        tone,
        environment: || Some(Environment::studio()),
        camera: camera([0., 1.1, 3.7], [0., 0.35, 0.]),
    }
}

/// Where a scene's golden is, for a family.
fn golden(family: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/reference")
        .join(family)
        .join(format!("{name}.png"))
}

fn encode_png((w, h): (u32, u32), rgba: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(rgba).unwrap();
    writer.finish().unwrap();
    bytes
}
fn write_png(path: &Path, size: (u32, u32), rgba: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, encode_png(size, rgba)).unwrap();
}
/// A golden as a line of the log: `ZGX-GOLDEN <family>/<name>.png <base64>`.
fn golden_line(family: &str, name: &str, size: (u32, u32), rgba: &[u8]) -> String {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = encode_png(size, rgba);
    let mut text = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            text.push(if i <= chunk.len() {
                DIGITS[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    format!("ZGX-GOLDEN {family}/{name}.png {text}")
}
fn read_png(path: &Path) -> ((u32, u32), Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgba).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba, "{}", path.display());
    rgba.truncate(info.buffer_size());
    ((info.width, info.height), rgba)
}

/// Pixels differing past `level` in any channel, and the largest difference.
fn differ(a: &[u8], b: &[u8], level: u8) -> (usize, u8) {
    let mut count = 0;
    let mut largest = 0;
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let d = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        largest = largest.max(d);
        count += (d > level) as usize;
    }
    (count, largest)
}

/// Writes actual, expected and their difference (each channel's difference
/// times eight, opaque) where a failure can be looked at; their paths.
fn keep_failure(name: &str, size: (u32, u32), actual: &[u8], expected: &[u8]) -> String {
    let dir = std::env::temp_dir().join("zgx-3d-reference");
    let diff: Vec<u8> = actual
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected.as_chunks::<4>().0)
        .flat_map(|(p, q)| {
            let d = |i: usize| p[i].abs_diff(q[i]).saturating_mul(8);
            [d(0), d(1), d(2), 255]
        })
        .collect();
    let mut paths = Vec::new();
    for (kind, pixels) in [
        ("actual", actual),
        ("expected", expected),
        ("diff", &diff[..]),
    ] {
        let path = dir.join(format!("{name}-{kind}.png"));
        write_png(&path, size, pixels);
        paths.push(path.display().to_string());
    }
    paths.join("\n  ")
}

#[test]
fn each_scene_matches_its_golden() {
    let bless = std::env::var_os("ZGX_BLESS").is_some_and(|v| v == "1");
    let required = std::env::var_os("ZGX_REQUIRE_GPU").is_some();
    let mut failures = Vec::new();
    let mut lines = Vec::new();
    let mut missing = Vec::new();
    let mut family = None;
    for scene in scenes() {
        let (world, _) = scene.world();
        let Some(mut window) = Window::new(&world, (SIDE, SIDE)) else {
            return;
        };
        window.draw();
        let family = family.get_or_insert_with(|| window.family()).clone();
        let actual = window.pixels();
        let path = golden(&family, scene.name);
        if bless {
            write_png(&path, window.size, &actual);
            continue;
        }
        if !path.exists() {
            missing.push(path.display().to_string());
            lines.push(golden_line(&family, scene.name, window.size, &actual));
            continue;
        }
        let (size, expected) = read_png(&path);
        assert_eq!(size, window.size, "{} is another size", path.display());
        let (count, largest) = differ(&actual, &expected, scene.tolerance.level);
        let allowed = (scene.tolerance.fraction * (SIDE * SIDE) as f64) as usize;
        if count > allowed {
            failures.push(format!(
                "{} ({family}): {count} pixels differ past {} (allowed {allowed}), by up to {largest}:\n  {}",
                scene.name,
                scene.tolerance.level,
                keep_failure(scene.name, size, &actual, &expected)
            ));
            lines.push(golden_line(&family, scene.name, size, &actual));
        }
    }
    if !missing.is_empty() {
        let report = format!(
            "no golden for this adapter's family; bless with ZGX_BLESS=1:\n  {}",
            missing.join("\n  ")
        );
        if required || std::env::var_os("ZGX_REQUIRE_GOLDENS").is_some() {
            failures.push(report);
        } else {
            eprintln!("{report}");
        }
    }
    if required && !failures.is_empty() {
        failures.extend(lines);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A way a world reaches its state the long way round: changes made and
/// changed back (or on to a state `then` gives a fresh world), a frame drawn
/// after each step, so that what it leaves must draw as a world built that
/// way does. Every scene runs every sequence
/// (`a_retained_window_draws_what_a_fresh_one_does`): a kept structure adds
/// one entry, and is tried against every scene.
struct Sequence {
    name: &'static str,
    /// From the scene as built and settled, in a `SIDE` window drawn once.
    /// It must end with the window back at `SIDE`, and without `then` the
    /// scene's settings as they were (the test settles them again after).
    run: fn(&World, &mut Window, &Scene, &[ObjectId]),
    /// Where the sequence ends, if not at the scene as built: applied to the
    /// fresh world, built and settled, before it's drawn. With it, `run`'s
    /// end is compared as it left it, unsettled (a camera it turned stays
    /// turned).
    then: Option<fn(&World, &Scene, &[ObjectId])>,
    /// Frames drawn at the final state before comparing: what rests before
    /// looking again (culling) catches up within them.
    settle: usize,
    /// Compares what pixels can't show: the retained world and window,
    /// then a fresh world's and window's, each drawn `settle` more frames.
    check: Option<Check>,
}
/// "an object added"'s object, in front of every scene's camera target.
fn added(rgb: u32) -> Object {
    Object::mesh(Geometry::cuboid(0.4, 0.4, 0.4), Material::rgb(rgb)).at([0.3, 0.5, 0.6])
}
/// "masked caster"'s maps: opaque, and its left half clear; the placement
/// moving the map half across.
fn opaque_map() -> Texture {
    Texture::from_rgba8(2, 1, vec![255; 8]).unwrap()
}
fn half_map() -> Texture {
    Texture::from_rgba8(2, 1, vec![255, 255, 255, 0, 255, 255, 255, 255]).unwrap()
}
fn half_across() -> MapPlacement {
    MapPlacement::new().offset(0.5, 0.)
}
/// `camera` turned to look the other way, from where it stands.
fn turned_away(camera: Camera) -> Camera {
    let back = camera.position + (camera.position - camera.target);
    Camera::looking_at(camera.position, back)
}
/// A sequence's check: the retained world and window, then the fresh ones.
type Check = fn(&World, &mut Window, &World, &mut Window) -> Result<(), String>;

const SEQUENCES: &[Sequence] = &[
    Sequence {
        name: "sizes",
        run: |_, window, _, _| {
            window.resize((96, 72));
            window.resize((160, 120));
            window.resize((SIDE, SIDE));
        },
        then: None,
        settle: 0,
        check: None,
    },
    Sequence {
        name: "settings",
        run: |world, window, scene, _| {
            world.set_shadows(!scene.shadows);
            window.draw();
            world.set_tone_mapping(match scene.tone {
                ToneMapping::Aces => ToneMapping::Linear,
                _ => ToneMapping::Aces,
            });
            world.set_exposure(1.5);
            window.draw();
            world.set_environment(match (scene.environment)() {
                Some(_) => None,
                None => Some(Environment::studio()),
            });
            world.set_background(None);
            window.draw();
            let mut moved = scene.camera;
            moved.position.x += 0.7;
            moved.fov *= 0.8;
            world.set_camera(moved);
            world.set_samples(1);
            window.draw();
        },
        then: None,
        settle: 0,
        check: None,
    },
    Sequence {
        name: "looks",
        run: |world, window, scene, objects| {
            // Each object's look changed and changed back.
            let looks: Vec<_> = objects
                .iter()
                .map(|&id| {
                    let look = (
                        world.object(id).unwrap().material,
                        world.transmission(id),
                        world.clearcoat(id),
                    );
                    let mut green = Material::rgb(0x00ff00);
                    green.unlit = true; // shown whatever lights it
                    world.set_material(id, green);
                    world.set_transmission(id, None);
                    world.set_clearcoat(id, Some(Clearcoat::new(1.)));
                    look
                })
                .collect();
            window.draw();
            // The control: the changes showed (green, however exposed).
            let changed = window.pixels();
            let greens = changed
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[1] > p[0].saturating_add(30) && p[1] > p[2].saturating_add(30))
                .count();
            assert!(
                greens > 100,
                "{}: the green it was changed to never showed",
                scene.name
            );
            for (&id, (material, transmission, clearcoat)) in objects.iter().zip(looks) {
                world.set_material(id, material);
                world.set_transmission(id, transmission);
                world.set_clearcoat(id, clearcoat);
            }
        },
        then: None,
        settle: 0,
        check: None,
    },
    // Each object's custom material taken off and put back.
    #[cfg(feature = "shaders")]
    Sequence {
        name: "custom materials",
        run: |world, window, _, objects| {
            let shaders: Vec<_> = objects.iter().map(|&id| world.shader(id)).collect();
            for &id in objects {
                world.set_shader(id, None);
            }
            window.draw();
            for (&id, shader) in objects.iter().zip(shaders) {
                world.set_shader(id, shader);
            }
            window.draw();
        },
        then: None,
        settle: 0,
        check: None,
    },
    // Ends elsewhere: an object added to a drawn world, then its look
    // changed, against a world built with it as it ends.
    Sequence {
        name: "an object added",
        run: |world, window, _, _| {
            let added = world.add(added(0xff00ff));
            window.draw();
            world.set_material(added, Material::rgb(0x30c0a0).with(0., 0.4));
            window.draw();
        },
        then: Some(|world, _, _| {
            let added = world.add(added(0xff00ff));
            world.set_material(added, Material::rgb(0x30c0a0).with(0., 0.4));
        }),
        settle: 0,
        check: None,
    },
    Sequence {
        // A cut-out caster's map, placement and glass changed after it's
        // drawn, ending on a map only (what its shadow must follow).
        name: "masked caster",
        run: |world, window, _, _| {
            let card = world.add(added(0xffffff));
            world.set_alpha_cutoff(card, Some(0.5));
            world.set_map(card, Map::BaseColour, Some(opaque_map()));
            window.draw();
            world.set_map(card, Map::BaseColour, None);
            window.draw();
            world.set_transmission(card, Some(Transmission::new(1.)));
            window.draw();
            world.set_transmission(card, None);
            world.set_map_placement(card, Map::BaseColour, Some(half_across()));
            window.draw();
            world.set_map(card, Map::BaseColour, Some(half_map()));
            window.draw();
        },
        then: Some(|world, _, _| {
            let card = world.add(added(0xffffff));
            world.set_alpha_cutoff(card, Some(0.5));
            world.set_map_placement(card, Map::BaseColour, Some(half_across()));
            world.set_map(card, Map::BaseColour, Some(half_map()));
        }),
        settle: 0,
        check: None,
    },
    Sequence {
        // A mesh moved while culling rests, then the camera turned away:
        // culling looks again within a rest, as a fresh world culls at once.
        name: "culling at rest",
        run: |world, window, _, objects| {
            window.draw();
            if let Some(&first) = objects.first() {
                let place = world.object(first).unwrap().transform;
                let mut moved = place;
                moved.position.x += 0.01;
                world.set_transform(first, moved);
                window.draw();
                world.set_transform(first, place);
                window.draw();
            }
            // Turned away, frames past a rest (a nudge each: a frame only
            // draws as something changes), ending where the fresh one looks.
            let away = turned_away(world.camera());
            for step in 1..=17 {
                let mut nudged = away;
                nudged.position.y += step as f32 * 1e-4;
                world.set_camera(nudged);
                window.draw();
            }
            world.set_camera(away);
            window.draw();
        },
        then: Some(|world, _, _| world.set_camera(turned_away(world.camera()))),
        settle: 0,
        check: Some(|retained, _, fresh, _| {
            if retained.culled() == fresh.culled() {
                Ok(())
            } else {
                Err(format!(
                    "culled {} retained, {} fresh",
                    retained.culled(),
                    fresh.culled()
                ))
            }
        }),
    },
    Sequence {
        name: "an object come and gone",
        run: |world, window, _, _| {
            let extra = world.add(Object::mesh(
                Geometry::cuboid(1., 1., 1.),
                Material::rgb(0xff00ff),
            ));
            world.set_transmission(extra, Some(Transmission::new(1.)));
            window.draw();
            world.remove(extra);
            window.draw();
        },
        then: None,
        settle: 0,
        check: None,
    },
];

/// Apple7 GPUs (the M1 family): an exception to fresh-against-retained,
/// cause under investigation. On an M1 Max, each new GPU device drew a
/// scene without shadows with its silhouette edges as one of two images
/// (up to 44 pixels by 11 levels), kept for the device's life; the shader
/// text Metal compiled was byte-identical every time, and Metal's
/// validation reported nothing (lane/aaa-perf-fresh-variation, runs a-g).
/// Windows here share one device (`Window::new`), which leaves one scene
/// with a residue: `lighting`'s second window, 15 pixels by up to 3 in two
/// processes of three. The M5 Max and llvmpipe draw every image exactly.
/// So there, up to `APPLE7` (pixels, levels) may differ from the fresh
/// window, and the retained sequence must draw the same image twice.
const APPLE7: (usize, u8) = (16, 3);
fn apple7(window: &Window) -> bool {
    let info = window.gpu.adapter_info();
    info.backend == zgx_gpu::wgpu::Backend::Metal && info.name.starts_with("Apple M1")
}

#[test]
fn a_retained_window_draws_what_a_fresh_one_does() {
    let mut failures = Vec::new();
    for scene in scenes() {
        for sequence in SEQUENCES {
            let (fresh_world, fresh_objects) = scene.world();
            if let Some(then) = sequence.then {
                then(&fresh_world, &scene, &fresh_objects);
            }
            let Some(mut fresh) = Window::new(&fresh_world, (SIDE, SIDE)) else {
                return;
            };
            for _ in 0..=sequence.settle {
                fresh.draw();
            }
            let expected = fresh.pixels();
            let software = fresh.software();
            let apple7 = apple7(&fresh);

            // Another world, reaching the same state the long way round.
            let retained = || {
                let (world, objects) = scene.world();
                let mut window = Window::new(&world, (SIDE, SIDE)).unwrap();
                window.draw();
                (sequence.run)(&world, &mut window, &scene, &objects);
                if sequence.then.is_none() {
                    scene.settle(&world);
                }
                for _ in 0..=sequence.settle {
                    window.draw();
                }
                let pixels = window.pixels();
                (world, window, pixels)
            };
            let (world, mut window, actual) = retained();

            let level = if software { 0 } else { 1 };
            let (count, largest) = differ(&actual, &expected, level);
            let excepted = apple7 && count <= APPLE7.0 && largest <= APPLE7.1;
            if count > 0 && !excepted {
                failures.push(format!(
                    "{} after {}: {count} pixels differ past {level} from a fresh window's, by up to {largest}{}:\n  {}",
                    scene.name,
                    sequence.name,
                    if apple7 { " (past Apple7's exception)" } else { "" },
                    keep_failure(
                        &format!("{}-{}", scene.name, sequence.name.replace(' ', "-")),
                        (SIDE, SIDE),
                        &actual,
                        &expected
                    )
                ));
            }
            // Where a fresh image may vary, the retained one may not: the
            // same sequence again draws it exactly.
            if apple7 {
                let (_, _, again) = retained();
                let (count, largest) = differ(&again, &actual, 0);
                if count > 0 {
                    failures.push(format!(
                        "{} after {}: the same sequence again drew {count} pixels differently, by up to {largest}",
                        scene.name, sequence.name
                    ));
                }
            }
            if let Some(check) = sequence.check
                && let Err(why) = check(&world, &mut window, &fresh_world, &mut fresh)
            {
                failures.push(format!("{} after {}: {why}", scene.name, sequence.name));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_goldens_fit_their_budget() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference");
    let mut total = 0;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            total += std::fs::metadata(&path).unwrap().len();
            if path.extension().is_some_and(|e| e == "png") {
                let ((w, h), _) = read_png(&path);
                assert!(w <= 256 && h <= 256, "{} is {w}x{h}", path.display());
            }
        }
    }
    assert!(
        total <= BUDGET,
        "the goldens hold {total} bytes, past {BUDGET}"
    );
}
