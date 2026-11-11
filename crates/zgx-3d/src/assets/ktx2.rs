//! KTX2 files (`Texture::decode`, the `ktx2` feature): RGBA8 texels, sRGB
//! or not, and block-compressed ones (BC1 to BC7, ETC2 and EAC, ASTC),
//! Zstandard-supercompressed or not, their levels taken as they are (no
//! decoding, and their smaller levels the file's own, not made again).
//! Basis Universal payloads (ETC1S, UASTC) are transcoded with the
//! `ktx2-basis` feature (`Ktx2::transcoded`), refused without it; files of
//! more than one 2D image are refused, saying so.
use std::borrow::Cow;
use zgx_gpu::wgpu;

/// A KTX2 file's identifier, its first 12 bytes.
pub(crate) const IDENTIFIER: [u8; 12] = *b"\xabKTX 20\xbb\r\n\x1a\n";

/// The most bytes a level may hold, decompressed (as a PNG's pixels may).
const MAX_LEVEL: usize = 64 << 20;

/// Vulkan's `VK_FORMAT_R8G8B8A8_UNORM` and `_SRGB`.
const RGBA8: u32 = 37;
const RGBA8_SRGB: u32 = 43;

/// What a KTX2 file's texels are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Texels {
    /// Four bytes each.
    Rgba8,
    /// Blocks of this format (its sRGB-ness as the file says: a map reads
    /// it as its use needs).
    Blocks(wgpu::TextureFormat),
    /// Basis Universal's (ETC1S or UASTC), transcoded to what a device
    /// holds (`Ktx2::transcoded`): the `ktx2-basis` feature.
    #[cfg(feature = "ktx2-basis")]
    Basis,
}

/// What a KTX2 file holds that a texture reads: its size, its texels, and
/// where each of its levels is in it, the first the largest.
#[derive(Debug, PartialEq)]
pub(crate) struct Ktx2 {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) texels: Texels,
    zstd: bool,
    /// Each level's bytes in the file, and how many they are decompressed.
    levels: Vec<(std::ops::Range<usize>, usize)>,
}

impl Ktx2 {
    /// How many levels it has.
    pub(crate) fn level_count(&self) -> usize {
        self.levels.len()
    }
    /// Level `level`'s texels from `file` (the bytes it was read from): as
    /// they're stored, or decompressed. `None` if they don't decompress to
    /// what the file says.
    pub(crate) fn level<'a>(&self, file: &'a [u8], level: usize) -> Option<Cow<'a, [u8]>> {
        let (range, length) = self.levels.get(level)?;
        let stored = file.get(range.clone())?;
        if !self.zstd {
            return Some(Cow::Borrowed(stored));
        }
        let mut out = vec![0; *length];
        let written = ruzstd::decoding::FrameDecoder::new()
            .decode_all(stored, &mut out)
            .ok()?;
        (written == *length).then_some(Cow::Owned(out))
    }
    /// Its first `count` levels (or all it has, if fewer) transcoded to
    /// `target`, from `file` (the bytes it was read from; its Basis
    /// Universal texels), the first the largest: each its blocks or RGBA8
    /// texels, as long as its size says. `None` if the file doesn't
    /// transcode: a level would be more than `MAX_LEVEL` (checked before the
    /// transcoder makes it), the transcoder refused it, gave other sizes, or
    /// panicked (caught here, on whichever thread: a hostile file is a map
    /// that draws as if it had none).
    #[cfg(feature = "ktx2-basis")]
    pub(crate) fn transcoded(
        &self,
        file: &[u8],
        target: Target,
        count: usize,
    ) -> Option<Vec<Vec<u8>>> {
        if self.texels != Texels::Basis {
            return None;
        }
        let count = count.min(self.level_count());
        let expected = |level| self.transcoded_size(target, level);
        if !self.transcodes_within(target) {
            return None;
        }
        let format = match target {
            Target::Bc7 => basisu::TargetFormat::Bc7Rgba,
            Target::Astc => basisu::TargetFormat::Astc4x4Rgba,
            Target::Rgba8 => basisu::TargetFormat::Rgba32,
        };
        // Zstandard's levels decompressed here (by this crate's decoder, not
        // a second one): the file as if stored so.
        let stored;
        let input = if self.zstd {
            stored = self.decompressed(file)?;
            &stored[..]
        } else {
            file
        };
        std::panic::catch_unwind(|| {
            let transcoder = basisu::Transcoder::new(input).ok()?;
            if transcoder.level_count() as usize != self.level_count() {
                return None;
            }
            (0..count)
                .map(|level| {
                    let out = transcoder
                        .transcode(level as u32, format, basisu::DecodeFlags::NONE)
                        .ok()?;
                    (out.len() == expected(level)).then_some(out)
                })
                .collect()
        })
        .ok()?
    }
    /// Level `level`'s bytes transcoded to `target`.
    #[cfg(feature = "ktx2-basis")]
    fn transcoded_size(&self, target: Target, level: usize) -> usize {
        let (w, h) = (
            (self.width >> level).max(1) as usize,
            (self.height >> level).max(1) as usize,
        );
        match target {
            Target::Rgba8 => w * h * 4,
            Target::Bc7 | Target::Astc => w.div_ceil(4) * h.div_ceil(4) * 16,
        }
    }
    /// Whether its largest level transcoded to `target` is within
    /// `MAX_LEVEL` (an 8192² map's RGBA8 texels aren't, its BC7 blocks
    /// are): if not, a device that takes `target` refuses it, unread, and
    /// another may take it (`Texture::kept_levels`).
    #[cfg(feature = "ktx2-basis")]
    pub(crate) fn transcodes_within(&self, target: Target) -> bool {
        self.transcoded_size(target, 0) <= MAX_LEVEL
    }
    /// `file` with its Zstandard levels decompressed, as if stored so: its
    /// header, index and other sections as they are (all before its first
    /// level), its levels after them. `None` if a level doesn't decompress
    /// or a section isn't before them.
    #[cfg(feature = "ktx2-basis")]
    fn decompressed(&self, file: &[u8]) -> Option<Vec<u8>> {
        let first = self.levels.iter().map(|(range, _)| range.start).min()?;
        let mut out = file.get(..first)?.to_vec();
        out.get_mut(44..48)?.copy_from_slice(&0u32.to_le_bytes());
        for level in 0..self.level_count() {
            let texels = self.level(file, level)?;
            // Each level on 16 bytes, as KTX2 lays UASTC's blocks.
            out.resize(out.len().next_multiple_of(16), 0);
            let (at, length) = (out.len() as u64, texels.len() as u64);
            let index = 80 + 24 * level;
            out.get_mut(index..index + 24)?
                .copy_from_slice(&[at, length, length].map(u64::to_le_bytes).concat());
            out.extend_from_slice(&texels);
        }
        Some(out)
    }
}

