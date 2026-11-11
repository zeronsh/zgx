//! Renderers on threads of one process, each on a context of its own, made,
//! drawn with and dropped while the others draw: every one draws its own
//! pixels. On WARP (Direct3D 12 on the CPU, 10.0.26100) a device per thread
//! faulted in the driver or drew nothing at all; there they share one.
use std::sync::Arc;
use zgx::{
    canvas::*,
    scene::{Color, Layout, NodeKind, Rect, Scene, Style},
};
use zgx_gpu::GpuRenderer;

const THREADS: u8 = 8;
const ROUNDS: u8 = 4;
const FRAMES: u8 = 6;
const WIDTH: u32 = 200;
const HEIGHT: u32 = 150;

/// A chart's filled area and a dashed curve (drawn as GPU areas and
/// polylines, or rastered), over a solid band of `solid`, beside `color`.
fn scene(color: Color, solid: Color, frame: u8) -> Scene {
    let mut canvas = Canvas::new();
    canvas.fill(Path::rectangle(Rect::new(0., 0., 140., 40.)), solid);
    let mut area = Path::builder();
    area.move_to(0., 140.);
    for k in 0..120u8 {
        let t = f32::from(k);
        let y = 90. + 20. * (t * 0.31 + f32::from(frame)).sin() + 8. * (t * 1.7).cos();
        area.line_to(t * 140. / 119., y);
    }
    area.line_to(140., 140.);
    area.close();
    canvas.fill(
        area.build().unwrap(),
        Brush::linear(
            Point::new(0., 60.),
            Point::new(0., 140.),
            vec![
                GradientStop {
                    offset: 0.,
                    color: Color(58, 139, 255, 0x90),
                },
                GradientStop {
                    offset: 1.,
                    color: Color(58, 139, 255, 0),
                },
            ],
        )
        .unwrap(),
    );
    let mut curve = Path::builder();
    curve.move_to(10., 70.).cubic_to(
        Point::new(40., 40.),
        Point::new(90., 120.),
        Point::new(130., 60.),
    );
    canvas.stroke(
        curve.build().unwrap(),
        Color(255, 255, 255, 255),
        Stroke {
            width: 3.,
            cap: LineCap::Round,
            dash: vec![6., 3.],
            ..Default::default()
        },
    );
    let mut scene = Scene::new(WIDTH as f32, HEIGHT as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let root = scene.root();
    scene.append(
        root,
        NodeKind::Rect(color),
        Style {
            width: Some(WIDTH as f32),
            height: Some(HEIGHT as f32),
            ..Default::default()
        },
    );
    scene.append(
        root,
        NodeKind::Canvas(Arc::new(canvas)),
        Style {
            width: Some(140.),
            height: Some(HEIGHT as f32),
            ..Default::default()
        },
    );
    scene
}

/// A colour of whole channels (no rounding or dither to allow for).
fn whole(bits: u8) -> Color {
    let channel = |bit: u8| if bits & bit != 0 { 255 } else { 0 };
    Color(channel(1), channel(2), channel(4), 255)
}

#[test]
fn renderers_made_and_drawn_at_once_on_threads_each_draw_their_pixels() {
    if GpuRenderer::new(8, 8).is_err() {
        eprintln!("no GPU here");
        return;
    }
    let threads: Vec<_> = (0..THREADS)
        .map(|thread| {
            std::thread::spawn(move || {
                let mut drawn = 0;
                for round in 0..ROUNDS {
                    let mut gpu = GpuRenderer::new(WIDTH, HEIGHT).unwrap();
                    let full = [Rect::new(0., 0., WIDTH as f32, HEIGHT as f32)];
                    for frame in 0..FRAMES {
                        let k = thread + round + frame;
                        let (color, solid) = (whole(k % 7 + 1), whole((k + 3) % 7 + 1));
                        let mut scene = scene(color, solid, frame);
                        scene.flush();
                        gpu.render(&scene, &full).unwrap();
                        let pixels = gpu.readback().unwrap();
                        let at = |x: u32, y: u32| {
                            let i = 4 * (y * WIDTH + x) as usize;
                            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
                        };
                        let name = gpu.adapter_info().name;
                        let place =
                            format!("thread {thread}, round {round}, frame {frame} ({name})");
                        for y in [0, HEIGHT / 2, HEIGHT - 1] {
                            for x in [150, WIDTH - 1] {
                                let expected = [color.0, color.1, color.2, 255];
                                assert_eq!(at(x, y), expected, "{place}, beside at {x},{y}");
                            }
                        }
                        for x in [2, 70, 137] {
                            let expected = [solid.0, solid.1, solid.2, 255];
                            assert_eq!(at(x, 20), expected, "{place}, the canvas at {x},20");
                        }
                        drawn += 1;
                    }
                }
                drawn
            })
        })
        .collect();
    let drawn: u32 = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .sum();
    assert_eq!(
        drawn,
        u32::from(THREADS) * u32::from(ROUNDS) * u32::from(FRAMES)
    );
}
