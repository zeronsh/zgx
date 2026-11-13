//! A wrapping row breaks on the unrounded text widths, as Chrome does: 55.43 + 4 + 179.38
//! is 238.81, which does not fit 238.
#![cfg(target_os = "macos")]
use std::sync::Arc;
use zgx::layout::{FlexWrap, Length};
use zgx::scene::*;
use zgx::text_layout::{FontStyle, LetterSpacing};
use zgx_gpu::GpuRenderer;
#[test]
fn a_wrap_row_breaks_on_the_unrounded_sum_of_its_texts() {
    let gpu = GpuRenderer::new(600, 200).unwrap();
    let mut scene = Scene::new(600., 200.);
    gpu.install_text(&mut scene);
    let row = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Row),
        Style {
            width: Some(238.),
            layout_options: Some(Arc::new(zgx::layout::LayoutOptions {
                wrap: Some(FlexWrap::Wrap),
                gap_x: Some(Length::Px(4.)),
                ..Default::default()
            })),
            ..Style::default()
        },
    );
    let mut add = |text: &str, size: f32, font: FontStyle| {
        let id = scene.append(
            row,
            NodeKind::Text {
                text: text.into(),
                color: Color(0, 0, 0, 255),
                font_size: size,
            },
            Style::default(),
        );
        scene.set_font(id, font);
        id
    };
    let a = add(
        "$100",
        24.,
        FontStyle {
            weight: 600,
            letter_spacing: LetterSpacing::px(-0.6),
            ..Default::default()
        },
    );
    let b = add(
        "/ month / seat, billed monthly",
        13.,
        FontStyle {
            weight: 500,
            ..Default::default()
        },
    );
    scene.flush();
    assert_eq!(scene.bounds(a).y, 0.);
    assert!(
        scene.bounds(b).y > 0.,
        "the suffix wraps under the price: {:?}",
        scene.bounds(b)
    );
}