/// What a Basis Universal texture is transcoded to: what the device holds.
#[cfg(feature = "ktx2-basis")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Target {
    Bc7,
    Astc,
    Rgba8,
}
#[cfg(feature = "ktx2-basis")]
impl Target {
    /// The best of BC7 and ASTC 4x4 a device with `features` holds for a
    /// texture of `size` (whole 4 by 4 blocks, as WebGPU asks of them), else
    /// RGBA8; and the texture format it's put on the GPU as (`None`: RGBA8).
    pub(crate) fn for_device(
        features: wgpu::Features,
        (width, height): (u32, u32),
    ) -> (Target, Option<wgpu::TextureFormat>) {
        let whole = width.is_multiple_of(4) && height.is_multiple_of(4);
        if whole && features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
            (Target::Bc7, Some(wgpu::TextureFormat::Bc7RgbaUnormSrgb))
        } else if whole && features.contains(wgpu::Features::TEXTURE_COMPRESSION_ASTC) {
            let format = wgpu::TextureFormat::Astc {
                block: wgpu::AstcBlock::B4x4,
                channel: wgpu::AstcChannel::UnormSrgb,
            };
            (Target::Astc, Some(format))
        } else {
            (Target::Rgba8, None)
        }
    }
}

/// Vulkan format `format` as a block-compressed texture format.
fn blocks(format: u32) -> Option<wgpu::TextureFormat> {
    use wgpu::{AstcBlock as B, AstcChannel, TextureFormat as F};
    let srgb = |unorm, srgb_: bool| if srgb_ { AstcChannel::UnormSrgb } else { unorm };
    Some(match format {
        // BC1's RGB and RGBA blocks are the same blocks.
        131 | 133 => F::Bc1RgbaUnorm,
        132 | 134 => F::Bc1RgbaUnormSrgb,
        135 => F::Bc2RgbaUnorm,
        136 => F::Bc2RgbaUnormSrgb,
        137 => F::Bc3RgbaUnorm,
        138 => F::Bc3RgbaUnormSrgb,
        139 => F::Bc4RUnorm,
        140 => F::Bc4RSnorm,
        141 => F::Bc5RgUnorm,
        142 => F::Bc5RgSnorm,
        143 => F::Bc6hRgbUfloat,
        144 => F::Bc6hRgbFloat,
        145 => F::Bc7RgbaUnorm,
        146 => F::Bc7RgbaUnormSrgb,
        147 => F::Etc2Rgb8Unorm,
        148 => F::Etc2Rgb8UnormSrgb,
        149 => F::Etc2Rgb8A1Unorm,
        150 => F::Etc2Rgb8A1UnormSrgb,
        151 => F::Etc2Rgba8Unorm,
        152 => F::Etc2Rgba8UnormSrgb,
        153 => F::EacR11Unorm,
        154 => F::EacR11Snorm,
        155 => F::EacRg11Unorm,
        156 => F::EacRg11Snorm,
        157..=184 => {
            let block = [
                B::B4x4,
                B::B5x4,
                B::B5x5,
                B::B6x5,
                B::B6x6,
                B::B8x5,
                B::B8x6,
                B::B8x8,
                B::B10x5,
                B::B10x6,
                B::B10x8,
                B::B10x10,
                B::B12x10,
                B::B12x12,
            ][((format - 157) / 2) as usize];
            F::Astc {
                block,
                channel: srgb(AstcChannel::Unorm, (format - 157) % 2 == 1),
            }
        }
        _ => return None,
    })
}

