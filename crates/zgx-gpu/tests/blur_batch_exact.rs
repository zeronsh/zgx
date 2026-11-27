//! Backdrop filters blurred together in one batch draw what each draws
//! blurred alone. A filter joins the batch before it when nothing drawn since
//! that batch's first filter paints in its box; a blur samples only inside its
//! box, so whatever is drawn outside it (even within its kernel's reach) can't
//! change it. Every pixel, on every renderer.
use zgx::{
    compose::prelude::{rgba, *},
    scene::Rect,
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

/// A glass: frosted, rounded, over whatever lies under it.
fn glass(left: f32, top: f32, w: f32, h: f32, radius: f32) -> View {
    div()
        .absolute()
        .left(left)
        .top(top)
        .size(w, h)
        .rounded(radius)
        .backdrop_blur(16.)
        .bg(rgba(0xffffffa0))
}

/// A transcript under `parts`, drawn in order, at `scale` (through Metal
/// directly when `direct`, where it can): its pixels and blur passes.
fn draw(parts: Vec<View>, scale: f32, direct: bool) -> (Vec<u8>, usize) {
    let mut ui = Ui::new(W, H);
    ui.scene.borrow_mut().set_pixel_scale(scale);
    let offset = ui.signal(0_f32);
    let rows = virtual_list(
        offset,
        24.,
        2,
        || 1000,
        |i| i,
        |_, key, _| {
            div()
                .w(380.)
                .h(20.)
                .bg(rgb(if key % 2 == 0 { 0x5a7a9a } else { 0x101820 }))
                .child(text(format!("row {key} of a transcript")).text_color(rgb(0xffffff)))
        },
    )
    .bg(rgb(0x101820))
    .w(W)
    .h(H);
    let mut root = overlay().w(W).h(H).child(rows);
    for part in parts {
        root = root.child(part);
    }
    ui.mount(root);
    let mut gpu = GpuRenderer::new((W * scale) as u32, (H * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    gpu.set_direct_metal(direct);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let stats = gpu
        .render(&ui.scene.borrow(), &[Rect::new(0., 0., W, H)])
        .unwrap();
    (gpu.readback().unwrap(), stats.blur_passes)
}

/// The first pixel of rows `from..to` (in points) where `a` and `b` differ,
/// with how many do.
fn differs(a: &[u8], b: &[u8], scale: f32, from: f32, to: f32) -> Option<(usize, usize, usize)> {
    let w = (W * scale) as usize;
    let rows = (from * scale) as usize * w..(to * scale) as usize * w;
    let wrong: Vec<usize> = rows
        .filter(|&i| a[i * 4..i * 4 + 4] != b[i * 4..i * 4 + 4])
        .collect();
    wrong.first().map(|&i| (i % w, i / w, wrong.len()))
}

#[test]
fn a_glass_batched_beside_another_draws_as_it_does_alone() {
    let _gpu = gpu_fixture();
    for scale in [1., 2.] {
        for direct in [true, false] {
            let a = || glass(20., 100., 120., 60., 16.);
            let b = || glass(240., 100., 120., 60., 16.);
            let (pair, pair_passes) = draw(vec![a(), b()], scale, direct);
            let (alone, alone_passes) = draw(vec![b()], scale, direct);
            assert_eq!(
                pair_passes, alone_passes,
                "at {scale}x (direct {direct}): the two glasses blur in one batch"
            );
            // Right of A's box and its blur: what B alone draws.
            let w = (W * scale) as usize;
            let right = |pixels: &[u8]| {
                pixels
                    .chunks(4)
                    .enumerate()
                    .filter(|(i, _)| i % w >= (190. * scale) as usize)
                    .map(|(_, p)| p.to_vec())
                    .collect::<Vec<_>>()
            };
            let (pair_right, alone_right) = (right(&pair), right(&alone));
            let wrong = pair_right.iter().zip(&alone_right).filter(|(p, q)| p != q);
            assert_eq!(
                wrong.count(),
                0,
                "at {scale}x (direct {direct}): glass B batched with A differs from it alone"
            );
        }
    }
}

#[test]
fn a_fill_beside_a_batched_glass_is_outside_its_blur() {
    let _gpu = gpu_fixture();
    // Drawn after A, beside B: outside B's box, within its kernel's reach
    // (sigma 16). B still joins A's batch, and draws what it does with the
    // fill drawn before A, its backdrop then complete when the batch began.
    let fill = || {
        div()
            .absolute()
            .left(200.)
            .top(96.)
            .size(32., 68.)
            .bg(rgb(0xffffff))
    };
    for scale in [1., 2.] {
        for direct in [true, false] {
            let a = || glass(20., 100., 120., 60., 16.);
            let b = || glass(240., 100., 120., 60., 16.);
            let (after, after_passes) = draw(vec![a(), fill(), b()], scale, direct);
            let (before, before_passes) = draw(vec![fill(), a(), b()], scale, direct);
            assert_eq!(
                after_passes, before_passes,
                "at {scale}x (direct {direct}): one batch either way"
            );
            assert_eq!(
                differs(&after, &before, scale, 0., H),
                None,
                "at {scale}x (direct {direct}): (x, y, pixels differing)"
            );
        }
    }
}

#[test]
fn a_pill_batched_above_a_composer_draws_as_it_does_alone() {
    let _gpu = gpu_fixture();
    // A composer and a pill floating just above it, both glass: one batch.
    for scale in [1., 2.] {
        for direct in [true, false] {
            let composer = || glass(60., 210., 280., 60., 12.);
            let pill = || glass(140., 172., 120., 32., 16.);
            let (both, both_passes) = draw(vec![composer(), pill()], scale, direct);
            let (alone, alone_passes) = draw(vec![pill()], scale, direct);
            assert_eq!(
                both_passes, alone_passes,
                "at {scale}x (direct {direct}): the composer and the pill blur in one batch"
            );
            // Above the composer and its blur: what the pill alone draws.
            assert_eq!(
                differs(&both, &alone, scale, 0., 206.),
                None,
                "at {scale}x (direct {direct}): (x, y, pixels differing)"
            );
        }
    }
}
