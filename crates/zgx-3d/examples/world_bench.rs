//! The CPU work of 3D frames, for the gate: a generated world drawn through
//! wgpu's no-op backend, so its count holds zgx-3d and wgpu's validation, not
//! a driver's or a software rasteriser's. Each frame moves what a real one
//! does: the whole world, and skins' joints and morph weights.
//!
//! `SCENE=textured` is meshes with a base colour map and nothing else, a sun
//! casting shadows: a feature here costs what it costs meshes not using it.
//! `SCENE=mixed` is those among skinned, morphing, double-sided cut-out and
//! placed ones. `SCENE=still` is `textured`'s meshes standing still, the
//! camera going round them: a frame where only the view changed.
//! `SCENE=lights` is `textured` under 256 point lights, the frame choosing
//! which to shade (`lit`). `SCENE=anim` is `textured` with a quarter of its
//! meshes skinned to one 64-joint chain posed every frame (`rig`, `posed`).
//! With the `gltf` feature, `ANIMATOR` poses that chain by an animator
//! playing the same motion as a clip instead (`animated`): `clip`, one
//! layer; `layers`, a masked override and an additive layer over it;
//! `crowd`, each skinned mesh a chain of its own playing the clip from its
//! own time; `lockstep`, those all from the start (one pose, shared). Frames are then 1/60 s apart, whenever drawn.
//! `SCENE=shader` (with the `shaders` feature) is `textured`'s meshes, a
//! tenth drawn by one of four custom materials (two shading, one moving its
//! vertices, one cutting out), one shader's parameter set each frame.
//! `SCENE=probes` (with `--features gi`) is `textured` inside a volume of
//! 8 by 4 by 8 irradiance probes on a device tracing rays: each frame the
//! world turns, its ray scene is built again and 64 probes are updated.
//! `MESHES` and `FRAMES` size it; `CULLING=0` draws what the camera can't
//! see too; `CASCADES=n`, with the `lighting` feature, casts the sun's
//! shadow through n cascades (`still`: they follow the camera; `textured`:
//! their casters move). `SCENE=compute`, with the `compute` feature, adds
//! the app's GPU data and compute passes to `textured`
//! (`world_bench/compute.rs`), and `SCENE=particles`, with `particles`, rain
//! and sparks (`world_bench/particles.rs`). None has an environment:
//! prefiltering one takes two thirds of a cell, and its threads move the count.
//! Run: cargo run --release -p zgx-3d --example world_bench
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use zgx_3d::*;
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const SIZE: u32 = 256;
/// Geometries and textures the meshes share, as a model's would.
const SHAPES: usize = 8;
const TEXTURES: usize = 16;

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// A future of the no-op backend's, which never waits.
fn now<T>(future: impl Future<Output = T>) -> T {
    let mut context = Context::from_waker(Waker::noop());
    match pin!(future).poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the no-op backend waited"),
    }
}

// (Its scenes' modules' own: `compute`, `particles`.)
#[cfg_attr(not(any(feature = "compute", feature = "particles")), allow(dead_code))]
fn device() -> (wgpu::Device, wgpu::Queue) {
    device_with(false)
}

