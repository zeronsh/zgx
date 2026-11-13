//! The system font past Medium and before Semibold draws emboldened under 17.5
//! points, as Chrome's font synthesis does (headless Chrome 157, 2x,
//! `-webkit-font-smoothing: antialiased`, "Accuracy drops more than speed":
//! ink per inked column, 500 2358, 501 3071, 550 3183, 599 3290, 600 2588 at
//! 16 px; none at 18 px): the ratios to 500 within 3%.
#![cfg(target_os = "macos")]
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_gpu::GpuRenderer;

fn column_ink(size: f32, weight: u16) -> Option<f64> {
    let mut gpu = GpuRenderer::new(400, 120).unwrap();
    gpu.set_scale_factor(2.);
    let family = gpu
        .text_system()
        .borrow()
        .db()
        .faces()
        .find(|f| f.post_script_name.starts_with(".SFNS-"))
        .and_then(|f| f.families.first().map(|(n, _)| n.clone()))?;
    let mut ui = Ui::new(200., 60.);
    gpu.install_text(&mut ui.scene.borrow_mut());
    ui.mount(
        div().size(200., 60.).bg(Color(255, 255, 255, 255)).child(
            div().absolute().ml(8.).mt(8.).w(190.).h(36.).child(
                text("Accuracy drops more than speed")
                    .font_family(family.as_str())
                    .font_weight(weight)
                    .text_size(size)
                    .line_height(size * 1.5)
                    .text_color(Color(0, 0, 0, 255)),
            ),
        ),
    );
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    let px = gpu.readback().unwrap();
    let (mut total, mut columns) = (0u64, 0u64);
    for x in 0..400usize {
        let col: Vec<u8> = (0..120).map(|y| px[(y * 400 + x) * 4]).collect();
        if col.iter().any(|v| *v < 128) {
            columns += 1;
        }
        total += col.iter().map(|v| u64::from(255 - *v)).sum::<u64>();
    }
    Some(total as f64 / columns.max(1) as f64)
}

#[test]
fn system_font_weights_501_to_599_are_emboldened_under_17_points() {
    let Some(plain) = column_ink(16., 500) else {
        eprintln!("SF isn't here: nothing checked");
        return;
    };
    // Chrome, against 500: 501 1.302, 550 1.350, 599 1.395, 600 1.098.
    let ratio = |got: f64, want: f64, what: &str| {
        assert!(
            (got / plain / want - 1.).abs() < 0.03,
            "{what}: {:.3} against Chrome's {want:.3}",
            got / plain
        );
    };
    ratio(column_ink(16., 501).unwrap(), 1.302, "501 at 16 px");
    ratio(column_ink(16., 550).unwrap(), 1.350, "550 at 16 px");
    ratio(column_ink(16., 599).unwrap(), 1.395, "599 at 16 px");
    ratio(column_ink(16., 600).unwrap(), 1.098, "600 at 16 px");
    // Not at 18 px: 550 is lighter than 600 there.
    assert!(column_ink(18., 550).unwrap() < column_ink(18., 600).unwrap());
}