/// `bytes` read as a KTX2 file, or why they can't be.
pub(crate) fn read(bytes: &[u8]) -> Result<Ktx2, String> {
    let short = || "a KTX2 file cut short".to_owned();
    let u32_at = |at: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            bytes.get(at..at + 4).ok_or_else(short)?.try_into().unwrap(),
        ))
    };
    let u64_at = |at: usize| -> Result<u64, String> {
        Ok(u64::from_le_bytes(
            bytes.get(at..at + 8).ok_or_else(short)?.try_into().unwrap(),
        ))
    };
    if !bytes.starts_with(&IDENTIFIER) {
        return Err("not a KTX2 file".into());
    }
    let format = u32_at(12)?;
    let (width, height, depth) = (u32_at(20)?, u32_at(24)?, u32_at(28)?);
    let (layers, faces, count, supercompression) =
        (u32_at(32)?, u32_at(36)?, u32_at(40)?, u32_at(44)?);
    let texels = match format {
        RGBA8 | RGBA8_SRGB => Texels::Rgba8,
        #[cfg(feature = "ktx2-basis")]
        0 => Texels::Basis,
        #[cfg(not(feature = "ktx2-basis"))]
        0 => {
            return Err(
                "a Basis Universal KTX2 texture (ETC1S or UASTC): it needs zgx-3d's \
                 `ktx2-basis` feature"
                    .into(),
            );
        }
        other => Texels::Blocks(blocks(other).ok_or_else(|| {
            format!("a KTX2 texture of Vulkan format {other}: not RGBA8, BC, ETC2, EAC or ASTC")
        })?),
    };
    // None, or Zstandard (2); BasisLZ (1) is Basis Universal's ETC1S, read
    // by its transcoder (its levels aren't ours to size).
    #[cfg(feature = "ktx2-basis")]
    let basislz = supercompression == 1 && texels == Texels::Basis;
    #[cfg(not(feature = "ktx2-basis"))]
    let basislz = false;
    let zstd = match supercompression {
        0 => false,
        2 => true,
        1 if basislz => false,
        other => {
            return Err(format!(
                "a KTX2 texture of supercompression {other}: not read"
            ));
        }
    };
    if depth > 1 || layers > 1 || faces != 1 {
        return Err("a KTX2 texture of more than one 2D image: not read".into());
    }
    let max = crate::texture::MAX_SIDE;
    if width == 0 || height == 0 || width > max || height > max {
        return Err("a texture is 1 to 8192 pixels on a side".into());
    }
    let (bw, bh) = match texels {
        Texels::Rgba8 => (1, 1),
        Texels::Blocks(format) => format.block_dimensions(),
        // UASTC's 4 by 4 blocks; any size (transcoded to RGBA8 where it
        // isn't whole blocks).
        #[cfg(feature = "ktx2-basis")]
        Texels::Basis => (4, 4),
    };
    #[cfg(feature = "ktx2-basis")]
    let whole_blocks_asked = texels != Texels::Basis;
    #[cfg(not(feature = "ktx2-basis"))]
    let whole_blocks_asked = true;
    if whole_blocks_asked && (!width.is_multiple_of(bw) || !height.is_multiple_of(bh)) {
        return Err(format!(
            "a KTX2 texture of {bw} by {bh} blocks {width} by {height}: not whole blocks"
        ));
    }
    let block = match texels {
        Texels::Rgba8 => 4,
        Texels::Blocks(format) => format.block_copy_size(None).unwrap_or(16) as usize,
        #[cfg(feature = "ktx2-basis")]
        Texels::Basis => 16,
    };
    // No levels given: the first alone.
    let count = count.max(1);
    if count > 32 - width.max(height).leading_zeros() {
        return Err("a KTX2 texture with more levels than its size has".into());
    }
    // The level index, after the 48-byte header and the 32-byte index of
    // its other sections.
    let mut levels = Vec::with_capacity(count as usize);
    for level in 0..count {
        let at = 80 + 24 * level as usize;
        let (offset, stored, length) = (u64_at(at)?, u64_at(at + 8)?, u64_at(at + 16)?);
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        let expected = (w.div_ceil(bw) * h.div_ceil(bh)) as usize * block;
        // As much as a PNG's pixels may be: no file asks for more memory.
        if expected > MAX_LEVEL {
            return Err("a KTX2 level of more than 64 MiB".into());
        }
        let length = if zstd { length } else { stored };
        // ETC1S's levels are its transcoder's to size: only no larger than
        // a level may be.
        if basislz && stored > MAX_LEVEL as u64 {
            return Err("a KTX2 level of more than 64 MiB".into());
        }
        if !basislz && length != expected as u64 {
            return Err(format!(
                "a KTX2 texture's level {level} isn't {w} by {h}'s {expected} bytes"
            ));
        }
        let start = usize::try_from(offset).map_err(|_| "a KTX2 level past the file")?;
        let end = usize::try_from(stored)
            .ok()
            .and_then(|stored| start.checked_add(stored))
            .filter(|&end| end <= bytes.len())
            .ok_or("a KTX2 level past the file")?;
        levels.push((start..end, expected));
    }
    Ok(Ktx2 {
        width,
        height,
        texels,
        zstd,
        levels,
    })
}

