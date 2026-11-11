//! A world rests once what kept it drawing is turned off: for every reason
//! a painter holds in its frame (`Reason` in world.rs), the feature is
//! turned on until the world draws by itself, turned off, and the world
//! must stop within a few frames (or, where work finishes off the frame, a
//! few seconds), however far along it was. Idle windows doing no work is a
//! merge rule; this is its check, one row a reason. A lane adding a reason
//! adds its row here; a row failing in code another lane owns stays in as
//! `#[ignore = "<finding>: <owner>"]` until that lane fixes it.
// (With none of the features, every row is out: what they share goes unused.)
#![cfg_attr(not(feature = "gltf"), allow(dead_code, unused_imports))]
use zgx_3d::*;

#[path = "common/settle.rs"]
mod harness;
use harness::{caught, rests, rests_caught};

#[cfg(feature = "gltf")]
#[path = "../examples/common/rig.rs"]
mod rig;

#[cfg(feature = "lighting")]
#[test]
fn cascades_turned_off_while_one_is_owed() {
    rests("cascades off", |world, _, window| {
        world.set_shadows(true);
        world.set_cascades(Some(Cascades::new(4).distance(40.)));
        window.draw(world);
        // A turn of the camera: the far cascades are owed a frame each.
        world.set_camera(Camera::looking_at([5., 5., 7.], [-2., 0., 1.]));
        Some(Box::new(|world| world.set_cascades(None)))
    });
}

#[cfg(feature = "lighting")]
#[test]
fn sun_shadows_turned_off_while_a_cascade_is_owed() {
    rests("shadows off", |world, _, window| {
        world.set_shadows(true);
        world.set_cascades(Some(Cascades::new(4).distance(40.)));
        window.draw(world);
        world.set_camera(Camera::looking_at([5., 5., 7.], [-2., 0., 1.]));
        Some(Box::new(|world| world.set_shadows(false)))
    });
}

#[cfg(feature = "shaders")]
#[test]
fn an_animated_shader_taken_off_its_mesh() {
    rests("animated shader", |world, mesh, _| {
        let pulse = world
            .add_shader(
                MaterialShader::new(
                    "#zgx material 1
                     fn surface(s: SurfaceIn) -> Surface {
                         var m = surface_default(s);
                         m.base_color *= 0.5 + 0.5 * sin(frame.time);
                         return m;
                     }",
                )
                .animated(),
            )
            .unwrap();
        world.set_shader(mesh, Some(pulse));
        Some(Box::new(move |world| world.set_shader(mesh, None)))
    });
}

#[cfg(feature = "compute")]
#[test]
fn a_compute_pass_due_every_frame_removed() {
    rests("compute", |world, _, _| {
        let count = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
        let pass = world
            .add_compute(
                Compute::new(
                    "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                     @compute @workgroup_size(1) fn main() { count[0] += 1u; }",
                )
                .buffer("count", &count, Access::ReadWrite)
                .every_frame(),
            )
            .unwrap();
        Some(Box::new(move |world| world.remove_compute(pass)))
    });
}

#[cfg(feature = "particles")]
#[test]
fn an_emitter_removed_while_its_particles_live() {
    rests("particles", |world, _, _| {
        let rain = world
            .add_emitter(
                Emitter::new(1000)
                    .spawn(Spawn::box_([-1., 1., -1.], [1., 1., 1.]).rate(500.))
                    .lifetime(0.3..0.4)
                    .velocity([0., -5., 0.], 0.)
                    .draw(Shape::Streak { length: 0.05 }, Blend::Additive),
            )
            .unwrap();
        Some(Box::new(move |world| world.remove_emitter(rain)))
    });
}

#[cfg(feature = "post")]
#[test]
fn an_animated_post_pass_removed() {
    rests("post pass", |world, _, _| {
        let pulse = world
            .add_post(
                PostPass::new(
                    "#zgx post 1
fn post(p: PostIn) -> vec4<f32> {
    return color(p.uv) * (1.0 + 0.5 * sin(frame.time));
}
",
                )
                .animated(),
            )
            .unwrap();
        Some(Box::new(move |world| world.remove_post(pulse)))
    });
}

