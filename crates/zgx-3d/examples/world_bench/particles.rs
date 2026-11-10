//! `SCENE=particles` (with the `particles` feature): `textured`'s world with
//! rain beside it, so a frame counts what emitters cost the CPU: 200,000
//! streaks at 250,000 a second and a spark emitter bursting every tenth
//! frame (`ON_CPU=1`: updated on the CPU, as tier B does). The pipelines
//! are made before the frames counted.
use super::*;

pub(super) fn main() {
    let (meshes, frames) = (setting("MESHES", 2000), setting("FRAMES", 30));
    let (device, queue) = device();
    let (world, root, joint, morphs) = world(false, meshes);
    world
        .add_emitter(
            Emitter::new(200_000)
                .spawn(Spawn::box_([-40., 30., -40.], [40., 32., 40.]).rate(250_000.))
                .lifetime(0.6..0.9)
                .velocity([0., -22., 0.], 1.5)
                .force(Force::Gravity)
                .force(Force::Wind([2., 0., 1.]))
                .force(Force::Drag(0.1))
                .collide(Collide::Plane {
                    height: 0.,
                    then: Response::Die,
                })
                .draw(Shape::Streak { length: 0.04 }, Blend::Alpha)
                .size(0.01..0.015)
                .color([0.7, 0.75, 0.8, 0.4]),
        )
        .expect("an emitter");
    let sparks = world
        .add_emitter(
            Emitter::new(4096)
                .spawn(Spawn::cone([0., 1., 0.], [0., 1., 0.], 0.6))
                .lifetime(0.3..0.8)
                .velocity([0., 6., 0.], 1.)
                .force(Force::Gravity)
                .collide(Collide::Plane {
                    height: 0.,
                    then: Response::Bounce {
                        keep: 0.4,
                        friction: 0.3,
                    },
                })
                .draw(Shape::Streak { length: 0.02 }, Blend::Additive)
                .color([4., 2., 0.6, 1.]),
        )
        .expect("an emitter");
    if std::env::var_os("ON_CPU").is_some() {
        world.set_particles_on_cpu(true);
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
    let start = std::time::Instant::now();
    let mut draw = |index: usize| {
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
        drawing.time = start + std::time::Duration::from_micros(index as u64 * 16_667);
        painter.render(&mut drawing);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::Poll).expect("the no-op device");
    };
    draw(0);
    while !world.particles_ready() {
        std::thread::sleep(std::time::Duration::from_millis(20));
        draw(0);
    }
    for frame in 0..frames {
        moved(&world, (root, joint, &morphs), frame);
        if frame % 10 == 0 {
            world.burst(sparks, 500);
        }
        draw(frame + 1);
    }
    assert!(world.view().animating(), "raining");
}
