//! WebP decodes (`assets::decode_image`, and kept as drawn), as the lossless
//! picture it was encoded from.
#![cfg(feature = "webp")]
use image::{ExtendedColorType, ImageEncoder};

fn webp_of(width: u32, height: u32, alpha: bool) -> (Vec<u8>, Vec<u8>) {
    let mut rgba = Vec::new();
    for y in 0..height {
        for x in 0..width {
            rgba.extend([
                (x * 255 / width) as u8,
                (y * 255 / height) as u8,
                ((x + y) % 256) as u8,
                if alpha && x % 2 == 0 { 128 } else { 255 },
            ]);
        }
    }
    let mut bytes = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
        .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
        .unwrap();
    (bytes, rgba)
}

#[test]
fn a_lossless_webp_decodes_to_its_pixels() {
    for alpha in [false, true] {
        let (bytes, rgba) = webp_of(37, 23, alpha);
        let image = zgx_gpu::assets::decode_image(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (37, 23));
        assert_eq!(image.pixels(), &rgba[..], "alpha {alpha}");
    }
}

#[test]
fn a_webp_drawn_small_is_kept_and_shrunk() {
    let (bytes, _) = webp_of(512, 384, false);
    let image = zgx_gpu::assets::decode_image_kept(bytes.into()).unwrap();
    assert!(image.keeps_file());
    image.set_decoding(zgx::image::Decoding::Sync);
    let held = image.pixels_at((128, 96)).expect("decoded");
    assert_eq!((held.width, held.height), (128, 96));
}
