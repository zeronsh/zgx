//! Bevy 0.19.1's animation CPU cost on a glTF clip: N instances of a model
//! playing its first clip (ARM=play) against the same scene standing still
//! (ARM=still), rendered offscreen to an image each frame. Frame time is
//! a fixed 1/60 s. Prints instructions and CPU time over FRAMES frames.
#[path = "../../common/rusage.rs"]
mod rusage;

use bevy::{
    animation::{
        AnimationPlayer,
        graph::{AnimationGraph, AnimationGraphHandle, AnimationNodeIndex},
    },
    app::ScheduleRunnerPlugin,
    camera::RenderTarget,
    gltf::GltfAssetLabel,
    image::Image,
    prelude::*,
    render::{
        RenderPlugin,
        render_resource::{TextureFormat, TextureUsages},
        settings::{Backends, RenderCreation, WgpuSettings},
    },
    time::TimeUpdateStrategy,
    window::ExitCondition,
    world_serialization::{WorldAssetRoot, WorldInstanceReady},
};
use std::{path::Path, time::Duration};

#[derive(Resource)]
struct Setup {
    play: bool,
    graph: Handle<AnimationGraph>,
    node: AnimationNodeIndex,
    ready: usize,
}

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let model = std::env::var("MODEL").expect("MODEL");
    let model = Path::new(&model);
    let folder = model.parent().unwrap().to_string_lossy().into_owned();
    let file = model.file_name().unwrap().to_string_lossy().into_owned();
    let instances = setting("INSTANCES", 100);
    let frames = setting("FRAMES", 300);
    let play = std::env::var("ARM").map_or(true, |a| a == "play");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: folder,
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: Some(Backends::METAL),
                    ..default()
                })),
                ..default()
            }),
    )
    .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::ZERO))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1. / 60.,
    )));

    let image = {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let mut target = Image::new_target_texture(256, 256, TextureFormat::Bgra8UnormSrgb, None);
        target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        images.add(target)
    };
    let (graph, node) = {
        let assets = app.world().resource::<AssetServer>().clone();
        let clip = assets.load(GltfAssetLabel::Animation(0).from_asset(file.clone()));
        let (graph, node) = AnimationGraph::from_clip(clip);
        let mut graphs = app.world_mut().resource_mut::<Assets<AnimationGraph>>();
        (graphs.add(graph), node)
    };
    app.insert_resource(Setup {
        play,
        graph,
        node,
        ready: 0,
    });
    {
        let assets = app.world().resource::<AssetServer>().clone();
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(file.clone()));
        let world = app.world_mut();
        world.spawn((
            Camera3d::default(),
            RenderTarget::Image(image.into()),
            Transform::from_xyz(0., 2., 30.).looking_at(Vec3::new(0., 1., 0.), Vec3::Y),
        ));
        world.spawn((
            DirectionalLight::default(),
            Transform::from_xyz(1., 2., 1.).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let side = (instances as f32).sqrt().ceil() as usize;
        for i in 0..instances {
            let (x, z) = ((i % side) as f32, (i / side) as f32);
            world
                .spawn((
                    WorldAssetRoot(scene.clone()),
                    Transform::from_xyz(x * 1.5 - side as f32 * 0.75, 0., -z * 1.5),
                ))
                .observe(ready);
        }
    }
    app.finish();
    app.cleanup();
    // Load and warm up.
    let mut guard = 0;
    while app.world().resource::<Setup>().ready < instances {
        app.update();
        guard += 1;
        assert!(guard < 100_000, "never ready");
    }
    for _ in 0..60 {
        app.update();
    }
    let players = app
        .world_mut()
        .query::<&AnimationPlayer>()
        .iter(app.world())
        .count();
    let before = rusage::sample();
    for _ in 0..frames {
        app.update();
    }
    let after = rusage::sample();
    let playing = app
        .world_mut()
        .query::<&AnimationPlayer>()
        .iter(app.world())
        .filter(|p| !p.all_finished() && p.playing_animations().count() > 0)
        .count();
    println!(
        "bevy arm={} instances={instances} players={players} playing={playing} frames={frames} instructions/frame={:.0} cpu_us/frame={:.1}",
        if play { "play" } else { "still" },
        (after.instructions - before.instructions) as f64 / frames as f64,
        (after.cpu_ns - before.cpu_ns) / frames as f64 / 1e3,
    );
}

fn ready(
    event: On<WorldInstanceReady>,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
    mut setup: ResMut<Setup>,
    mut commands: Commands,
) {
    setup.ready += 1;
    for entity in children.iter_descendants(event.entity) {
        if let Ok(mut player) = players.get_mut(entity) {
            if setup.play {
                player.play(setup.node).repeat();
            }
            commands
                .entity(entity)
                .insert(AnimationGraphHandle(setup.graph.clone()));
        }
    }
}