/// A KTX2 file of `levels` (each half the one before, the first `width` by
/// `height`) in Vulkan `format`, stored as they are: for tests.
#[cfg(test)]
pub(crate) fn write(width: u32, height: u32, format: u32, levels: &[Vec<u8>]) -> Vec<u8> {
    let mut file = IDENTIFIER.to_vec();
    let header = [format, 1, width, height, 0, 0, 1, levels.len() as u32, 0];
    header.iter().for_each(|v| file.extend(v.to_le_bytes()));
    // No data format descriptor, key/value data or supercompression data.
    file.extend([0u8; 32]);
    let mut at = (80 + 24 * levels.len()) as u64;
    for level in levels {
        file.extend(at.to_le_bytes());
        file.extend((level.len() as u64).to_le_bytes());
        file.extend((level.len() as u64).to_le_bytes());
        at += level.len() as u64;
    }
    levels.iter().for_each(|level| file.extend(level));
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_rgba8_files_levels_are_read_where_they_are() {
        let levels = vec![vec![1; 4 * 4 * 4], vec![2; 2 * 2 * 4], vec![3; 4]];
        let file = write(4, 4, RGBA8_SRGB, &levels);
        let read = read(&file).unwrap();
        assert_eq!(
            (read.width, read.height, read.texels),
            (4, 4, Texels::Rgba8)
        );
        let found: Vec<Vec<u8>> = (0..read.level_count())
            .map(|l| read.level(&file, l).unwrap().into_owned())
            .collect();
        assert_eq!(found, levels);
    }

    #[test]
    fn a_block_files_levels_are_whole_blocks() {
        // BC7, 8 by 4: levels of 2x1, 1x1, 1x1, 1x1 blocks of 16 bytes.
        let levels = vec![vec![7; 32], vec![7; 16], vec![7; 16], vec![7; 16]];
        let file = write(8, 4, 146, &levels);
        let read = read(&file).unwrap();
        assert_eq!(
            read.texels,
            Texels::Blocks(wgpu::TextureFormat::Bc7RgbaUnormSrgb)
        );
        assert_eq!(read.level_count(), 4);
        // ASTC 6x6, sRGB: its format from Vulkan's numbering.
        let astc = write(12, 6, 166, &[vec![0; 32]]);
        assert_eq!(
            read_texels(&astc),
            Texels::Blocks(wgpu::TextureFormat::Astc {
                block: wgpu::AstcBlock::B6x6,
                channel: wgpu::AstcChannel::UnormSrgb
            })
        );
        // Not whole blocks: refused.
        assert!(
            super::read(&write(6, 4, 146, &[vec![7; 32]]))
                .unwrap_err()
                .contains("whole blocks")
        );
    }

    fn read_texels(file: &[u8]) -> Texels {
        read(file).unwrap().texels
    }

    #[test]
    fn a_zstd_level_is_decompressed() {
        // A Zstandard frame of one raw block holding the level's 64 bytes.
        let level: Vec<u8> = (0..64).collect();
        let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd];
        // Frame header: single segment, a 1-byte content size.
        frame.extend([0x20, 64]);
        // A last raw block of 64 bytes: (64 << 3) | (0 << 1) | 1.
        let header = (64u32 << 3) | 1;
        frame.extend(&header.to_le_bytes()[..3]);
        frame.extend(&level);
        let mut file = write(4, 4, RGBA8, &[frame.clone()]);
        // Supercompression 2, and the level's uncompressed length.
        file[44..48].copy_from_slice(&2u32.to_le_bytes());
        file[96..104].copy_from_slice(&64u64.to_le_bytes());
        let read = read(&file).unwrap();
        assert_eq!(read.level(&file, 0).unwrap().as_ref(), &level[..]);
        // A frame that doesn't decompress to its length: none.
        let mut cut = file.clone();
        let end = cut.len() - 1;
        cut[end] ^= 0xff;
        cut.truncate(end - 8);
        assert!(super::read(&cut).is_err() || super::read(&cut).unwrap().level(&cut, 0).is_none());
    }

    #[test]
    fn a_hostile_file_is_refused_not_read_past() {
        // Every length of a good file cut short, and its level index's
        // offsets and lengths made wrong in many ways: refused, or a level
        // that doesn't decompress, and never a panic or a read past it.
        let good = write(
            8,
            8,
            RGBA8,
            &[vec![1; 256], vec![2; 64], vec![3; 16], vec![4; 4]],
        );
        let zstd = {
            let mut file = good.clone();
            file[44..48].copy_from_slice(&2u32.to_le_bytes());
            file
        };
        for file in [&good, &zstd] {
            for end in 0..file.len() {
                if let Ok(read) = read(&file[..end]) {
                    (0..read.level_count()).for_each(|l| drop(read.level(&file[..end], l)));
                }
            }
            for level in 0..4 {
                for field in 0..3 {
                    for value in [0, 1, 15, 255, 1 << 31, u64::MAX >> 1, u64::MAX] {
                        let mut bad = file.clone();
                        let at = 80 + 24 * level + 8 * field;
                        bad[at..at + 8].copy_from_slice(&value.to_le_bytes());
                        if let Ok(read) = read(&bad) {
                            (0..read.level_count()).for_each(|l| drop(read.level(&bad, l)));
                        }
                    }
                }
            }
        }
        // Sizes past what any level may hold: refused before anything is
        // made for them.
        let mut huge = good.clone();
        huge[20..28].copy_from_slice(&[0, 32, 0, 0, 0, 32, 0, 0]);
        assert!(read(&huge).is_err());
    }

    #[cfg(feature = "ktx2-basis")]
    #[test]
    fn a_basis_texture_is_transcoded_to_what_its_device_holds() {
        use wgpu::Features as F;
        let both = F::TEXTURE_COMPRESSION_BC | F::TEXTURE_COMPRESSION_ASTC;
        assert_eq!(Target::for_device(both, (64, 64)).0, Target::Bc7);
        assert_eq!(
            Target::for_device(F::TEXTURE_COMPRESSION_ASTC, (64, 64)).0,
            Target::Astc
        );
        assert_eq!(Target::for_device(F::empty(), (64, 64)).0, Target::Rgba8);
        // Not whole 4 by 4 blocks: RGBA8, whatever the device holds.
        assert_eq!(Target::for_device(both, (300, 171)).0, Target::Rgba8);
    }

    #[test]
    fn what_isnt_read_says_why() {
        let file = write(4, 4, RGBA8, &[vec![1; 64]]);
        let mut unknown = file.clone();
        unknown[12..16].copy_from_slice(&1000u32.to_le_bytes());
        assert!(read(&unknown).unwrap_err().contains("format 1000"));
        #[cfg(not(feature = "ktx2-basis"))]
        {
            let mut basis = file.clone();
            basis[12..16].copy_from_slice(&0u32.to_le_bytes());
            assert!(read(&basis).unwrap_err().contains("Basis"));
        }
        let mut basislz = file.clone();
        basislz[44..48].copy_from_slice(&1u32.to_le_bytes());
        assert!(read(&basislz).unwrap_err().contains("supercompression 1"));
        assert!(read(&file[..file.len() - 1]).is_err(), "cut short");
        assert!(read(b"not a ktx2").is_err());
        // A level of the wrong size.
        assert!(read(&write(4, 4, RGBA8, &[vec![1; 60]])).is_err());
    }
}
