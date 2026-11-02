use std::{borrow::Cow, time::Duration};
use zgx::animation::LoopCount;
fn fixture(repeat: Option<gif::Repeat>) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = gif::Encoder::new(
            &mut bytes,
            3,
            1,
            &[0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255],
        )
        .unwrap();
        if let Some(repeat) = repeat {
            writer.set_repeat(repeat).unwrap();
        }
        let frames = [
            (0, 3, vec![1, 1, 1], gif::DisposalMethod::Keep, 2),
            (1, 1, vec![2], gif::DisposalMethod::Previous, 3),
            (2, 1, vec![3], gif::DisposalMethod::Background, 0),
            (0, 1, vec![2], gif::DisposalMethod::Keep, 1),
        ];
        for (left, width, buffer, dispose, delay) in frames {
            writer
                .write_frame(&gif::Frame {
                    left,
                    width,
                    height: 1,
                    dispose,
                    delay,
                    buffer: Cow::Owned(buffer),
                    ..Default::default()
                })
                .unwrap();
        }
    }
    bytes
}
#[test]
fn gif_offsets_disposal_timing_and_repeat_extension_are_preserved() {
    let data = zgx_gpu::assets::decode_gif(&fixture(None)).unwrap();
    assert_eq!(data.loop_count(), LoopCount::ONCE);
    let pixels: Vec<_> = data.frames().iter().map(|f| f.image.pixels()).collect();
    assert_eq!(pixels[0], &[255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255]);
    assert_eq!(pixels[1], &[255, 0, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255]);
    assert_eq!(pixels[2], &[255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255]);
    assert_eq!(pixels[3], &[0, 255, 0, 255, 255, 0, 0, 255, 0, 0, 0, 0]);
    assert_eq!(data.frames()[2].duration, Duration::from_millis(100));
    assert_eq!(
        zgx_gpu::assets::decode_gif(&fixture(Some(gif::Repeat::Infinite)))
            .unwrap()
            .loop_count(),
        LoopCount::Infinite
    );
    assert_eq!(
        zgx_gpu::assets::decode_gif(&fixture(Some(gif::Repeat::Finite(1))))
            .unwrap()
            .loop_count(),
        LoopCount::Finite(std::num::NonZeroU32::new(2).unwrap())
    );
    assert!(zgx_gpu::assets::decode_gif(b"invalid").is_err());
}

/// A `width` by `height` GIF of `frames` frames, frame `i` filled with
/// palette colour `i % 256` (red `i`, green `255 - i`, blue `i * 7`), each
/// 20 ms, looping.
fn long(width: u16, height: u16, frames: usize) -> Vec<u8> {
    let palette: Vec<u8> = (0..256u32)
        .flat_map(|i| [i as u8, 255 - i as u8, (i * 7) as u8])
        .collect();
    let mut bytes = Vec::new();
    {
        let mut writer = gif::Encoder::new(&mut bytes, width, height, &palette).unwrap();
        writer.set_repeat(gif::Repeat::Infinite).unwrap();
        for i in 0..frames {
            writer
                .write_frame(&gif::Frame {
                    width,
                    height,
                    delay: 2,
                    buffer: Cow::Owned(vec![
                        (i % 256) as u8;
                        usize::from(width) * usize::from(height)
                    ]),
                    ..Default::default()
                })
                .unwrap();
        }
    }
    bytes
}

/// A GIF whose frames don't fit `GIF_KEPT` streams: the same frames as kept
/// (compositing and disposal included), in any order asked, holding one.
#[test]
fn a_streamed_gif_gives_the_frames_a_kept_one_does() {
    use std::sync::Arc;
    for bytes in [fixture(Some(gif::Repeat::Infinite)), long(64, 48, 40)] {
        let bytes: Arc<[u8]> = Arc::from(bytes);
        let kept = zgx_gpu::assets::decode_gif_keeping(bytes.clone(), usize::MAX).unwrap();
        let streamed = zgx_gpu::assets::decode_gif_keeping(bytes, 0).unwrap();
        assert!(!kept.is_streamed() && streamed.is_streamed());
        assert_eq!(kept.len(), streamed.len());
        assert_eq!(kept.cycle_duration(), streamed.cycle_duration());
        assert_eq!(streamed.frames().len(), 1, "its first frame alone kept");
        let n = kept.len();
        // In order, again from the start (a loop), and back and forth.
        let order: Vec<usize> = (0..n)
            .chain(0..n)
            .chain([n - 1, 1, n / 2, 0, n - 2])
            .collect();
        for i in order {
            assert_eq!(
                kept.frame(i).pixels(),
                streamed.frame(i).pixels(),
                "frame {i}"
            );
        }
    }
    // Past the budget by default, a long one streams; a small one doesn't.
    let long = zgx_gpu::assets::decode_gif(&long(400, 300, 20)).unwrap();
    assert!(long.is_streamed());
    assert_eq!(long.frame(7).pixels()[..3], [7, 248, 49]);
    assert!(
        !zgx_gpu::assets::decode_gif(&fixture(None))
            .unwrap()
            .is_streamed()
    );
}
