//! A blurred box's shadow outside its box draws before its blur, so a filter
//! beside it that reads that shadow shares its batch, a split of the window's
//! pass the less: every frame, drawn from its damage, draws what the window
//! drawn whole with its shadow after the blur draws, and in fewer passes.
use std::sync::Arc;
use zgx::{
    compose::prelude::{rgba, *},
    reactive::Signal,
    scene::{BoxShadow, Color, Rect, Transform},
    widgets::Ui,
};
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const W: f32 = 400.;
const H: f32 = 300.;

#[derive(Clone, Copy)]
struct Look {
    name: &'static str,
    page: u32,
    row: u32,
    tint: u32,
    shadow: Color,
    /// Sigma of the composer's shadow, and how far it spreads.
    sigma: f32,
    spread: f32,
    /// Blur of both surfaces (0: opaque material).
    blur: f32,
    radius: f32,
}

/// A list under a composer, with a shadow reaching the pill floating just
/// above it.
fn scene(ui: &mut Ui, offset: Signal<f32>, look: Look) {
    let rows = virtual_list(
        offset,
        24.,
        2,
        || 1000,
        |i| i,
        move |_, key, _| {
            div()
                .w(380.)
                .h(20.)
                .bg(rgb(if key % 2 == 0 { look.row } else { look.page }))
                .child(
                    text(format!("row {key} of a transcript"))
                        .text_color(rgb(0xffffff ^ look.page)),
                )
        },
    )
    .bg(rgb(look.page))
    .w(W)
    .h(H);
    let shadows: Arc<[BoxShadow]> = Arc::new([
        BoxShadow {
            color: look.shadow,
            blur_radius: look.sigma,
            spread: look.spread,
            offset: Transform { x: 0., y: 4. },
        },
        BoxShadow {
            color: Color(0, 0, 0, 40),
            blur_radius: 2.,
            spread: 0.,
            offset: Transform { x: 1., y: 0. },
        },
    ]);
    ui.mount(
        overlay()
            .w(W)
            .h(H)
            .child(rows)
            .child(
                div()
                    .absolute()
                    .left(60.)
                    .top(210.)
                    .size(280., 60.)
                    .rounded(look.radius)
                    .backdrop_blur(look.blur)
                    .bg(rgba(look.tint))
                    .shadows(shadows),
            )
            .child(
                div()
                    .absolute()
                    .left(140.)
                    .top(172.)
                    .size(120., 32.)
                    .rounded(16.)
                    .backdrop_blur(look.blur)
                    .bg(rgba(look.tint)),
            ),
    );
}

const FROST_DARK: Look = Look {
    name: "frost, dark",
    page: 0x101820,
    row: 0x5a7a9a,
    tint: 0x20202080,
    shadow: Color(0, 0, 0, 96),
    sigma: 12.,
    spread: 0.,
    blur: 16.,
    radius: 22.,
};
const LOOKS: [Look; 4] = [
    FROST_DARK,
    Look {
        name: "frost, light",
        page: 0xf4f4f8,
        row: 0xc8d4e0,
        tint: 0xffffffa0,
        shadow: Color(0, 0, 0, 64),
        sigma: 8.,
        spread: 2.,
        blur: 16.,
        radius: 12.,
    },
    Look {
        name: "square corners, wide shadow",
        sigma: 24.,
        radius: 0.,
        ..FROST_DARK
    },
    Look {
        name: "opaque material",
        blur: 0.,
        tint: 0x202020ff,
        ..FROST_DARK
    },
];

