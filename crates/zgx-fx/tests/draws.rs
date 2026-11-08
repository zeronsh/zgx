//! Ready-made effects (`zgx_fx`), each drawn over a known backdrop: it
//! compiles, it draws, its frames follow from its time and seed alone, and at
//! rest, with no intensity or with motion reduced it holds still or draws
//! nothing. Their looks are for eyes (the `fx_sheet` example); what can be
//! pinned is here.
use std::sync::Arc;
use std::time::Instant;
use zgx::decoration::{Background, Decoration};
use zgx::scene::*;
use zgx_fx::*;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const SIZE: u32 = 240;
const BLUE: Color = Color(30, 60, 200, 255);
const ORANGE: Color = Color(230, 130, 30, 255);
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
/// Blue and orange stripes under a 120 by 90 box at (60, 80), holding a word.
fn setting(fx: &Fx) -> (Scene, NodeId) {
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    for i in 0..12 {
        let stripe = scene.append(
            root,
            NodeKind::Rect(if i % 2 == 0 { BLUE } else { ORANGE }),
            fixed(SIZE as f32, 20.),
        );
        scene.set_transform(
            stripe,
            Transform {
                x: 0.,
                y: i as f32 * 20.,
            },
        );
    }
    let quad = QuadStyle {
        radius: 12.,
        fill: Color(0, 0, 0, if fx.kind() == FxKind::Fill { 255 } else { 0 }),
        decoration: (fx.kind() == FxKind::Fill).then(|| {
            Arc::new(Decoration {
                background: Some(Background::Shader(fx.shader())),
                ..Default::default()
            })
        }),
        ..Default::default()
    };
    let id = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad,
        },
        fixed(120., 90.),
    );
    scene.set_transform(id, Transform { x: 60., y: 80. });
    if fx.kind() == FxKind::Layer {
        let word = scene.append(
            id,
            NodeKind::Rect(Color(250, 250, 250, 255)),
            fixed(40., 12.),
        );
        scene.set_transform(word, Transform { x: 40., y: 40. });
    }
    (scene, id)
}
fn drawn(gpu: &mut GpuRenderer, scene: &mut Scene) -> Vec<u8> {
    let frame = scene.flush();
    gpu.render(scene, &frame.damage).unwrap();
    gpu.readback().unwrap()
}
fn draw(gpu: &mut GpuRenderer, fx: &Fx, time: f32, reduced: bool, now: Instant) -> Vec<u8> {
    let (mut scene, id) = setting(fx);
    fx.apply(&mut scene, id, time, reduced, now);
    drawn(gpu, &mut scene)
}
fn plain(gpu: &mut GpuRenderer) -> Vec<u8> {
    let idle: Fx = Glow {
        intensity: 0.,
        ..Default::default()
    }
    .into();
    draw(gpu, &idle, 0., false, Instant::now())
}
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 2))
        .count()
}
/// One of each effect at its defaults, moving where it would not otherwise
/// show (a motion blur moving, a split with stripes to split).
fn each() -> Vec<(String, Fx)> {
    let mut all = catalog();
    all.retain(|(name, _)| name != "motion_blur");
    all.push((
        "motion_blur moving".into(),
        MotionBlur {
            dx: 20.,
            dy: 30.,
            ..Default::default()
        }
        .into(),
    ));
    all.retain(|(name, _)| name != "chroma_split");
    all.push((
        "chroma_split down".into(),
        ChromaSplit {
            dx: 0.,
            dy: 6.,
            ..Default::default()
        }
        .into(),
    ));
    all
}

#[test]
fn every_effect_compiles_as_the_renderer_compiles_it() {
    for (name, kind, source) in sources() {
        let checked = match kind {
            FxKind::Layer => zgx_gpu::check_layer_shader(source),
            FxKind::Fill => zgx_gpu::check_fill_shader(source),
        };
        assert_eq!(checked, Ok(()), "{name}");
    }
}

#[test]
fn every_effect_draws_and_every_frame_follows_from_its_time_and_seed() {
    let _gpu = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let base = plain(&mut gpu);
    let start = Instant::now();
    for (name, fx) in each() {
        // Start one-shots, and see them a moment on.
        fx.fire(60., 45.);
        let _ = fx.uniforms(0., false, start);
        let later = start + std::time::Duration::from_millis(250);
        let one = draw(&mut gpu, &fx, 1.31, false, later);
        // Grain over this content is 3 levels at most, faint by design.
        let least = if name.contains("grain") && !name.contains("gradient") {
            0
        } else {
            40
        };
        // Some are quiet between beats and sweeps: at one of these moments.
        let most = [0.1, 0.45, 0.8, 1.31]
            .map(|time| differing(&draw(&mut gpu, &fx, time, false, later), &base))
            .into_iter()
            .max();
        assert!(most >= Some(least), "{name}: draws {most:?} pixels");
        let again = draw(&mut gpu, &fx, 1.31, false, later);
        assert_eq!(one, again, "{name}: another run of the same moment differs");
        if fx.is_animated() && fx.shots().is_none() {
            let moved = [0.1, 0.45, 0.8, 5.77]
                .map(|time| differing(&one, &draw(&mut gpu, &fx, time, false, later)))
                .into_iter()
                .max();
            assert!(
                moved > Some(0),
                "{name} is animated: other times look the same"
            );
        }
    }
}

#[test]
fn seeds_give_other_frames() {
    let _gpu = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let now = Instant::now();
    let sparkles = |seed| {
        Fx::from(Sparkles {
            seed,
            count: 12.,
            ..Default::default()
        })
    };
    let a = draw(&mut gpu, &sparkles(1), 0.5, false, now);
    let same = draw(&mut gpu, &sparkles(1), 0.5, false, now);
    let b = draw(&mut gpu, &sparkles(2), 0.5, false, now);
    assert_eq!(a, same);
    assert!(differing(&a, &b) > 0);
}

