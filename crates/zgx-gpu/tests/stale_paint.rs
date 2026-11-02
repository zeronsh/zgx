//! A frame's damage covers every pixel that frame changes, frame after
//! frame, as nodes mount and go while a list moves: a node mounted under a
//! row in a frame where the list moved, and gone in a later one while it
//! still moves, leaves nothing where it was drawn (Zeron's reply hover lane,
//! hanging in empty space under the reply).
use zgx::{compose::prelude::*, reactive::Signal, scene::Rect, widgets::Ui};
use zgx_gpu::GpuRenderer;

const W: u32 = 240;
const H: u32 = 200;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A measured list of `rows`, scrolled to `offset`, whose row `lane` shows
/// a lane under its line, as a hovered reply shows its time and Copy.
fn list(ui: &Ui, rows: usize, offset: &Signal<f32>, lane: &Signal<Option<usize>>) -> View {
    let heights = VariableHeights::new(&ui.runtime, rows, 40.);
    let shown = lane.clone();
    measured_virtual_list(
        offset.clone(),
        heights,
        1,
        |i| i,
        move |_, i, _| {
            let shown = shown.clone();
            // Natural heights the estimate (40) doesn't know: the list
            // re-anchors as rows come into view and measure. Each paints
            // little of its room (a line of text, say): the rest is the
            // window's background, which nothing else repaints.
            column()
                .w_full()
                .child(
                    div()
                        .h(30. + (i % 4) as f32 * 17.)
                        .child(div().w(20. + (i % 5) as f32 * 9.).h(8.).bg(rgb(0x5080c0))),
                )
                .child(switch(
                    move || shown.get() == Some(i),
                    |on, _| match on {
                        true => div().w(60.).h(16.).bg(rgb(0xff2020)),
                        false => div(),
                    },
                ))
        },
    )
    .w_full()
    .h_full()
}

/// Every frame drawn whole, checked against the last: what changed lies in
/// the frame's damage, or where its scroll copies moved the last frame's
/// pixels. (Drawing the damage alone can hide a miss, as a renderer may
/// repaint more than it is asked to.)
struct Frames {
    whole: GpuRenderer,
    last: Option<Vec<u8>>,
}

impl Frames {
    fn new() -> Self {
        Self {
            whole: GpuRenderer::new(W, H).unwrap(),
            last: None,
        }
    }

    fn frame(&mut self, ui: &Ui, what: &str) {
        ui.try_prepare_frame().unwrap();
        let report = ui.scene.borrow_mut().flush();
        let scene = ui.scene.borrow();
        (self.whole)
            .render(&scene, &[Rect::new(0., 0., W as f32, H as f32)])
            .unwrap();
        let now = self.whole.readback().unwrap();
        let Some(mut last) = self.last.replace(now.clone()) else {
            return;
        };
        let at = |x: u32, y: u32| ((y * W + x) * 4) as usize;
        // A move between pixels can't be copied: the renderer repaints its
        // view.
        let mut damage = report.damage.clone();
        for scroll in scene.scroll_moves() {
            let clip = scroll.clip;
            if scroll.dx.fract() != 0. || scroll.dy.fract() != 0. {
                damage.push(clip);
                continue;
            }
            let (dx, dy) = (scroll.dx as i64, scroll.dy as i64);
            let inside = |x: i64, y: i64| {
                x as f32 >= clip.x
                    && ((x + 1) as f32) <= clip.x + clip.width
                    && y as f32 >= clip.y
                    && ((y + 1) as f32) <= clip.y + clip.height
            };
            let moved = last.clone();
            for y in 0..H as i64 {
                for x in 0..W as i64 {
                    if inside(x, y) && inside(x - dx, y - dy) {
                        let (to, from) =
                            (at(x as u32, y as u32), at((x - dx) as u32, (y - dy) as u32));
                        last[to..to + 4].copy_from_slice(&moved[from..from + 4]);
                    }
                }
            }
        }
        let damaged = |x: u32, y: u32| {
            let pixel = Rect::new(x as f32, y as f32, 1., 1.);
            damage.iter().any(|rect| rect.intersects(pixel))
        };
        let missed: Vec<(u32, u32)> = (0..H)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .filter(|&(x, y)| last[at(x, y)..at(x, y) + 4] != now[at(x, y)..at(x, y) + 4])
            .filter(|&(x, y)| !damaged(x, y))
            .collect();
        assert!(
            missed.is_empty(),
            "{what}: {} changed pixels lie outside the damage {:?}, from {:?} to {:?}",
            missed.len(),
            damage,
            missed.first(),
            missed.last()
        );
    }
}

