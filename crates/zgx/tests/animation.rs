use std::{cell::RefCell, num::NonZeroU32, rc::Rc, sync::Arc, time::Duration};
use zgx::{
    animation::{Animation, Frame, LoopCount},
    compose::{TaskRunner, prelude::*},
    image::ImageData,
    scene::{Effects, Transform},
    task::LocalExecutor,
    widgets::Ui,
};
fn animation(loops: LoopCount) -> Arc<Animation> {
    Arc::new(
        Animation::new(
            vec![
                Frame {
                    image: Arc::new(ImageData::new(1, 1, vec![255, 0, 0, 255]).unwrap()),
                    duration: Duration::from_millis(20),
                },
                Frame {
                    image: Arc::new(ImageData::new(1, 1, vec![0, 255, 0, 255]).unwrap()),
                    duration: Duration::from_millis(30),
                },
            ],
            loops,
        )
        .unwrap(),
    )
}
#[test]
fn timeline_skips_delayed_frames_and_retains_last_finite_frame() {
    let data = animation(LoopCount::Finite(NonZeroU32::new(2).unwrap()));
    assert_eq!(
        data.at(Duration::ZERO),
        (0, Some(Duration::from_millis(20)))
    );
    assert_eq!(
        data.at(Duration::from_millis(20)),
        (1, Some(Duration::from_millis(30)))
    );
    assert_eq!(
        data.at(Duration::from_millis(50)),
        (0, Some(Duration::from_millis(20)))
    );
    assert_eq!(
        data.at(Duration::from_millis(99)),
        (1, Some(Duration::from_millis(1)))
    );
    assert_eq!(data.at(Duration::from_millis(100)), (1, None));
    assert_eq!(
        animation(LoopCount::Infinite).at(Duration::from_secs(100000)),
        (0, Some(Duration::from_millis(20)))
    );
}
#[test]
fn hidden_offscreen_paused_and_removed_players_cancel_owned_tasks() {
    let mut ui = Ui::new(100., 100.);
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let playing = ui.signal(true);
    let handle = ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        animated_image_controlled("GIF", animation(LoopCount::Infinite), playing.clone())
            .id("animation")
            .size(40., 40.),
    ));
    ui.prepare_frame();
    executor.borrow_mut().tick();
    assert!(!executor.borrow().is_empty());
    ui.set_presented(false);
    executor.borrow_mut().tick();
    assert!(executor.borrow().is_empty());
    ui.set_presented(true);
    executor.borrow_mut().tick();
    assert!(!executor.borrow().is_empty());
    let node = handle.find("animation").unwrap();
    ui.scene
        .borrow_mut()
        .set_transform(node, Transform { x: 200., y: 0. });
    ui.prepare_frame();
    executor.borrow_mut().tick();
    assert!(executor.borrow().is_empty());
    ui.scene
        .borrow_mut()
        .set_transform(node, Transform::default());
    ui.prepare_frame();
    executor.borrow_mut().tick();
    assert!(!executor.borrow().is_empty());
    ui.scene.borrow_mut().set_effects(
        node,
        Effects {
            opacity: 0.,
            ..Default::default()
        },
    );
    ui.prepare_frame();
    executor.borrow_mut().tick();
    assert!(executor.borrow().is_empty());
    ui.scene.borrow_mut().set_effects(node, Effects::default());
    ui.prepare_frame();
    executor.borrow_mut().tick();
    assert!(!executor.borrow().is_empty());
    playing.set(false);
    executor.borrow_mut().tick();
    assert!(executor.borrow().is_empty());
    playing.set(true);
    executor.borrow_mut().tick();
    assert!(!executor.borrow().is_empty());
    handle.unmount();
    executor.borrow_mut().tick();
    assert!(executor.borrow().is_empty());
}

/// The red pixel's first channel of the picture `view`'s image node shows.
fn shown(ui: &Ui, view: zgx::scene::NodeId) -> Option<u8> {
    let scene = ui.scene.borrow();
    let mut stack = vec![view];
    while let Some(node) = stack.pop() {
        if let zgx::scene::NodeKind::Image(image) = scene.kind(node) {
            return image.pixels().first().copied();
        }
        stack.extend(scene.children(node).iter().copied());
    }
    None
}