#[test]
fn no_intensity_draws_nothing_and_makes_no_layer() {
    let _gpu = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let base = plain(&mut gpu);
    for fx in [
        Fx::from(Glow {
            intensity: 0.,
            ..Default::default()
        }),
        Fx::from(HeatHaze {
            intensity: 0.,
            ..Default::default()
        }),
        Fx::from(MotionBlur::default()),
        // Its frost is a blur: not that either.
        Fx::from(Lens {
            intensity: 0.,
            ..Lens::frosted()
        }),
        // A one-shot with none alive.
        Fx::from(Ripple::default()),
        Fx::from(Shockwave::default()),
    ] {
        assert!(fx.is_idle(), "{}", fx.name());
        let (mut scene, id) = setting(&fx);
        fx.apply(&mut scene, id, 1., false, Instant::now());
        assert!(!scene.is_layer(id), "{}", fx.name());
        assert_eq!(drawn(&mut gpu, &mut scene), base, "{}", fx.name());
    }
}

#[test]
fn with_motion_reduced_an_effect_holds_one_pose() {
    let _gpu = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let now = Instant::now();
    for (name, fx) in each() {
        if fx.shots().is_some() {
            continue;
        }
        let a = draw(&mut gpu, &fx, 0.4, true, now);
        let b = draw(&mut gpu, &fx, 3.9, true, now);
        assert_eq!(a, b, "{name} moves with motion reduced");
    }
}

#[test]
fn a_metaball_is_whole_at_its_centre() {
    // Its field climbs without bound there: an edge taken from its slope
    // left dark pixels around each blob's centre, the background mixed in.
    let _gpu = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE * 2, SIZE * 2).unwrap();
    gpu.set_scale_factor(2.);
    let fx: Fx = Metaballs::default().into();
    let side = (SIZE * 2) as usize;
    let mut specks = Vec::new();
    for time in [0.1, 0.7, 1.2, 2.9, 4.4] {
        let pixels = draw(&mut gpu, &fx, time, false, Instant::now());
        // The background is about 52 in all, a blob's body 400 or more.
        let level = |x: usize, y: usize| {
            let i = (y * side + x) * 4;
            pixels[i..i + 3].iter().map(|&c| c as i32).sum::<i32>()
        };
        let near = |x: usize, y: usize, r: usize| {
            (y - r..=y + r).flat_map(move |ny| (x - r..=x + r).map(move |nx| (nx, ny)))
        };
        // The box, clear of its rounded corners.
        for y in 184..316 {
            for x in 144..336 {
                // Well inside a blob, two pixels from any edge, it shades
                // smoothly: no pixel a pit, far darker than the neighbours on
                // both sides of it.
                if !near(x, y, 2).all(|(nx, ny)| level(nx, ny) > 200) {
                    continue;
                }
                let pit = [(1, 0), (0, 1), (1, 1), (1, -1)].iter().any(|&(dx, dy)| {
                    let side =
                        |s: i32| level((x as i32 + s * dx) as usize, (y as i32 + s * dy) as usize);
                    side(1).min(side(-1)) - level(x, y) > 45
                });
                if pit {
                    specks.push((time, x, y));
                }
            }
        }
    }
    assert!(specks.is_empty(), "dark specks inside blobs: {specks:?}");
}

#[test]
fn one_shots_overlap_within_their_pool_and_end() {
    let fx: Fx = Ripple::default().into();
    let start = Instant::now();
    let at = |ms: u64| start + std::time::Duration::from_millis(ms);
    assert!(fx.is_idle());
    fx.fire(1., 1.);
    let u = fx.uniforms(0., false, at(0));
    assert_eq!(u.params[1][..2], [1., 1.]);
    fx.fire(2., 2.);
    fx.fire(3., 3.);
    let u = fx.uniforms(0., false, at(100));
    assert!(
        (u.params[1][2] - 100. / 750.).abs() < 1e-3,
        "{:?}",
        u.params
    );
    assert_eq!(u.params[3][..2], [3., 3.]);
    assert_eq!(u.params[3][3], 1.);
    // A fourth: the oldest leaves in a moment, not cut.
    fx.fire(4., 4.);
    let now = Instant::now();
    let u = fx.uniforms(0., false, now + std::time::Duration::from_millis(40));
    assert!(
        u.params[1][3] > 0.2 && u.params[1][3] < 1.,
        "{:?}",
        u.params[1]
    );
    let u = fx.uniforms(0., false, now + std::time::Duration::from_millis(400));
    assert_eq!(u.params[3][..2], [4., 4.], "{:?}", u.params);
    // Past their lives: none alive, nothing to draw.
    let _ = fx.uniforms(0., false, now + std::time::Duration::from_secs(5));
    assert!(fx.is_idle());
}

#[test]
fn every_effect_can_be_built_by_name() {
    for name in NAMES {
        let fx = Fx::build(name, None, []).unwrap();
        assert_eq!(fx.name(), *name);
    }
    let neon = Fx::build(
        "glow",
        Some("neon"),
        [
            ("radius", FxValue::Number(30.)),
            ("style", FxValue::Text("aura")),
        ],
    )
    .unwrap();
    assert_eq!(neon.bleed(), 75.);
    assert!(
        Fx::build("glow", Some("nope"), [])
            .unwrap_err()
            .contains("neon")
    );
    assert!(
        Fx::build("glow", None, [("nope", FxValue::Number(1.))])
            .unwrap_err()
            .contains("radius")
    );
    assert!(Fx::build("nope", None, []).unwrap_err().contains("glow"));
}