#[test]
fn a_node_mounted_and_gone_while_a_list_moves_leaves_nothing() {
    mounted_and_gone_while_moving(false);
}

/// The same with the list a layer of its own (its own pixels, as a fade or
/// an isolated group makes it), which a move can't copy: it repaints what
/// its content paints, before and after the move.
#[test]
fn a_node_mounted_and_gone_while_a_layered_list_moves_leaves_nothing() {
    mounted_and_gone_while_moving(true);
}

fn mounted_and_gone_while_moving(layered: bool) {
    let _gpu = gpu_fixture();
    let mut ui = Ui::new(W as f32, H as f32);
    let offset = ui.signal(0.);
    let lane = ui.signal(None::<usize>);
    let view = list(&ui, 1_000, &offset, &lane);
    ui.mount(view);
    if layered {
        let mut scene = ui.scene.borrow_mut();
        let list = scene.children(scene.root())[0];
        scene.set_isolated(list, true);
    }
    let mut frames = Frames::new();
    frames.frame(&ui, "at rest");
    let mut at = 0.;
    for step in 0..12 {
        at += 23.;
        offset.set(at);
        // Mounted in a frame the list moved, gone two frames later: once
        // the move is laid out, as a hover under a still pointer finds it.
        ui.try_prepare_frame().unwrap();
        match step {
            3 => {
                lane.set(Some(4));
            }
            5 => {
                lane.set(None);
            }
            _ => {}
        }
        frames.frame(&ui, &format!("step {step}"));
    }
}

/// The same past a list's end, stretched by a trackpad (elastic scrolling):
/// the last row's lane mounts while the content is stretched and goes while
/// it springs back.
#[test]
fn a_node_mounted_and_gone_while_a_list_stretches_leaves_nothing() {
    use std::time::Duration;
    use zgx::input::{InputEvent, ScrollGesture};
    let _gpu = gpu_fixture();
    let mut ui = Ui::new(W as f32, H as f32);
    ui.set_elastic_scrolling(true);
    let offset = ui.signal(0.);
    let lane = ui.signal(None::<usize>);
    let view = list(&ui, 8, &offset, &lane);
    ui.mount(view);
    let wheel = |ui: &mut Ui, dy: f32| {
        ui.dispatch(InputEvent::Scroll {
            x: 100.,
            y: 100.,
            delta_x: 0.,
            delta_y: dy,
        });
    };
    let mut frames = Frames::new();
    frames.frame(&ui, "at rest");
    offset.set(f32::MAX);
    frames.frame(&ui, "at its end");
    ui.dispatch(InputEvent::ScrollGesture(ScrollGesture::Began));
    for step in 0..6 {
        wheel(&mut ui, 22.);
        ui.try_prepare_frame().unwrap();
        if step == 2 {
            lane.set(Some(7));
        }
        frames.frame(&ui, &format!("stretching {step}"));
    }
    ui.dispatch(InputEvent::ScrollGesture(ScrollGesture::Ended));
    let mut step = 0;
    while let Some(next) = ui.next_interaction_deadline() {
        ui.advance_interactions(next + Duration::from_millis(16))
            .unwrap();
        ui.try_prepare_frame().unwrap();
        if step == 2 {
            lane.set(None);
        }
        frames.frame(&ui, &format!("springing {step}"));
        step += 1;
        if step > 60 {
            break;
        }
    }
}