/// An image view whose source gives an animation's still
/// (`ImageData::animated`, a GIF's) plays it, as `animated_image` does: its
/// playback set up once the first one comes, nothing for a still picture.
/// Another source changes what plays, and a still stops it.
#[test]
fn an_image_view_plays_the_animation_its_source_gives() {
    let mut ui = Ui::new(100., 100.);
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let still = Arc::new(ImageData::new(1, 1, vec![9, 9, 9, 255]).unwrap());
    let gif = Arc::new(ImageData::animated(animation(LoopCount::Infinite)));
    assert!(gif.animation().is_some());
    let source = ui.signal(still.clone());
    let read = source.clone();
    let handle = ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        image_signal("picture", move || read.get())
            .id("picture")
            .size(40., 40.),
    ));
    let node = handle.find("picture").unwrap();
    let settle = |ui: &mut Ui| {
        for _ in 0..3 {
            ui.prepare_frame();
            executor.borrow_mut().tick();
        }
    };
    settle(&mut ui);
    assert_eq!(shown(&ui, node), Some(9));
    assert!(executor.borrow().is_empty(), "a still plays nothing");
    // Its frames, red then green, each after its time.
    source.set(gif.clone());
    settle(&mut ui);
    assert_eq!(shown(&ui, node), Some(255), "the first frame");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while shown(&ui, node) != Some(0) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
        settle(&mut ui);
    }
    assert_eq!(shown(&ui, node), Some(0), "the second frame");
    // A still again: it stops, showing the still.
    source.set(still.clone());
    settle(&mut ui);
    assert_eq!(shown(&ui, node), Some(9));
    std::thread::sleep(Duration::from_millis(60));
    settle(&mut ui);
    assert_eq!(shown(&ui, node), Some(9), "nothing plays over it");
    assert!(executor.borrow().is_empty());
}