/// The no-op device, asking for ray queries where `ray_queries` (`probes`).
fn device_with(ray_queries: bool) -> (wgpu::Device, wgpu::Queue) {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::NOOP;
    descriptor.backend_options.noop.enable = true;
    descriptor.flags = wgpu::InstanceFlags::empty();
    let adapter =
        now(wgpu::Instance::new(descriptor)
            .request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("the no-op adapter");
    let descriptor = if ray_queries {
        wgpu::DeviceDescriptor {
            required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY,
            required_limits: wgpu::Limits::default()
                .using_minimum_supported_acceleration_structure_values(),
            // SAFETY: the no-op backend runs nothing.
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            ..Default::default()
        }
    } else {
        wgpu::DeviceDescriptor::default()
    };
    now(adapter.request_device(&descriptor)).expect("the no-op device")
}

/// A texture of `index`, its colour its own, clear in a corner.
fn texture(index: usize) -> Texture {
    let shade = (index * 16) as u8;
    let pixels = (0..16)
        .flat_map(|i| [shade, 255 - shade, 128, if i == 0 { 0 } else { 255 }])
        .collect();
    Texture::from_rgba8(4, 4, pixels).expect("its size")
}

/// Which each mesh is in `SCENE=mixed`, in turn.
#[derive(Clone, Copy)]
enum Kind {
    Textured,
    Skinned,
    Morphing,
    CutOut,
    Placed,
}
const MIXED: [Kind; 5] = [
    Kind::Textured,
    Kind::Skinned,
    Kind::Morphing,
    Kind::CutOut,
    Kind::Placed,
];

/// The world, and what its frames move: its root, the joint its skinned
/// meshes share, its morphing meshes, and its custom materials.
fn world(mixed: bool, meshes: usize) -> (World, ObjectId, ObjectId, Vec<ObjectId>) {
    let world = World::new();
    world.set_shadows(true);
    world.add_light(Light::Directional {
        direction: vec3(-0.5, -1., -0.4),
        color: [1., 0.97, 0.92],
        intensity: 2.5,
    });
    world.set_camera(Camera::looking_at([0., 40., 80.], [0., 0., 0.]));
    let root = world.add(Object::group());
    let joint = world.add(Object::group().child_of(root));
    let shapes: Vec<Geometry> = (0..SHAPES)
        .map(|i| Geometry::cuboid(1., 1. + i as f32 * 0.1, 1.))
        .collect();
    let skinned: Vec<Geometry> = shapes
        .iter()
        .map(|shape| {
            let vertices = shape.vertices().len();
            shape
                .clone()
                .with_skin(vec![[0; 4]; vertices], vec![[1., 0., 0., 0.]; vertices])
                .expect("an entry a vertex")
        })
        .collect();
    let morphing: Vec<Geometry> = shapes
        .iter()
        .map(|shape| {
            let up = vec![[0., 0.5, 0.]; shape.vertices().len()];
            shape
                .clone()
                .with_morph_targets(vec![MorphTarget::new(up)])
                .expect("an entry a vertex")
        })
        .collect();
    let textures: Vec<Texture> = (0..TEXTURES).map(texture).collect();
    #[cfg(feature = "shaders")]
    let mut all = Vec::with_capacity(meshes);
    let side = (meshes as f32).sqrt().ceil() as usize;
    let mut morphs = Vec::new();
    for i in 0..meshes {
        let kind = if mixed {
            MIXED[i % MIXED.len()]
        } else {
            Kind::Textured
        };
        let shape = i % SHAPES;
        let geometry = match kind {
            Kind::Skinned => skinned[shape].clone(),
            Kind::Morphing => morphing[shape].clone(),
            _ => shapes[shape].clone(),
        };
        let material = Material::rgb(0xffffff).with(0.1, 0.6);
        let mesh = world.add(Object::mesh(geometry, material).child_of(root));
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        let half = side as f32 / 2.;
        world.set_transform(mesh, Transform::at([(x - half) * 2., 0., (z - half) * 2.]));
        world.set_texture(mesh, Some(textures[i % TEXTURES].clone()));
        #[cfg(feature = "shaders")]
        all.push(mesh);
        match kind {
            Kind::Textured => {}
            Kind::Skinned => {
                let skin = Skin::new(vec![joint], vec![Mat4::IDENTITY]).expect("a joint");
                world.set_skin(mesh, Some(skin));
            }
            Kind::Morphing => morphs.push(mesh),
            Kind::CutOut => {
                world.set_alpha_cutoff(mesh, Some(0.5));
                world.set_double_sided(mesh, true);
            }
            Kind::Placed => {
                let tiled = MapPlacement::new().scaled(2., 2.).offset(0.25, 0.);
                world.set_map_placement(mesh, Map::BaseColour, Some(tiled));
            }
        }
    }
    #[cfg(feature = "shaders")]
    custom_shaders(&world, &all);
    (world, root, joint, morphs)
}

#[cfg(feature = "compute")]
#[path = "world_bench/compute.rs"]
mod compute;
#[cfg(feature = "gltf")]
#[path = "common/rig.rs"]
mod models;
#[cfg(feature = "particles")]
#[path = "world_bench/particles.rs"]
mod particles;

/// Whether this build runs `SCENE=scene`: one it doesn't, unknown or its
/// feature off, would count `textured` under another's name.
fn known(scene: &str) -> bool {
    // A table, not a match: with every feature on, a match of `true`s and
    // one `false` is clippy's `matches!`, and off it isn't.
    let featured = [
        ("shader", cfg!(feature = "shaders")),
        ("compute", cfg!(feature = "compute")),
        ("particles", cfg!(feature = "particles")),
        ("probes", cfg!(feature = "gi")),
    ];
    ["", "textured", "mixed", "still", "lights", "anim"].contains(&scene)
        || featured.iter().any(|&(name, on)| on && name == scene)
}

fn main() {
    let named = std::env::var("SCENE").unwrap_or_default();
    assert!(
        known(&named),
        "SCENE={named} isn't a scene of this build (unknown, or its feature off)"
    );
    #[cfg(feature = "compute")]
    if std::env::var("SCENE").is_ok_and(|scene| scene == "compute") {
        return compute::main();
    }
    #[cfg(feature = "particles")]
    if std::env::var("SCENE").is_ok_and(|scene| scene == "particles") {
        return particles::main();
    }
    let mixed = std::env::var("SCENE").is_ok_and(|scene| scene == "mixed");
    let (meshes, frames) = (setting("MESHES", 2000), setting("FRAMES", 30));
    let probes = std::env::var("SCENE").is_ok_and(|scene| scene == "probes");
    let (device, queue) = device_with(probes);
    let (world, root, joint, morphs) = world(mixed, meshes);
    let scene = std::env::var("SCENE").unwrap_or_default();
    if scene == "lights" {
        lit(&world);
    }
    let arm = (scene == "anim")
        .then(|| std::env::var("ANIMATOR").ok())
        .flatten()
        .filter(|arm| !arm.is_empty());
    let joints = if let Some(arm) = &arm {
        animated(&world, root, arm);
        Vec::new()
    } else if scene == "anim" {
        rig(&world, root)
    } else {
        Vec::new()
    };
    let start = arm.as_ref().map(|_| std::time::Instant::now());
    if probes {
        #[cfg(feature = "gi")]
        {
            let half = (meshes as f32).sqrt().ceil();
            world
                .add_probes(ProbeVolume::new(
                    [-half, -1., -half],
                    [half, 6., half],
                    [8, 4, 8],
                ))
                .expect("a volume");
        }
        #[cfg(not(feature = "gi"))]
        panic!("SCENE=probes needs --features gi");
    }
    let mut painter = world.viewport();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("A frame"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[FORMAT.add_srgb_suffix()],
    });
    let view = target.create_view(&Default::default());
    let srgb = target.create_view(&wgpu::TextureViewDescriptor {
        format: Some(FORMAT.add_srgb_suffix()),
        ..Default::default()
    });
    let still = std::env::var("SCENE").is_ok_and(|scene| scene == "still");
    // `CULLING=0`: every mesh drawn, to compare.
    world.set_culling(std::env::var("CULLING").map_or(true, |on| on != "0"));
    // `CASCADES=n`, with the `lighting` feature: the sun's shadow in n
    // cascades (of 2048, reaching 100 m) instead of one map, in any scene.
    #[cfg(feature = "lighting")]
    if let Some(count) = std::env::var("CASCADES").ok().and_then(|n| n.parse().ok()) {
        world.set_cascades(Some(Cascades::new(count)));
    }
    for frame in 0..frames {
        if still {
            orbit(&world, frame);
        } else {
            moved(&world, (root, joint, &morphs), frame);
        }
        posed(&world, &joints, frame);
        #[cfg(feature = "shaders")]
        shader_frame(&world, frame);
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut drawing = ViewportFrame::new(
            &device,
            &queue,
            &mut encoder,
            &view,
            FORMAT,
            (SIZE, SIZE),
            1.,
        );
        drawing.target_srgb = Some(&srgb);
        if let Some(start) = start {
            drawing.time = start + std::time::Duration::from_secs_f64(frame as f64 / 60.);
        }
        painter.render(&mut drawing);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::Poll).expect("the no-op device");
    }
    if arm.is_some() {
        // It did the work: its animators played through every frame.
        assert!(world.view().animating() && world.frames_drawn() == frames as u64);
    }
}

