//! The system font past Medium and before Semibold draws on its weight axis
//! alone, with no fake-bold stroke, as Anara's Electron app draws it (pin
//! only: the pin's reference is that app). Two measurements of Chrome
//! disagree, and what differs between them is not yet known:
//!
//! - Headless Chrome 157, 2x, `-webkit-font-smoothing: antialiased`,
//!   "Accuracy drops more than speed": ink per inked column 500 2358, 501
//!   3071, 550 3183, 599 3290, 600 2588 at 16 px, none at 18 px: 30-40% more
//!   ink at 501 to 599 under 17.5 points, font synthesis' stroke.
//! - Anara's Electron app: 550's ink equal to the unstroked draw (3088 and
//!   3088), where the stroke drew about 35% heavier.
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
fn system_font_weights_501_to_599_draw_on_the_axis_alone() {
    let Some(w500) = column_ink(16., 500) else {
        eprintln!("SF isn't here: nothing checked");
        return;
    };
    let w600 = column_ink(16., 600).unwrap();
    // Between Medium and Semibold, as the axis goes: never past Semibold,
    // as the stroke drew them (501 1.302 of 500's ink, 600 1.098).
    let mut last = w500;
    for weight in [501, 550, 599] {
        let ink = column_ink(16., weight).unwrap();
        assert!(
            ink >= last * 0.99 && ink <= w600 * 1.01,
            "{weight} at 16 px: {ink:.0}, between {last:.0} and 600's {w600:.0}"
        );
        last = ink;
    }
    let w501 = column_ink(16., 501).unwrap();
    assert!(
        w501 / w500 < 1.03,
        "501 at 16 px is 500's weight, not stroked: {:.3} of 500's ink",
        w501 / w500
    );
    // And at 18 px, past where the stroke ever was, 550 lighter than 600.
    assert!(column_ink(18., 550).unwrap() < column_ink(18., 600).unwrap());
}