/// Scrolls `look` at `scale` in a renderer drawing shadows apart and one
/// drawing them whole, and compares every frame; the passes each took.
fn compare(look: Look, scale: f32, direct: bool) -> (usize, usize) {
    let (w, h) = ((W * scale) as u32, (H * scale) as u32);
    let side = |whole: bool| {
        let mut ui = Ui::new(W, H);
        ui.scene.borrow_mut().set_pixel_scale(scale);
        let offset = ui.signal(0_f32);
        scene(&mut ui, offset.clone(), look);
        let mut gpu = GpuRenderer::new(w, h).unwrap();
        gpu.set_scale_factor(scale);
        gpu.set_direct_metal(direct);
        if whole {
            gpu.debug_keep_shadows_whole();
        }
        (ui, offset, gpu)
    };
    let (ui, offset, mut gpu) = side(false);
    let (plain, plain_offset, mut whole) = side(true);
    let full = Rect::new(0., 0., W, H);
    let (mut apart_passes, mut whole_passes) = (0, 0);
    for (frame, dy) in [0_f32, 8., 16., 40., 2., -24., 60., 100., -8.]
        .iter()
        .enumerate()
    {
        offset.update(|o| *o += dy);
        plain_offset.update(|o| *o += dy);
        // Apart, from the frame's own damage; whole, the whole window.
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        apart_passes += gpu
            .render(&ui.scene.borrow(), &damage)
            .unwrap()
            .render_passes;
        plain.prepare_frame();
        plain.scene.borrow_mut().flush();
        whole_passes += whole
            .render(&plain.scene.borrow(), &[full])
            .unwrap()
            .render_passes;
        let (a, b) = (gpu.readback().unwrap(), whole.readback().unwrap());
        let wrong = a.chunks(4).zip(b.chunks(4)).position(|(a, b)| a != b);
        assert!(
            wrong.is_none(),
            "{} at {scale}x (direct {direct}), frame {frame}: differs at {:?}: {:?} for {:?}",
            look.name,
            wrong.map(|p| (p % w as usize, p / w as usize)),
            wrong.map(|p| &a[p * 4..p * 4 + 4]),
            wrong.map(|p| &b[p * 4..p * 4 + 4]),
        );
    }
    (apart_passes, whole_passes)
}

#[test]
fn a_shadow_apart_from_its_box_draws_the_same_pixels() {
    let _gpu = gpu_fixture();
    for look in LOOKS {
        for scale in [1., 1.5, 2.] {
            for direct in [true, false] {
                compare(look, scale, direct);
            }
        }
    }
}

#[test]
fn a_filter_reading_a_neighbours_shadow_shares_its_batch() {
    let _gpu = gpu_fixture();
    for direct in [true, false] {
        let (apart, whole) = compare(FROST_DARK, 2., direct);
        assert!(
            apart < whole,
            "direct {direct}: {apart} passes apart, {whole} whole"
        );
    }
    // Nothing blurred, nothing to share: the same passes either way.
    let (apart, whole) = compare(LOOKS[3], 2., false);
    assert!(
        apart <= whole,
        "opaque: {apart} passes apart, {whole} whole"
    );
}

/// Glass over a list: two surfaces with shadows, the second's beside its own
/// box. It shares the first's batch, as nothing but its own shadow lies near.
#[test]
fn a_shadowed_filter_joins_the_batch_before_it() {
    let _gpu = gpu_fixture();
    let (scale, (w, h)) = (2., ((W * 2.) as u32, (H * 2.) as u32));
    let shadows = || -> Arc<[BoxShadow]> {
        Arc::new([BoxShadow {
            color: Color(0, 0, 0, 96),
            blur_radius: 2.,
            spread: 0.,
            offset: Transform { x: 0., y: 1. },
        }])
    };
    let side = |whole: bool| {
        let mut ui = Ui::new(W, H);
        ui.scene.borrow_mut().set_pixel_scale(scale);
        let offset = ui.signal(0_f32);
        let glass = |left: f32| {
            div()
                .absolute()
                .left(left)
                .top(200.)
                .size(120., 60.)
                .rounded(16.)
                .backdrop_blur(16.)
                .bg(rgba(0x20202080))
                .shadows(shadows())
        };
        let rows = virtual_list(
            offset.clone(),
            24.,
            2,
            || 1000,
            |i| i,
            |_, key, _| div().w(380.).h(20.).bg(rgb(0x5a7a9a + key as u32)),
        )
        .bg(rgb(0x101820))
        .w(W)
        .h(H);
        ui.mount(
            overlay()
                .w(W)
                .h(H)
                .child(rows)
                .child(glass(20.))
                .child(glass(240.)),
        );
        let mut gpu = GpuRenderer::new(w, h).unwrap();
        gpu.set_scale_factor(scale);
        if whole {
            gpu.debug_keep_shadows_whole();
        }
        (ui, offset, gpu)
    };
    let (ui, offset, mut apart) = side(false);
    let (plain, plain_offset, mut whole) = side(true);
    let full = Rect::new(0., 0., W, H);
    for dy in [0_f32, 16., 40.] {
        offset.update(|o| *o += dy);
        plain_offset.update(|o| *o += dy);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        let one = apart.render(&ui.scene.borrow(), &[full]).unwrap();
        plain.prepare_frame();
        plain.scene.borrow_mut().flush();
        let other = whole.render(&plain.scene.borrow(), &[full]).unwrap();
        assert_eq!(apart.readback().unwrap(), whole.readback().unwrap());
        assert!(
            one.render_passes <= other.render_passes,
            "{} passes apart, {} whole",
            one.render_passes,
            other.render_passes
        );
        assert_eq!(one.blur_passes, 3, "the two glass surfaces share a batch");
    }
}