thread_local! {
    /// `SCENE=shader`'s shader whose parameter each frame sets.
    #[cfg(feature = "shaders")]
    static TINTED: std::cell::Cell<Option<ShaderId>> = const { std::cell::Cell::new(None) };
}
/// `SCENE=shader`'s four shaders (none in another scene), given to every
/// tenth of `meshes`.
#[cfg(feature = "shaders")]
fn custom_shaders(world: &World, meshes: &[ObjectId]) {
    if !std::env::var("SCENE").is_ok_and(|scene| scene == "shader") {
        return;
    }
    let shaders: Vec<ShaderId> = [
        "fn surface(s: SurfaceIn) -> Surface { var m = surface_default(s); m.base_color *= params.tint.rgb; return m; }",
        "fn surface(s: SurfaceIn) -> Surface { var m = surface_default(s); m.roughness = 0.2 + 0.6 * noise3(s.world_position); return m; }",
        "fn displace(v: VertexIn) -> Displaced { var d: Displaced; d.position = v.position * (1.0 + 0.1 * params.tint.x); d.normal = v.normal; return d; }",
        "fn opacity(s: SurfaceIn) -> f32 { return step(0.2, fract(s.uv.x * 4.0)); }",
    ]
    .into_iter()
    .map(|body| {
        let shader =
            MaterialShader::new(format!("#zgx material 1\n{body}")).param("tint", Param::Color);
        let id = world.add_shader(shader).expect("it compiles");
        world.set_param(id, "tint", [1., 0.8, 0.6, 1.]).expect("its parameter");
        id
    })
    .collect();
    for (i, &mesh) in meshes.iter().enumerate().step_by(10) {
        world.set_shader(mesh, Some(shaders[i / 10 % shaders.len()]));
    }
    TINTED.set(Some(shaders[0]));
}
/// What a frame of `SCENE=shader` changes: one shader's parameter.
#[cfg(feature = "shaders")]
fn shader_frame(world: &World, frame: usize) {
    if let Some(shader) = TINTED.get() {
        let tint = 0.5 + 0.5 * (frame as f32 * 0.1).sin();
        world
            .set_param(shader, "tint", [tint, 0.8, 0.6, 1.])
            .expect("its parameter");
    }
}

