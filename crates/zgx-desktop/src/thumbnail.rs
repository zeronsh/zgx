//! Pictures shrunk to thumbnails: a terminal's last frame, a region of what
//! a window showed (`Ui::snapshot`).

/// BGRA premultiplied `bytes` (`width` by `height`, rows `stride` apart)
/// box-filtered down by the least whole factor that fits `max_width`: its
/// size and its straight-alpha RGBA.
#[allow(dead_code)]
pub(crate) fn shrink(
    bytes: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    max_width: usize,
) -> (usize, usize, Vec<u8>) {
    let factor = width.div_ceil(max_width.max(1)).max(1);
    if factor == 1 {
        return (width, height, straight_rgba(bytes, width, height, stride));
    }
    let (w, h) = ((width / factor).max(1), (height / factor).max(1));
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            let mut count = 0;
            for sy in y * factor..((y + 1) * factor).min(height) {
                let row = &bytes[sy * stride..];
                for sx in x * factor..((x + 1) * factor).min(width) {
                    let px = &row[sx * 4..sx * 4 + 4];
                    for (s, v) in sum.iter_mut().zip(px) {
                        *s += u32::from(*v);
                    }
                    count += 1;
                }
            }
            let [b, g, r, a] = sum.map(|s| s / count.max(1));
            // Premultiplied to straight alpha.
            let straight = |c: u32| (c * 255).checked_div(a).map_or(0, |v| v.min(255) as u8);
            let at = (y * w + x) * 4;
            out[at..at + 4].copy_from_slice(&[straight(r), straight(g), straight(b), a as u8]);
        }
    }
    (w, h, out)
}

/// `bytes` as they are, BGRA premultiplied to straight RGBA: `shrink` by a
/// factor of 1 (a picture that fits already, a dragged row's), a pixel at a
/// time, without its sums.
fn straight_rgba(bytes: &[u8], width: usize, height: usize, stride: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for &[b, g, r, a] in bytes[y * stride..][..width * 4].as_chunks::<4>().0 {
            let straight = |c: u8| match a {
                0 => 0,
                255 => c,
                a => (u32::from(c) * 255 / u32::from(a)).min(255) as u8,
            };
            out.extend_from_slice(&[straight(r), straight(g), straight(b), a]);
        }
    }
    out
}
/// `shrink`, as an image.
#[allow(dead_code)]
pub(crate) fn image(
    bytes: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    max_width: usize,
) -> Option<std::sync::Arc<zgx::image::ImageData>> {
    let (w, h, pixels) = shrink(bytes, width, height, stride, max_width);
    zgx::image::ImageData::new(w as u32, h as u32, pixels)
        .ok()
        .map(std::sync::Arc::new)
}

#[cfg(test)]
mod tests {
    use super::shrink;

    #[test]
    fn a_frame_shrinks_by_a_whole_factor_to_straight_rgba() {
        // 4 by 2 BGRA, rows padded to 20 bytes: opaque red on the left
        // half, half-transparent white (premultiplied) on the right.
        let mut bytes = vec![0u8; 20 * 2];
        for y in 0..2 {
            for x in 0..4 {
                let px = if x < 2 {
                    [0, 0, 255, 255]
                } else {
                    [128, 128, 128, 128]
                };
                bytes[y * 20 + x * 4..y * 20 + x * 4 + 4].copy_from_slice(&px);
            }
        }
        let (w, h, out) = shrink(&bytes, 4, 2, 20, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(&out[0..4], &[255, 0, 0, 255]);
        assert_eq!(&out[4..8], &[255, 255, 255, 128]);
        // Fitting already: as it is.
        let (w, h, _) = shrink(&bytes, 4, 2, 20, 8);
        assert_eq!((w, h), (4, 2));
    }

    /// Fitting already, a picture is each pixel straightened alone: the
    /// same bytes the box filter gives at a factor of 1, sums of one.
    #[test]
    fn a_picture_that_fits_is_each_pixel_straightened() {
        let (width, height, stride) = (7, 3, 32);
        let mut bytes = vec![0u8; stride * height];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = (i * 37 % 251) as u8;
        }
        // Premultiplied: no channel past its alpha.
        for px in bytes.as_chunks_mut::<4>().0 {
            let a = px[3];
            for c in &mut px[..3] {
                *c = (*c).min(a);
            }
        }
        let (w, h, fast) = shrink(&bytes, width, height, stride, width);
        assert_eq!((w, h), (width, height));
        let mut summed = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let px = &bytes[y * stride + x * 4..][..4];
                let [b, g, r, a] = [px[0], px[1], px[2], px[3]].map(u32::from);
                let straight = |c: u32| (c * 255).checked_div(a).map_or(0, |v| v.min(255) as u8);
                summed.extend_from_slice(&[straight(r), straight(g), straight(b), a as u8]);
            }
        }
        assert_eq!(fast, summed);
    }
}