/// A GIF's view plays only while it is seen: scrolled off, clipped away,
/// faded out or in a window not presented (hidden, occluded), it arms no
/// timer and shows no frame, so it wakes nothing. Shown again, it plays.
/// With the system's reduce motion it shows its first frame, still.
#[test]
fn a_gif_unseen_or_with_reduced_motion_wakes_nothing() {
    let mut ui = Ui::new(100., 100.);
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let gif = Arc::new(ImageData::animated(animation(LoopCount::Infinite)));
    let held = Arc::downgrade(gif.animation().unwrap());
    let source = ui.signal(gif.clone());
    let read = source.clone();
    // In a 40-point clip, so it can be moved out of what shows.
    let handle = ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        div().size(40., 40.).overflow_hidden().child(
            image_signal("picture", move || read.get())
                .id("picture")
                .size(40., 40.),
        ),
    ));
    drop(gif);
    let node = handle.find("picture").unwrap();
    let settle = |ui: &mut Ui| {
        for _ in 0..3 {
            ui.prepare_frame();
            executor.borrow_mut().tick();
        }
    };
    settle(&mut ui);
    assert!(!executor.borrow().is_empty(), "it plays");
    // Frames shown over `wait` (each frame is 20 or 30 ms).
    let frames_over = |ui: &mut Ui, wait: Duration| {
        let mut seen = vec![shown(ui, node)];
        let until = std::time::Instant::now() + wait;
        while std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
            settle(ui);
            let now = shown(ui, node);
            if seen.last() != Some(&now) {
                seen.push(now);
            }
        }
        seen.len() - 1
    };
    // Whether a frame comes within 2 s (a slow machine's frames come late).
    let plays = |ui: &mut Ui| {
        let before = shown(ui, node);
        let until = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
            settle(ui);
            if shown(ui, node) != before {
                return true;
            }
        }
        false
    };
    assert!(plays(&mut ui));
    type Hide = fn(&mut Ui, zgx::scene::NodeId);
    type Show = fn(&mut Ui, zgx::scene::NodeId);
    let cases: [(&str, Hide, Show); 4] = [
        (
            "scrolled off",
            |ui, node| {
                ui.scene
                    .borrow_mut()
                    .set_transform(node, Transform { x: 300., y: 0. })
            },
            |ui, node| {
                ui.scene
                    .borrow_mut()
                    .set_transform(node, Transform::default())
            },
        ),
        (
            "clipped away",
            |ui, node| {
                ui.scene
                    .borrow_mut()
                    .set_transform(node, Transform { x: 60., y: 0. })
            },
            |ui, node| {
                ui.scene
                    .borrow_mut()
                    .set_transform(node, Transform::default())
            },
        ),
        (
            "faded out",
            |ui, node| {
                ui.scene.borrow_mut().set_effects(
                    node,
                    Effects {
                        opacity: 0.,
                        ..Default::default()
                    },
                )
            },
            |ui, node| ui.scene.borrow_mut().set_effects(node, Effects::default()),
        ),
        (
            "window not presented",
            |ui, _| ui.set_presented(false),
            |ui, _| ui.set_presented(true),
        ),
    ];
    for (what, hide, show) in cases {
        hide(&mut ui, node);
        settle(&mut ui);
        assert!(executor.borrow().is_empty(), "{what}: no timer armed");
        assert_eq!(
            frames_over(&mut ui, Duration::from_millis(80)),
            0,
            "{what}: no frame"
        );
        show(&mut ui, node);
        settle(&mut ui);
        assert!(
            !executor.borrow().is_empty(),
            "{what}: shown, it plays again"
        );
    }
    // Reduce motion: the first frame, still; then it plays again.
    let themes = zgx::theme::Themes::of(&ui.runtime);
    themes.set_system_preferences(zgx::theme::SystemPreferences {
        reduced_motion: true,
        ..Default::default()
    });
    settle(&mut ui);
    assert_eq!(shown(&ui, node), Some(255), "its first frame");
    assert!(executor.borrow().is_empty(), "reduce motion: no timer");
    assert_eq!(frames_over(&mut ui, Duration::from_millis(80)), 0);
    themes.set_system_preferences(Default::default());
    settle(&mut ui);
    assert!(plays(&mut ui), "it plays again");
    // Unmounted, with its source let go, nothing holds the animation.
    handle.unmount();
    settle(&mut ui);
    drop(source);
    settle(&mut ui);
    assert!(executor.borrow().is_empty());
    assert!(held.upgrade().is_none(), "its frames are let go");
}

/// A frame of a GIF repaints the picture alone: the frame's damage is its
/// box.
#[test]
fn a_gif_frame_damages_its_picture_alone() {
    let mut ui = Ui::new(200., 200.);
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let gif = Arc::new(ImageData::animated(animation(LoopCount::Infinite)));
    let handle = ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        column()
            .size(200., 200.)
            .child(div().size(200., 50.).bg(rgb(0x334455)))
            .child(
                image_signal("picture", move || gif.clone())
                    .id("picture")
                    .size(40., 40.),
            ),
    ));
    let node = handle.find("picture").unwrap();
    for _ in 0..3 {
        ui.prepare_frame();
        executor.borrow_mut().tick();
        ui.scene.borrow_mut().flush();
    }
    let bounds = ui.scene.borrow().bounds(node);
    let before = shown(&ui, node);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let damage = loop {
        std::thread::sleep(Duration::from_millis(5));
        executor.borrow_mut().tick();
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        if shown(&ui, node) != before {
            break damage;
        }
        assert!(damage.is_empty(), "nothing between frames: {damage:?}");
        assert!(std::time::Instant::now() < deadline, "a frame came");
    };
    assert!(!damage.is_empty());
    for rect in &damage {
        let inside = rect.x >= bounds.x - 0.5
            && rect.y >= bounds.y - 0.5
            && rect.x + rect.width <= bounds.x + bounds.width + 0.5
            && rect.y + rect.height <= bounds.y + bounds.height + 0.5;
        assert!(inside, "{rect:?} past the picture's {bounds:?}");
    }
}