/// What a frame of `textured` or `mixed` moves: the whole world turns, the
/// skinned meshes' joint with it, and the morphing meshes' weights.
fn moved(world: &World, (root, joint, morphs): (ObjectId, ObjectId, &[ObjectId]), frame: usize) {
    let turned = frame as f32 * 0.01;
    world.set_transform(
        root,
        Transform {
            rotation: Quat::from_axis_angle(Vec3::Y, turned),
            ..Transform::default()
        },
    );
    world.set_transform(
        joint,
        Transform {
            rotation: Quat::from_axis_angle(vec3(1., 0., 0.), turned),
            ..Transform::default()
        },
    );
    for &mesh in morphs {
        world.set_morph_weights(mesh, &[0.5 + turned.sin() * 0.5]);
    }
}

/// `SCENE=lights`: 256 point lights over the meshes, each a colour of its
/// own; a frame shades the nearest the camera's target (`POINT_LIGHTS`).
fn lit(world: &World) {
    for i in 0..256u32 {
        let (x, z) = ((i % 16) as f32 * 8. - 60., (i / 16) as f32 * 8. - 60.);
        let warm = (i % 5) as f32 / 4.;
        world.add_light(Light::Point {
            position: vec3(x, 3., z),
            color: [1., 0.6 + 0.4 * (1. - warm), 0.4 + 0.6 * warm],
            intensity: 2.,
            range: 8.,
        });
    }
}

/// `SCENE=anim`: a chain of 64 joints under the root, and a quarter of the
/// meshes skinned to it, each vertex to one joint; `posed` turns every
/// joint each frame, so every skinned mesh's palette is made again.
fn rig(world: &World, root: ObjectId) -> Vec<ObjectId> {
    const JOINTS: usize = 64;
    let mut joints = Vec::with_capacity(JOINTS);
    let mut parent = root;
    for _ in 0..JOINTS {
        parent = world.add(Object::group().at([0., 0.1, 0.]).child_of(parent));
        joints.push(parent);
    }
    let shape = Geometry::cuboid(1., 1., 1.);
    let count = shape.vertices().len();
    let skinned = shape
        .with_skin(
            (0..count).map(|v| [(v % JOINTS) as u16, 0, 0, 0]).collect(),
            vec![[1., 0., 0., 0.]; count],
        )
        .expect("an entry a vertex");
    let skin = Skin::new(joints.clone(), vec![Mat4::IDENTITY; JOINTS]).expect("64 joints");
    let meshes = setting("MESHES", 2000) / 4;
    let side = (meshes as f32).sqrt().ceil() as usize;
    for i in 0..meshes {
        let at = [
            (i % side) as f32 * 2. - side as f32,
            4.,
            (i / side) as f32 * 2. - side as f32,
        ];
        let mesh = world.add(
            Object::mesh(skinned.clone(), Material::rgb(0xd0c0a0).with(0., 0.6))
                .at(at)
                .child_of(root),
        );
        world.set_skin(mesh, Some(skin.clone()));
    }
    joints
}

