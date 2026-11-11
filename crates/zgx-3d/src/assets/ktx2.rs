//! KTX2 files (`Texture::decode`, the `ktx2` feature): read where they hold
//! RGBA8 texels, sRGB or not, with no supercompression, their levels taken
//! as they are (no decoding, and their smaller levels the file's own, not
//! made again). Block-compressed and Basis Universal payloads are refused,
//! saying so.

/// A KTX2 file's identifier, its first 12 bytes.
pub(crate) const IDENTIFIER: [u8; 12] = *b"\xabKTX 20\xbb\r\n\x1a\n";

/// Vulkan's `VK_FORMAT_R8G8B8A8_UNORM` and `_SRGB`.
const RGBA8: u32 = 37;
const RGBA8_SRGB: u32 = 43;

/// What a KTX2 file holds that a texture reads: its size and where each of
/// its levels is in it, the first the largest.
#[derive(Debug, PartialEq)]
pub(crate) struct Ktx2 {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) levels: Vec<std::ops::Range<usize>>,
}

/// `bytes` read as a KTX2 file, or why they can't be.
pub(crate) fn read(bytes: &[u8]) -> Result<Ktx2, String> {
    let u32_at = |at: usize| -> Result<u32, String> {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| "a KTX2 file cut short".to_owned())
    };
    let u64_at = |at: usize| -> Result<u64, String> {
        bytes
            .get(at..at + 8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| "a KTX2 file cut short".to_owned())
    };
    if !bytes.starts_with(&IDENTIFIER) {
        return Err("not a KTX2 file".into());
    }
    let format = u32_at(12)?;
    let (width, height, depth) = (u32_at(20)?, u32_at(24)?, u32_at(28)?);
    let (layers, faces, count, supercompression) =
        (u32_at(32)?, u32_at(36)?, u32_at(40)?, u32_at(44)?);
    match format {
        RGBA8 | RGBA8_SRGB => {}
        0 => return Err("a Basis Universal KTX2 texture: not read yet".into()),
        other => {
            return Err(format!(
                "a KTX2 texture of Vulkan format {other}: only RGBA8 ones are read yet"
            ));
        }
    }
    if supercompression != 0 {
        return Err("a supercompressed KTX2 texture: not read yet".into());
    }
    if depth > 1 || layers > 1 || faces != 1 {
        return Err("a KTX2 texture of more than one 2D image: not read".into());
    }
    if width == 0
        || height == 0
        || width > crate::texture::MAX_SIDE
        || height > crate::texture::MAX_SIDE
    {
        return Err("a texture is 1 to 8192 pixels on a side".into());
    }
    // No levels given: the first alone, the rest made as for any image.
    let count = count.max(1);
    if count > 32 - width.max(height).leading_zeros() {
        return Err("a KTX2 texture with more levels than its size has".into());
    }
    // The level index, after the 48-byte header and the 32-byte index of
    // its other sections.
    let mut levels = Vec::with_capacity(count as usize);
    for level in 0..count {
        let at = 80 + 24 * level as usize;
        let (offset, length) = (u64_at(at)?, u64_at(at + 8)?);
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        if length != u64::from(w) * u64::from(h) * 4 {
            return Err(format!(
                "a KTX2 texture's level {level} isn't {w} by {h} RGBA8 texels"
            ));
        }
        let start = usize::try_from(offset).map_err(|_| "a KTX2 level past the file")?;
        let end = start
            .checked_add(length as usize)
            .filter(|&end| end <= bytes.len())
            .ok_or("a KTX2 level past the file")?;
        levels.push(start..end);
    }
    Ok(Ktx2 {
        width,
        height,
        levels,
    })
}

/// A KTX2 file of RGBA8 `levels` (each half the one before, the first
/// `width` by `height`), sRGB or not: for tests, and for tools writing one.
#[cfg(test)]
pub(crate) fn write(width: u32, height: u32, srgb: bool, levels: &[Vec<u8>]) -> Vec<u8> {
    let mut file = IDENTIFIER.to_vec();
    let header = [
        if srgb { RGBA8_SRGB } else { RGBA8 },
        1,
        width,
        height,
        0,
        0,
        1,
        levels.len() as u32,
        0,
    ];
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
        let file = write(4, 4, true, &levels);
        let read = read(&file).unwrap();
        assert_eq!((read.width, read.height), (4, 4));
        let found: Vec<&[u8]> = read.levels.iter().map(|r| &file[r.clone()]).collect();
        assert_eq!(found, [&levels[0][..], &levels[1][..], &levels[2][..]]);
    }

    #[test]
    fn what_isnt_read_says_why() {
        let file = write(4, 4, false, &[vec![1; 64]]);
        let mut bc7 = file.clone();
        bc7[12..16].copy_from_slice(&145u32.to_le_bytes());
        assert!(read(&bc7).unwrap_err().contains("format 145"));
        let mut basis = file.clone();
        basis[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(read(&basis).unwrap_err().contains("Basis"));
        let mut zstd = file.clone();
        zstd[44..48].copy_from_slice(&2u32.to_le_bytes());
        assert!(read(&zstd).unwrap_err().contains("supercompressed"));
        assert!(read(&file[..file.len() - 1]).is_err(), "cut short");
        assert!(read(b"not a ktx2").is_err());
        // A level of the wrong size.
        let wrong = write(4, 4, false, &[vec![1; 60]]);
        assert!(read(&wrong).is_err());
    }
}
