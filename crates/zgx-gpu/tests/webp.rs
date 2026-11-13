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

/// A still lossless WebP's `VP8L` chunk (its header and bitstream).
fn vp8l_of(color: [u8; 4], width: u32, height: u32) -> Vec<u8> {
    let rgba: Vec<u8> = (0..width * height).flat_map(|_| color).collect();
    let mut bytes = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
        .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
        .unwrap();
    // RIFF, size, WEBP, then chunks: find ours.
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"VP8L" {
            return bytes[at..at + 8 + size + (size & 1)].to_vec();
        }
        at += 8 + size + (size & 1);
    }
    panic!("no VP8L chunk");
}

/// An animated WebP of two frames (container by hand, the frames from the
/// encoder's stills): it plays, each frame as long as it says.
#[test]
fn an_animated_webp_decodes_to_its_frames_and_delays() {
    let (w, h) = (16u32, 8u32);
    let le24 = |v: u32| [v as u8, (v >> 8) as u8, (v >> 16) as u8];
    let mut chunks = Vec::new();
    let mut add = |tag: &[u8; 4], payload: &[u8]| {
        chunks.extend_from_slice(tag);
        chunks.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        chunks.extend_from_slice(payload);
        if payload.len() & 1 == 1 {
            chunks.push(0);
        }
    };
    let mut vp8x = vec![0x12, 0, 0, 0];
    vp8x.extend(le24(w - 1));
    vp8x.extend(le24(h - 1));
    add(b"VP8X", &vp8x);
    add(b"ANIM", &[0, 0, 0, 0, 0, 0]);
    for (color, ms) in [([255, 0, 0, 255], 100), ([0, 0, 255, 255], 200)] {
        let mut anmf = Vec::new();
        anmf.extend(le24(0));
        anmf.extend(le24(0));
        anmf.extend(le24(w - 1));
        anmf.extend(le24(h - 1));
        anmf.extend(le24(ms));
        anmf.push(0);
        anmf.extend(vp8l_of(color, w, h));
        add(b"ANMF", &anmf);
    }
    let mut file = b"RIFF".to_vec();
    file.extend(((chunks.len() + 4) as u32).to_le_bytes());
    file.extend(b"WEBP");
    file.extend(chunks);

    for image in [
        zgx_gpu::assets::decode_image(&file).unwrap(),
        zgx_gpu::assets::decode_image_kept(file.clone().into()).unwrap(),
    ] {
        let animation = image.animation().expect("an animation");
        assert_eq!(animation.len(), 2);
        let frames = animation.frames();
        assert_eq!(frames[0].duration, std::time::Duration::from_millis(100));
        assert_eq!(frames[1].duration, std::time::Duration::from_millis(200));
        // (The decoder blends each frame over the canvas: a level out.)
        let near =
            |got: &[u8], want: [u8; 4]| got.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2);
        assert!(near(&frames[0].image.pixels()[..4], [255, 0, 0, 255]));
        assert!(near(&frames[1].image.pixels()[..4], [0, 0, 255, 255]));
    }
}