/// `SCENE=anim` with `ANIMATOR=arm`: `rig`'s chain and skinned meshes, the
/// chain a model's (`rig::chain`) posed by an animator playing its clip.
#[cfg(feature = "gltf")]
fn animated(world: &World, root: ObjectId, arm: &str) {
    const JOINTS: usize = 64;
    let model = Model::from_bytes(&models::chain(JOINTS)).expect("the chain");
    let clip = model.clip("Sway").expect("its clip").clone();
    let shape = Geometry::cuboid(1., 1., 1.);
    let count = shape.vertices().len();
    let skinned = shape
        .with_skin(
            (0..count).map(|v| [(v % JOINTS) as u16, 0, 0, 0]).collect(),
            vec![[1., 0., 0., 0.]; count],
        )
        .expect("an entry a vertex");
    let chain = |from: f32| {
        let placed = world.place_model(&model);
        world.update(placed.root, |o| o.parent = Some(root));
        let joints = (0..JOINTS)
            .map(|i| placed.node(i).expect("a joint"))
            .collect();
        let skin = Skin::new(joints, vec![Mat4::IDENTITY; JOINTS]).expect("64 joints");
        let animator = world.add_animator(&placed);
        world
            .play(animator, &clip, Play::new().from(from))
            .expect("a layer it has");
        (placed, animator, skin)
    };
    let meshes = setting("MESHES", 2000) / 4;
    let side = (meshes as f32).sqrt().ceil() as usize;
    let mesh = |i: usize, skin: &Skin| {
        let at = [
            (i % side) as f32 * 2. - side as f32,
            4.,
            (i / side) as f32 * 2. - side as f32,
        ];
        let mesh = world.add(
            Object::mesh(skinned.clone(), Material::rgb(0xd0c0a0).with(0., 0.6))
                .at(at)
                .child_of(root),
        );
        world.set_skin(mesh, Some(skin.clone()));
    };
    match arm {
        "clip" | "layers" => {
            let (placed, animator, skin) = chain(0.);
            if arm == "layers" {
                let upper = Mask::new().below(placed.node(JOINTS / 2).expect("a joint"), 1.);
                world
                    .set_layer(animator, 1, Layer::new().weight(0.5).mask(upper))
                    .expect("a layer it has");
                world
                    .play(animator, &clip, Play::new().layer(1).from(1.))
                    .expect("a layer it has");
                world
                    .set_layer(animator, 2, Layer::additive().weight(0.3))
                    .expect("a layer it has");
                world
                    .play(animator, &clip, Play::new().layer(2).from(0.5))
                    .expect("a layer it has");
            }
            for i in 0..meshes {
                mesh(i, &skin);
            }
        }
        "crowd" | "lockstep" => {
            for i in 0..meshes {
                // `lockstep`: all from the start, posing one pose.
                let from = if arm == "crowd" { i as f32 * 0.37 } else { 0. };
                let (_, _, skin) = chain(from);
                mesh(i, &skin);
            }
        }
        _ => panic!("ANIMATOR is clip, layers, crowd or lockstep"),
    }
}
#[cfg(not(feature = "gltf"))]
fn animated(_: &World, _: ObjectId, _: &str) {
    panic!("ANIMATOR needs the gltf feature");
}

/// What a frame of `anim` moves: every joint of the chain, a little.
fn posed(world: &World, joints: &[ObjectId], frame: usize) {
    for (i, &joint) in joints.iter().enumerate() {
        let angle = (frame as f32 * 0.05 + i as f32 * 0.1).sin() * 0.05;
        world.set_transform(
            joint,
            Transform {
                position: vec3(0., 0.1, 0.),
                rotation: Quat::from_axis_angle(vec3(0., 0., 1.), angle),
                ..Transform::default()
            },
        );
    }
}

/// What a frame of `still` moves: the camera, a step round the meshes.
fn orbit(world: &World, frame: usize) {
    let turned = frame as f32 * 0.01;
    let (sin, cos) = turned.sin_cos();
    world.set_camera(Camera::looking_at(
        [80. * sin, 40., 80. * cos],
        [0., 0., 0.],
    ));
}
