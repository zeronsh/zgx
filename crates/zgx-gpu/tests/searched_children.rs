//! A damaged frame finds a long column's children meeting its damage by
//! search (`Scene::layer_items_within`), not by asking each: every frame
//! here, as rows stream in, one grows and pushes those after it, and the
//! view scrolls, is a fresh full render's, shadows past the rows included.
use zgx::compose::prelude::*;
use zgx::scene::{BoxShadow, Color, Rect, Transform};
use zgx::widgets::Ui;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const W: f32 = 320.;
const H: f32 = 240.;

/// Row `n`, `tall` points more if it is the one grown: a box, every third
/// with a shadow reaching below it, and a line of text.
fn row(n: usize, tall: f32) -> View {
    let row = div()
        .w(150. + (n * 13 % 50) as f32)
        .h(10. + (n * 7 % 6) as f32 + tall)
        .bg(rgb(0x203040 + (n as u32 * 0x0b0a09) % 0x505050))
        .child(
            text(format!("Row {n}"))
                .text_size(9.)
                .text_color(rgb(0xffffff)),
        );
    match n % 3 {
        0 => row.shadow(BoxShadow {
            color: Color(0, 0, 0, 160),
            offset: Transform { x: 0., y: 5. },
            blur_radius: 6.,
            spread: 0.,
        }),
        _ => row,
    }
}

#[test]
fn a_long_column_s_damaged_frames_are_full_frames() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    let mut ui = Ui::new(W, H);
    let count = ui.signal(70_usize);
    let grown = ui.signal(0_f32);
    let offset = ui.signal(0_f32);
    // Row 20 keyed by how tall it is: made again as it grows, the rows
    // after it laid out lower.
    let rows = {
        let (count, grown) = (count.clone(), grown.clone());
        keyed(
            move || {
                let tall = grown.get();
                (0..count.get())
                    .map(|n| (n, if n == 20 { tall.to_bits() } else { 0 }))
                    .collect::<Vec<_>>()
            },
            |(n, tall), _| row(n, f32::from_bits(tall)),
        )
    };
    ui.mount(
        div().w(W).h(H).bg(rgb(0x101010)).child(
            scroll(offset.clone())
                .w(220.)
                .h(200.)
                .bg(rgb(0x182028))
                .child(column().gap(3.).p(4.).child(rows)),
        ),
    );
    let steps: [(usize, f32, f32); 10] = [
        (70, 0., 0.),
        (74, 0., 0.),
        (74, 30., 0.),
        (78, 30., 120.),
        (82, 5., 120.),
        (82, 5., 300.),
        (86, 60., 310.5),
        (90, 60., 0.),
        (94, 0., 2000.),
        (100, 12., 2000.),
    ];
    for (rows, tall, scrolled) in steps {
        count.set(rows);
        grown.set(tall);
        offset.set(scrolled);
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        let mut fresh = GpuRenderer::new(W as u32, H as u32).unwrap();
        fresh
            .render(&ui.scene.borrow(), &[Rect::new(0., 0., W, H)])
            .unwrap();
        let (a, b) = (gpu.readback().unwrap(), fresh.readback().unwrap());
        let differs = (0..(W * H) as usize).find(|i| a[i * 4..][..4] != b[i * 4..][..4]);
        assert!(
            differs.is_none(),
            "{rows} rows, row 20 {tall} taller, scrolled {scrolled}: at {:?} {:?} where a full frame shows {:?}",
            differs.map(|i| (i % W as usize, i / W as usize)),
            differs.map(|i| &a[i * 4..][..4]),
            differs.map(|i| &b[i * 4..][..4]),
        );
    }
}
