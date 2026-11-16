//! The system font past Medium draws as Anara's Electron app draws it (the
//! pin's reference): font synthesis' fake-bold stroke at 501 to 538 at any
//! size and at 501 to 599 at exactly 14 px, the weight axis alone elsewhere.
//! Measured in that app (Chromium, 2x, `-webkit-font-smoothing: antialiased`)
//! at 11 to 30 px against the 500-600 interpolation: stroked 1.32 to 1.41 of
//! it, unstroked 1.00; 539 and up, and 13.9 or 14.1 px, unstroked.
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

// Ink against the axis's own interpolation between 500 and 600.
fn against_axis(size: f32, weight: u16) -> Option<f64> {
    let (w500, w600) = (column_ink(size, 500)?, column_ink(size, 600)?);
    let ink = column_ink(size, weight)?;
    Some(ink / (w500 + (w600 - w500) * f64::from(weight - 500) / 100.))
}

#[test]
fn system_font_weights_501_to_538_draw_stroked_and_539_up_on_the_axis_alone() {
    if column_ink(16., 500).is_none() {
        eprintln!("SF isn't here: nothing checked");
        return;
    }
    for size in [12., 16., 24.] {
        for weight in [501, 538] {
            let r = against_axis(size, weight).unwrap();
            assert!(
                r > 1.2,
                "{weight} at {size} px is stroked: {r:.3} of the axis's ink"
            );
        }
        for weight in [539, 550, 599] {
            let r = against_axis(size, weight).unwrap();
            assert!(
                (0.97..1.03).contains(&r),
                "{weight} at {size} px is the axis alone: {r:.3}"
            );
        }
    }
}

#[test]
fn at_exactly_14_px_every_weight_past_medium_draws_stroked() {
    if column_ink(14., 500).is_none() {
        eprintln!("SF isn't here: nothing checked");
        return;
    }
    for weight in [550, 599] {
        let r = against_axis(14., weight).unwrap();
        assert!(r > 1.2, "{weight} at 14 px is stroked: {r:.3}");
        for size in [13.9, 14.1] {
            let r = against_axis(size, weight).unwrap();
            assert!(
                (0.97..1.03).contains(&r),
                "{weight} at {size} px is the axis alone: {r:.3}"
            );
        }
    }
    assert!(
        (0.97..1.03).contains(&against_axis(14., 600).unwrap()),
        "600 at 14 px is Semibold itself"
    );
}
