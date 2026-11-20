//! A picture of an area of what a window showed (`ShownFrame::read`, what
//! `Ui::snapshot` reads off Linux's and Windows' renderers): only that
//! region, as premultiplied BGRA rows, clipped to the frame, nothing past
//! it. (macOS reads the layer's surface, which an offscreen renderer has
//! none of: its pictures are checked through windows.)
#![cfg(not(target_os = "macos"))]
use zgx::{compose::prelude::*, scene::Rect, widgets::Ui};
use zgx_gpu::GpuRenderer;

const W: u32 = 120;
const H: u32 = 80;

#[test]
fn a_region_of_the_shown_frame_reads_as_it_was_drawn() {
    let mut ui = Ui::new(W as f32, H as f32);
    // Black, and a red box at (10, 20), 30 by 15.
    ui.mount(
        div().w_full().h_full().bg(rgb(0x000000)).child(
            div()
                .absolute()
                .ml(10.)
                .mt(20.)
                .w(30.)
                .h(15.)
                .bg(rgb(0xff0000)),
        ),
    );
    ui.try_prepare_frame().unwrap();
    ui.scene.borrow_mut().flush();
    let mut gpu = GpuRenderer::new(W, H).unwrap();
    gpu.render(&ui.scene.borrow(), &[Rect::new(0., 0., W as f32, H as f32)])
        .unwrap();
    let shown = gpu.shown_frame().expect("a frame to read");
    // The box alone: red, opaque, its size.
    let (w, h, pixels) = shown.read(10, 20, 30, 15).expect("the box");
    assert_eq!((w, h), (30, 15));
    assert_eq!(pixels.len(), 30 * 15 * 4);
    for pixel in pixels.as_chunks::<4>().0 {
        assert_eq!(*pixel, [0, 0, 255, 255], "BGRA red");
    }
    // Across its left edge: black, then red, row by row.
    let (w, _, pixels) = shown.read(5, 25, 10, 2).expect("its edge");
    assert_eq!(w, 10);
    let row: Vec<_> = pixels[..40]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p[2])
        .collect();
    assert_eq!(
        row,
        [0, 0, 0, 0, 0, 255, 255, 255, 255, 255],
        "red from x 10"
    );
    // Past the frame's corner: what is in it, no more.
    let (w, h, _) = shown.read(W - 4, H - 3, 50, 50).expect("its corner");
    assert_eq!((w, h), (4, 3));
    assert!(shown.read(W, 0, 10, 10).is_none(), "nothing past its edge");
}
