//! Animators played by a world's frames: drawn by its painter at the
//! frames' times (wgpu's no-op backend: the CPU's side alone), so the same
//! times pose the same, and a world whose clips are done draws no more.
#![cfg(feature = "gltf")]
#[path = "../examples/common/rig.rs"]
mod rig;

use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
use zgx_3d::*;
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

fn now<T>(future: impl Future<Output = T>) -> T {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the no-op backend waited"),
    }
}

/// Draws a world's frames at the times asked, as its view would.
struct Painter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    view: wgpu::TextureView,
    painter: Box<dyn Viewport>,
    start: Instant,
}
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
impl Painter {
    fn new(world: &World) -> Self {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::NOOP;
        descriptor.backend_options.noop.enable = true;
        let adapter = now(wgpu::Instance::new(descriptor).request_adapter(&Default::default()));
        let (device, queue) = now(adapter.unwrap().request_device(&Default::default())).unwrap();
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        Self {
            device,
            queue,
            view,
            painter: Box::new(world.viewport()),
            start: Instant::now(),
        }
    }
    fn frame(&mut self, seconds: f64) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut frame = ViewportFrame::new(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.view,
            FORMAT,
            (64, 64),
            1.,
        );
        frame.time = self.start + Duration::from_secs_f64(seconds);
        self.painter.render(&mut frame);
        self.queue.submit([encoder.finish()]);
    }
}

/// Joint 0's turn about z, in radians.
fn turn(world: &World, placed: &Placed, model: &Model) -> f32 {
    let _ = model;
    let joint = placed.node(1).expect("joint 0");
    let q = world.object(joint).unwrap().transform.rotation;
    2. * q.z.atan2(q.w)
}

#[test]
fn frames_pose_a_playing_clip_at_their_time() {
    let model = Model::from_bytes(&rig::rig(3)).unwrap();
    let clip = model.clip("Wave").unwrap().clone();
    let world = World::new();
    let placed = world.place_model(&model);
    let animator = world.add_animator(&placed);
    world.play(animator, &clip, Play::new()).unwrap();
    assert!(
        world.view().animating(),
        "playing: the world draws by itself"
    );
    let mut painter = Painter::new(&world);
    painter.frame(5.);
    let first = turn(&world, &placed, &model);
    painter.frame(5. + 0.25);
    let later = turn(&world, &placed, &model);
    // Placed::pose at the same time agrees.
    let other = World::new();
    let theirs = other.place_model(&model);
    theirs.pose(&other, &clip, 0.);
    assert_eq!(first, turn(&other, &theirs, &model));
    theirs.pose(&other, &clip, 0.25);
    assert_eq!(later, turn(&other, &theirs, &model));
    assert_ne!(first, later);
}

#[test]
fn a_finished_clip_lets_the_world_rest() {
    let model = Model::from_bytes(&rig::rig(2)).unwrap();
    let clip = model.clip("Wave").unwrap().clone();
    let world = World::new();
    let placed = world.place_model(&model);
    let animator = world.add_animator(&placed);
    world
        .play(animator, &clip, Play::new().once().speed(4.))
        .unwrap();
    let mut painter = Painter::new(&world);
    let mut t = 0.;
    while world.view().animating() {
        painter.frame(t);
        t += 1. / 60.;
        assert!(t < 2., "a half-second clip at four times ends");
    }
    let (revision, drawn) = (world.revision(), world.frames_drawn());
    // Done: the next frame (one already asked for) changes nothing.
    painter.frame(t + 1.);
    assert_eq!(world.revision(), revision);
    assert_eq!(world.frames_drawn(), drawn + 1);
    assert!(!world.view().animating(), "nothing asks for another");
}

#[test]
fn the_same_frame_times_pose_the_same_every_time() {
    let model = Model::from_bytes(&rig::rig(16)).unwrap();
    let clip = model.clip("Wave").unwrap().clone();
    let times = [0., 0.016, 0.034, 0.05, 0.2, 0.21, 0.4, 0.62, 0.8, 1.9, 2.05];
    let run = || {
        let world = World::new();
        let placed = world.place_model(&model);
        let animator = world.add_animator(&placed);
        world
            .play(animator, &clip, Play::new().speed(1.25))
            .unwrap();
        let mut painter = Painter::new(&world);
        let mut bits = Vec::new();
        for (i, t) in times.into_iter().enumerate() {
            if i == 5 {
                world
                    .play(animator, &clip, Play::new().layer(1).from(0.7).fade(0.2))
                    .unwrap();
                world
                    .set_layer(animator, 1, Layer::new().weight(0.6))
                    .unwrap();
            }
            painter.frame(t);
            for node in 1..=16 {
                let t = world.object(placed.node(node).unwrap()).unwrap().transform;
                let r = t.rotation;
                bits.extend([t.position.x, t.position.y, r.z, r.w].map(f32::to_bits));
            }
        }
        bits
    };
    assert_eq!(run(), run());
}

#[test]
fn a_layer_turned_off_draws_no_frames() {
    let model = Model::from_bytes(&rig::rig(4)).unwrap();
    let clip = model.clip("Wave").unwrap().clone();
    let world = World::new();
    let placed = world.place_model(&model);
    let animator = world.add_animator(&placed);
    world.play(animator, &clip, Play::new()).unwrap();
    let mut painter = Painter::new(&world);
    for frame in 0..30 {
        painter.frame(frame as f64 / 60.);
    }
    world
        .set_layer(animator, 0, Layer::new().weight(0.))
        .unwrap();
    // The frame it asked for poses its rest; then it asks for none.
    painter.frame(0.5);
    let drawn = world.frames_drawn();
    for frame in 31..631 {
        if world.view().animating() {
            painter.frame(frame as f64 / 60.);
        }
    }
    assert_eq!(world.frames_drawn() - drawn, 0, "10 s, no frame");
}

#[test]
fn a_node_is_found_by_its_name() {
    let model = Model::from_bytes(&rig::rig(4)).unwrap();
    // Node 0 holds the mesh; joint i is node i + 1.
    assert_eq!(model.node("joint2"), Some(3));
    assert_eq!(model.node("nothing"), None);
    let world = World::new();
    let placed = world.place_model(&model);
    let hips = placed.node(model.node("joint0").unwrap()).unwrap();
    assert_eq!(world.object(hips).unwrap().parent, Some(placed.root));
}
