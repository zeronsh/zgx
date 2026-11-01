//! Nine-slice images (`Scene::set_image_slice`, a nine-patch): corners keep
//! their size, edges stretch along their side, the middle both ways, and an
//! image without a slice draws whole as before.
use std::sync::Arc;
use zgx::image::{ImageData, ImageSlice};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

const SIZE: u32 = 60;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Each cell's colour, row by row.
const CELLS: [[u8; 3]; 9] = [
    [255, 0, 0],
    [0, 255, 0],
    [0, 0, 255],
    [255, 255, 0],
    [255, 255, 255],
    [0, 255, 255],
    [255, 0, 255],
    [128, 128, 128],
    [255, 128, 0],
];

/// A 9 by 9 image of nine 3 by 3 cells, each its own colour.
fn cells() -> Arc<ImageData> {
    let mut pixels = Vec::with_capacity(9 * 9 * 4);
    for y in 0..9 {
        for x in 0..9 {
            let [r, g, b] = CELLS[(y / 3) * 3 + x / 3];
            pixels.extend([r, g, b, 255]);
        }
    }
    Arc::new(ImageData::new(9, 9, pixels).unwrap())
}

/// The image filling a `SIZE` square, sliced or not; its pixels.
fn draw(slice: Option<ImageSlice>) -> Vec<u8> {
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let node = scene.append(
        scene.root(),
        NodeKind::Image(cells()),
        Style {
            width: Some(SIZE as f32),
            height: Some(SIZE as f32),
            ..Default::default()
        },
    );
    scene.set_image_slice(node, slice);
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    gpu.readback().unwrap()
}

fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 8)
}

#[test]
fn a_sliced_image_keeps_its_corners_and_stretches_its_edges() {
    let _gpu = gpu_fixture();
    let sliced = draw(Some(ImageSlice::new([3.; 4], [10.; 4])));
    // Each piece's middle: corners 10 points square, edges and the middle
    // stretched between them.
    let centres = [5, 30, 55];
    for (row, y) in centres.iter().enumerate() {
        for (column, x) in centres.iter().enumerate() {
            let want = CELLS[row * 3 + column];
            let got = at(&sliced, *x, *y);
            assert!(
                near(got, want),
                "piece ({row}, {column}) at ({x}, {y}): {got:?}, not {want:?}"
            );
        }
    }
    // 15 points in along the top: the top edge, where the image drawn whole
    // would still be its corner.
    let whole = draw(None);
    assert!(near(at(&sliced, 15, 5), CELLS[1]), "the top edge");
    assert!(near(at(&whole, 15, 5), CELLS[0]), "whole: its corner");
}

#[test]
fn widths_past_the_box_shrink_together() {
    let _gpu = gpu_fixture();
    // 40 points a side in a 60 point box: each side's share, 30 points.
    let sliced = draw(Some(ImageSlice::new([3.; 4], [40.; 4])));
    assert!(near(at(&sliced, 15, 15), CELLS[0]), "the top-left corner");
    assert!(
        near(at(&sliced, 45, 45), CELLS[8]),
        "the bottom-right corner"
    );
}