#[cfg(feature = "post")]
#[test]
fn automatic_exposure_turned_off_while_it_adapts() {
    rests("exposure", |world, _, window| {
        world.set_post(Post {
            auto_exposure: Some(AutoExposure::default()),
            ..Default::default()
        });
        // Only where the device runs compute does exposure adapt.
        window.draw(world);
        window.draw(world);
        if !world.view().animating() {
            return None;
        }
        Some(Box::new(|world| world.set_post(Post::default())))
    });
}

#[cfg(feature = "post")]
#[test]
fn temporal_antialiasing_turned_to_multisampling_while_it_gathers() {
    rests("taa to msaa", |world, _, window| {
        world.set_antialiasing(Antialiasing::Taa);
        window.draw(world);
        Some(Box::new(|world| world.set_antialiasing(Antialiasing::Msaa)))
    });
}

#[cfg(feature = "post")]
#[test]
fn temporal_antialiasing_turned_off_while_it_gathers() {
    rests("taa to none", |world, _, window| {
        world.set_antialiasing(Antialiasing::Taa);
        window.draw(world);
        Some(Box::new(|world| world.set_antialiasing(Antialiasing::None)))
    });
}

#[cfg(feature = "gi")]
#[test]
fn probes_removed_while_they_converge() {
    rests("probes", |world, _, window| {
        let probes = world
            .add_probes(ProbeVolume::new([-2., 0.05, -2.], [2., 2., 2.], [6, 6, 6]).per_frame(8))
            .unwrap();
        window.draw(world);
        window.draw(world);
        // Traced, or drawn as faces where the device has no ray queries
        // (this file asks for none).
        if !matches!(
            world.probes_status(),
            ProbesStatus::Traced | ProbesStatus::Rasterised
        ) {
            return None;
        }
        Some(Box::new(move |world| world.remove_probes(probes)))
    });
}

// Removing the last animator lets its world's own hold go outside a frame,
// and each painter's in its next frame (`animation::frame` with no slots).
#[cfg(feature = "post")]
#[test]
fn a_post_pass_removed_while_it_compiles() {
    rests_caught("post compiling", |world, _, window| {
        let pass = world
            .add_post(
                PostPass::new(
                    "#zgx post 1
fn post(p: PostIn) -> vec4<f32> {
    return color(p.uv) * params.k;
}
",
                )
                .param("k", PostParam::Float),
            )
            .unwrap();
        if !caught(world, window) {
            return None;
        }
        Some(Box::new(move |world| world.remove_post(pass)))
    });
}

#[cfg(feature = "gltf")]
#[test]
fn a_mesh_removed_while_its_map_loads() {
    rests_caught("loading", |world, mesh, window| {
        // A 512-square map, made here.
        let side = 512u32;
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, side, side);
            encoder.set_color(png::ColorType::Rgba);
            let pixels: Vec<u8> = (0..side * side)
                .flat_map(|i| [(i % 251) as u8, (i % 241) as u8, 128, 255])
                .collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
        world.set_texture(mesh, Some(Texture::decode(&bytes).unwrap()));
        if !caught(world, window) {
            return None;
        }
        Some(Box::new(move |world| world.remove(mesh)))
    });
}

// Audio has no row: no World call detaches a world's engine or its bus
// (`set_audio` sets them for good), and its hold, an emitter or listener
// moving, needs a move every frame, which a row turning a feature off and
// drawing on can't give.

#[cfg(feature = "gltf")]
#[test]
fn the_last_animator_removed_while_it_plays() {
    rests("animation", |world, _, _| {
        let model = Model::from_bytes(&rig::rig(3)).unwrap();
        let clip = model.clip("Wave").unwrap().clone();
        let placed = world.place_model(&model);
        let animator = world.add_animator(&placed);
        world.play(animator, &clip, Play::new()).unwrap();
        Some(Box::new(move |world| world.remove_animator(animator)))
    });
}
