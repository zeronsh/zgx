//! Textures: images a mesh's colour is multiplied by across its texture
//! coordinates (`World::set_texture`).
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

/// An image (sRGB, straight alpha) a mesh's colour is multiplied by across
/// its texture coordinates, repeating past 0 to 1. Shared by clones (the
/// same texture), immutable, uploaded once per renderer with its mipmaps.
///
/// One read from a PNG or JPEG (`decode`) keeps that file's bytes, not its
/// pixels: they're decoded as a renderer uploads it, and go once it has,
/// decoded again only if another upload needs them (a lost device, another
/// renderer's). One made from pixels (`from_rgba8`) keeps them.
#[derive(Clone)]
pub struct Texture(Arc<Data>);
struct Data {
    id: u64,
    width: u32,
    height: u32,
    /// Its pixels, while held (see `Texture`): the decoder's own, shared,
    /// not a copy.
    pixels: Mutex<Option<Arc<zgx::image::ImageData>>>,
    /// The PNG or JPEG it was read from, if it was.
    encoded: Option<Vec<u8>>,
    /// Its file failed to decode at an upload: it draws as if it had none,
    /// and isn't tried again.
    failed: std::sync::atomic::AtomicBool,
}
impl std::fmt::Debug for Texture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Texture")
            .field("width", &self.0.width)
            .field("height", &self.0.height)
            .finish()
    }
}
impl PartialEq for Texture {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}
/// What a mesh's texture gives it (`World::set_map`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Map {
    /// Its colour, multiplied by the material's (sRGB, its alpha the
    /// opacity of a translucent mesh).
    BaseColour,
    /// How metal (blue) and how rough (green) each point is, multiplying
    /// the material's metallic and roughness (glTF's metallicRoughness).
    MetalRoughness,
    /// Which way each point faces, in the surface's own frame (red right,
    /// green up the texture, blue out; glTF's and OpenGL's convention).
    Normal,
    /// The light it gives off, multiplying the material's emission (sRGB).
    Emission,
    /// How much of its surroundings' light reaches each point (red: 1 all,
    /// 0 none), dimming the hemisphere's ambient light, not the sun's or a
    /// bulb's (glTF's occlusion).
    Occlusion,
    /// How much of its light passes through each point (red), multiplying
    /// its transmission's factor (`World::set_transmission`; glTF's
    /// transmission texture). Read at its first texture coordinates as
    /// they are (never placed), and only while light passes through it.
    Transmission,
    /// How thick it is at each point (green), multiplying its
    /// transmission's thickness (glTF's volume thickness texture). Read as
    /// `Transmission` is.
    Thickness,
    /// How much of its clear coat there is at each point (red), multiplying
    /// the coat's factor (`World::set_clearcoat`; glTF's clearcoat
    /// texture). Read as `Transmission` is, and only while it has a coat.
    Clearcoat,
    /// How rough its clear coat is at each point (green), multiplying the
    /// coat's roughness. Read as `Clearcoat` is.
    ClearcoatRoughness,
    /// Which way its clear coat faces at each point, as `Normal` says for
    /// the surface under it (each its own). Read as `Clearcoat` is.
    ClearcoatNormal,
}
/// How many maps a mesh may have, but for those of its layers.
pub(crate) const MAPS: usize = 5;
/// How many maps a mesh's layers (light passing through, a clear coat) may
/// have: `Transmission`'s on.
pub(crate) const LAYER_MAPS: usize = 5;
impl Map {
    /// A map of a mesh's layers: its place in `Extras::layer_maps`. (They
    /// follow `ALL`'s: one comparison.)
    #[inline(always)]
    pub(crate) fn layer(self) -> Option<usize> {
        (self as usize).checked_sub(MAPS)
    }
    /// Every map, in the order a frame binds them.
    pub(crate) const ALL: [Map; MAPS] = [
        Map::BaseColour,
        Map::MetalRoughness,
        Map::Normal,
        Map::Emission,
        Map::Occlusion,
    ];
    /// Whether its texels are colours (sRGB) rather than data.
    pub(crate) fn srgb(self) -> bool {
        matches!(self, Map::BaseColour | Map::Emission)
    }
}

/// Of this many pixels or fewer, a texture's file is decoded on the thread
/// that uploads it: under a millisecond, less than a thread's start.
pub(crate) const SMALL: u32 = 128 * 128;
/// The most a texture's file, or its pixels, may hold (as zgx-gpu's
/// decoder allows).
#[cfg(feature = "gltf")]
const MAX_FILE: usize = 64 << 20;
/// A PNG's or a baseline or progressive JPEG's width and height, from its
/// header (`None`: neither, or no size found).
#[cfg(feature = "gltf")]
fn size_of_file(bytes: &[u8]) -> Option<(u32, u32)> {
    let be16 = |at: usize| {
        Some(u32::from(u16::from_be_bytes(
            bytes.get(at..at + 2)?.try_into().ok()?,
        )))
    };
    let be32 = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    // A PNG: its signature, then the IHDR chunk's width and height.
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return (bytes.get(12..16)? == b"IHDR").then(|| Some((be32(16)?, be32(20)?)))?;
    }
    // A JPEG: segments after its start, to the first frame's.
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut at = 2;
    loop {
        // Fill bytes before a marker.
        while *bytes.get(at)? == 0xff && *bytes.get(at + 1)? == 0xff {
            at += 1;
        }
        if *bytes.get(at)? != 0xff {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        match marker {
            // Markers with no length.
            0x01 | 0xd0..=0xd9 => at += 2,
            // A frame's start (not a table's or an arithmetic coding's):
            // precision, then height and width.
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                return Some((be16(at + 7)?, be16(at + 5)?));
            }
            _ => at += 2 + be16(at + 2)? as usize,
        }
    }
}
/// The longest side a texture may have.
pub(crate) const MAX_SIDE: u32 = 8192;
impl Texture {
    /// `width` by `height` pixels, four bytes each (red, green, blue, alpha;
    /// sRGB, straight alpha), rows top to bottom. Fails on an empty or
    /// oversized image or a pixel count that doesn't match.
    pub fn from_rgba8(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, &'static str> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err("a texture is 1 to 8192 pixels on a side");
        }
        if pixels.len() != width as usize * height as usize * 4 {
            return Err("a texture's pixels are 4 bytes each, width by height");
        }
        let image = zgx::image::ImageData::new(width, height, pixels)?;
        Ok(Self::made((width, height), Some(Arc::new(image)), None))
    }
    /// A PNG's or JPEG's bytes (as zgx-gpu's `png` and `jpeg` features
    /// allow), their header read: its pixels are decoded as a renderer
    /// uploads it, one texture at a time, and go once it has, so a model of
    /// many textures never holds them all decoded at once. Fails on a file
    /// whose header isn't an image's, or too large; a file whose pixels then
    /// fail to decode draws as if it had none. (Another format, a GIF,
    /// decodes here.)
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        Self::decode_owned(bytes.to_vec())
    }
    /// `decode`, keeping `bytes` themselves (a model's file, read for it):
    /// no copy of them.
    pub(crate) fn decode_owned(bytes: Vec<u8>) -> Result<Self, String> {
        let too_large = || "a texture is 1 to 8192 pixels on a side".to_owned();
        // A KTX2 file (the `ktx2` feature): its levels read as they are, on
        // upload; refused here, saying why, if it holds what isn't read.
        #[cfg(feature = "ktx2")]
        if bytes.starts_with(&crate::assets::ktx2::IDENTIFIER) {
            let file = crate::assets::ktx2::read(&bytes)?;
            return Ok(Self::made((file.width, file.height), None, Some(bytes)));
        }
        #[cfg(feature = "gltf")]
        if let Some((width, height)) = size_of_file(&bytes) {
            // PNG and JPEG, which the `gltf` feature decodes: read as they
            // are, their headers' size checked as `decode_image_kept` does.
            if width == 0 || width > MAX_SIDE || height == 0 || height > MAX_SIDE {
                return Err(too_large());
            }
            if bytes.len() > MAX_FILE || width as usize * height as usize * 4 > MAX_FILE {
                return Err("a texture's file or pixels exceed 64 MiB".into());
            }
            return Ok(Self::made((width, height), None, Some(bytes)));
        }
        let header = zgx_gpu::assets::decode_image_kept(bytes.as_slice().into())
            .map_err(|e| e.to_string())?;
        if header.width() > MAX_SIDE || header.height() > MAX_SIDE {
            return Err(too_large());
        }
        let (width, height) = (header.width(), header.height());
        // Decoded now (a format it doesn't keep), its pixels are held.
        let pixels = (!header.keeps_file()).then_some(header);
        Ok(Self::made((width, height), pixels, Some(bytes)))
    }
    fn made(
        (width, height): (u32, u32),
        pixels: Option<Arc<zgx::image::ImageData>>,
        encoded: Option<Vec<u8>>,
    ) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(Arc::new(Data {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            pixels: Mutex::new(pixels),
            encoded,
            failed: Default::default(),
        }))
    }
    /// Its width in pixels.
    pub fn width(&self) -> u32 {
        self.0.width
    }
    /// Its height in pixels.
    pub fn height(&self) -> u32 {
        self.0.height
    }
    /// Stable for its lifetime: renderers cache its upload by it.
    pub fn id(&self) -> u64 {
        self.0.id
    }
    /// Its pixels: held, or decoded from its file. `None` if that file
    /// doesn't decode to its header's size (remembered: not tried again).
    pub(crate) fn pixels(&self) -> Option<Arc<zgx::image::ImageData>> {
        if let Some(pixels) = self.held().clone() {
            return Some(pixels);
        }
        if self.0.failed.load(Ordering::Relaxed) {
            return None;
        }
        let encoded = self.0.encoded.as_ref()?;
        // A KTX2 file's first level, as it is, where it's RGBA8 (blocks
        // have no pixels here: they go to the GPU as blocks, `levels`).
        #[cfg(feature = "ktx2")]
        if encoded.starts_with(&crate::assets::ktx2::IDENTIFIER) {
            use crate::assets::ktx2;
            let first = ktx2::read(encoded)
                .ok()
                .filter(|file| file.texels == ktx2::Texels::Rgba8)
                .and_then(|file| Some(file.level(encoded, 0)?.into_owned()));
            let image = first.and_then(|pixels| {
                zgx::image::ImageData::new(self.0.width, self.0.height, pixels).ok()
            });
            if image.is_none() {
                self.0.failed.store(true, Ordering::Relaxed);
            }
            return image.map(Arc::new);
        }
        let image = zgx_gpu::assets::decode_image(encoded).ok();
        let image = image.filter(|i| (i.width(), i.height()) == (self.0.width, self.0.height));
        if image.is_none() {
            self.0.failed.store(true, Ordering::Relaxed);
        }
        image
    }
    /// Its levels, ready to upload: a file's own where it has them (a
    /// KTX2's, as they're stored, or decompressed: no decoding, and its
    /// smaller levels as the file made them), else its pixels and the levels
    /// below them made here (`mipmaps`). `None` if they don't decode.
    pub(crate) fn levels(&self) -> Option<Levels<'_>> {
        if let Some(kept) = self.kept_levels() {
            return Some(kept);
        }
        let image = self.pixels()?;
        let smaller = mipmaps(self.0.width, self.0.height, image.pixels());
        Some(Levels::Made(image, smaller))
    }
    /// Its file's own levels (a KTX2 file's, the `ktx2` feature): its
    /// blocks, or its RGBA8 texels where it has more than one level (one
    /// level of texels has the rest made, `levels`). `None` where it has
    /// none, or they don't decompress.
    pub(crate) fn kept_levels(&self) -> Option<Levels<'_>> {
        #[cfg(feature = "ktx2")]
        if let Some(encoded) = self.0.encoded.as_ref()
            && encoded.starts_with(&crate::assets::ktx2::IDENTIFIER)
            && let Ok(file) = crate::assets::ktx2::read(encoded)
        {
            use crate::assets::ktx2::Texels;
            let levels = || -> Option<Vec<_>> {
                (0..file.level_count())
                    .map(|level| file.level(encoded, level))
                    .collect()
            };
            return match file.texels {
                Texels::Blocks(format) => match levels() {
                    Some(levels) => Some(Levels::Blocks(format, levels)),
                    None => {
                        self.failed_to_decode();
                        None
                    }
                },
                Texels::Rgba8 if file.level_count() > 1 => levels().map(Levels::Kept),
                Texels::Rgba8 => None,
            };
        }
        None
    }
    /// Whether its file failed to decode (`pixels`): it draws none.
    pub(crate) fn failed(&self) -> bool {
        self.0.failed.load(Ordering::Relaxed)
    }
    /// Its file's decoder failed out of `pixels` (it panicked): it draws
    /// none, and isn't tried again.
    pub(crate) fn failed_to_decode(&self) {
        self.0.failed.store(true, Ordering::Relaxed);
    }
    /// A renderer has it: its pixels go if its file can bring them back.
    pub(crate) fn uploaded(&self) {
        if self.0.encoded.is_some() {
            *self.held() = None;
        }
    }
    /// Whether it holds its pixels now: given them, or a format decoded as
    /// it's read and not yet uploaded.
    pub(crate) fn holds_pixels(&self) -> bool {
        self.held().is_some()
    }
    fn held(&self) -> std::sync::MutexGuard<'_, Option<Arc<zgx::image::ImageData>>> {
        self.0.pixels.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A texture's levels, the first the largest (`Texture::levels`).
pub(crate) enum Levels<'a> {
    /// A file's own RGBA8 texels, as stored in it (or decompressed).
    #[cfg_attr(not(feature = "ktx2"), allow(dead_code))]
    Kept(Vec<std::borrow::Cow<'a, [u8]>>),
    /// A file's own blocks of this format (its sRGB-ness as the file says).
    #[cfg_attr(not(feature = "ktx2"), allow(dead_code))]
    Blocks(
        zgx_gpu::wgpu::TextureFormat,
        Vec<std::borrow::Cow<'a, [u8]>>,
    ),
    /// Its pixels, and the levels below made from them.
    Made(Arc<zgx::image::ImageData>, Vec<(u32, u32, Vec<u8>)>),
}
impl Levels<'_> {
    /// Each level's bytes, the first the largest.
    pub(crate) fn bytes(&self) -> Vec<&[u8]> {
        match self {
            Levels::Kept(levels) | Levels::Blocks(_, levels) => {
                levels.iter().map(AsRef::as_ref).collect()
            }
            Levels::Made(image, smaller) => std::iter::once(image.pixels())
                .chain(smaller.iter().map(|(_, _, pixels)| pixels.as_slice()))
                .collect(),
        }
    }
}

/// Each half-size level below `pixels` (`width` by `height`, RGBA8), down
/// to 1 by 1 (`halve`). `pixels` itself is the first level, not copied.
pub(crate) fn mipmaps(width: u32, height: u32, pixels: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    loop {
        let (w, h, above) = match levels.last() {
            Some((w, h, above)) => (*w, *h, above.as_slice()),
            None => (width, height, pixels),
        };
        if w <= 1 && h <= 1 {
            break;
        }
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0_u8; nw as usize * nh as usize * 4];
        halve((w, h), above, &mut next);
        levels.push((nw, nh, next));
    }
    levels
}
/// The level below `above` (`w` by `h`, RGBA8) into `next`, half as large
/// (at least 1): each texel the mean of the up to 2 by 2 above it, its
/// edges' repeated where `above` is odd.
pub(crate) fn halve((w, h): (u32, u32), above: &[u8], next: &mut [u8]) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    for y in 0..nh {
        for x in 0..nw {
            let mut sum = [0_u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
                let at = (sy * w + sx) as usize * 4;
                for (total, &byte) in sum.iter_mut().zip(&above[at..at + 4]) {
                    *total += u32::from(byte);
                }
            }
            let at = (y * nw + x) as usize * 4;
            for (out, total) in next[at..at + 4].iter_mut().zip(sum) {
                *out = ((total + 2) / 4) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 by 1 PNG: red, then blue.
    #[cfg(feature = "gltf")]
    const RED_BLUE: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 244, 34, 127, 138, 0, 0, 0, 14, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0,
        66, 255, 1, 15, 249, 3, 253, 133, 17, 153, 118, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
        130,
    ];

    #[cfg(feature = "gltf")]
    #[test]
    fn a_texture_read_from_a_file_decodes_its_pixels_only_as_they_are_uploaded() {
        let texture = Texture::decode(RED_BLUE).unwrap();
        assert_eq!((texture.width(), texture.height()), (2, 1), "its header's");
        assert!(!texture.holds_pixels(), "nothing decoded yet");
        // Decoded as an upload asks, the same each time, never kept.
        let pixels = [255, 0, 0, 255, 0, 0, 255, 255];
        assert_eq!(texture.pixels().unwrap().pixels(), pixels);
        assert_eq!(texture.pixels().unwrap().pixels(), pixels);
        texture.uploaded();
        assert!(!texture.holds_pixels(), "and not kept");
    }

    #[cfg(feature = "gltf")]
    #[test]
    fn a_file_whose_pixels_dont_decode_draws_none_and_isnt_tried_again() {
        // Its header whole, its image data cut short.
        let cut = &RED_BLUE[..RED_BLUE.len() - 20];
        let texture = Texture::decode(cut).unwrap();
        assert_eq!((texture.width(), texture.height()), (2, 1));
        assert!(texture.pixels().is_none());
        assert!(texture.0.failed.load(Ordering::Relaxed), "remembered");
        assert!(texture.pixels().is_none());
        // A file that isn't an image fails as it's read.
        assert!(Texture::decode(b"not an image").is_err());
    }

    #[cfg(feature = "gltf")]
    #[test]
    fn a_files_size_is_read_from_its_header() {
        assert_eq!(size_of_file(RED_BLUE), Some((2, 1)));
        // A JPEG: a table and an application segment before its frame's
        // start, fill bytes and a restart marker on the way; its frame says
        // 48 high, 640 wide.
        let jpeg = [
            &[0xff, 0xd8][..],
            &[0xff, 0xe0, 0, 4, 1, 2],
            &[0xff, 0xff, 0xd0],
            &[0xff, 0xc4, 0, 3, 9],
            &[0xff, 0xc2, 0, 11, 8, 0, 48, 2, 128, 3, 1, 0x11, 0],
        ]
        .concat();
        assert_eq!(size_of_file(&jpeg), Some((640, 48)));
        // Neither, or cut short: no size (another path reads it, or fails).
        assert_eq!(size_of_file(b"GIF89a"), None);
        assert_eq!(size_of_file(&RED_BLUE[..20]), None);
        assert_eq!(size_of_file(&jpeg[..jpeg.len() - 6]), None);
        // As a texture: its size, its file kept, nothing decoded.
        let texture = Texture::decode_owned(RED_BLUE.to_vec()).unwrap();
        assert_eq!((texture.width(), texture.height()), (2, 1));
        assert!(!texture.holds_pixels());
    }

    #[test]
    fn a_texture_keeps_pixels_it_was_given() {
        // They're all it has.
        let made = Texture::from_rgba8(2, 1, vec![255; 8]).unwrap();
        made.uploaded();
        assert!(made.holds_pixels());
    }
}

/// How a texture repeats past its edges (`MapPlacement::wrap`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Wrap {
    /// Over again.
    #[default]
    Repeat,
    /// Its edge's colour on.
    Clamp,
    /// Over again, every other time flipped.
    Mirror,
}

/// Where a map sits on a mesh (`World::set_map_placement`): which of its
/// texture coordinates it reads, moved how, and how it repeats. Unset, it
/// reads its first set as they are, repeating.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapPlacement {
    pub(crate) offset: [f32; 2],
    pub(crate) rotation: f32,
    pub(crate) scale: [f32; 2],
    pub(crate) uv_set: u8,
    pub(crate) wrap: [Wrap; 2],
}
impl Default for MapPlacement {
    fn default() -> Self {
        Self {
            offset: [0.; 2],
            rotation: 0.,
            scale: [1.; 2],
            uv_set: 0,
            wrap: [Wrap::Repeat; 2],
        }
    }
}
impl MapPlacement {
    /// As it is: the first texture coordinates, repeating.
    pub fn new() -> Self {
        Self::default()
    }
    /// It read `u`, `v` further on.
    pub fn offset(mut self, u: f32, v: f32) -> Self {
        self.offset = [u, v];
        self
    }
    /// It read turned `radians`, as glTF's `KHR_texture_transform` turns it.
    pub fn rotated(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }
    /// It read at `u`, `v` times its size in coordinates (2 repeats it twice).
    pub fn scaled(mut self, u: f32, v: f32) -> Self {
        self.scale = [u, v];
        self
    }
    /// It read from the geometry's second texture coordinates
    /// (`Geometry::with_second_uvs`) if `set` is 1, else its first.
    pub fn uv_set(mut self, set: u8) -> Self {
        self.uv_set = set;
        self
    }
    /// How it repeats along `u` and `v`. A mesh's maps share one sampler:
    /// its first map's wrap holds for all of them, its base colour's if it
    /// has one.
    pub fn wrap(mut self, u: Wrap, v: Wrap) -> Self {
        self.wrap = [u, v];
        self
    }
    /// Its two rows mapping coordinates: `u' = a u + b v + c`, the row's
    /// fourth its set.
    pub(crate) fn rows(&self) -> [[f32; 4]; 2] {
        let (sin, cos) = self.rotation.sin_cos();
        let [su, sv] = self.scale;
        let [ou, ov] = self.offset;
        let set = f32::from(u8::from(self.uv_set == 1));
        [[su * cos, sv * sin, ou, set], [-su * sin, sv * cos, ov, 0.]]
    }
    /// Whether it only reads its first coordinates as they are (wrapping
    /// takes a sampler, no more).
    pub(crate) fn in_place(&self) -> bool {
        self.offset == [0.; 2] && self.rotation == 0. && self.scale == [1.; 2] && self.uv_set != 1
    }
    /// All it says, as numbers: meshes alike share a binding by these.
    pub(crate) fn numbers(&self) -> [u32; 8] {
        let [ou, ov] = self.offset.map(f32::to_bits);
        let [su, sv] = self.scale.map(f32::to_bits);
        let [wu, wv] = self.wrap.map(|wrap| wrap as u32);
        let set = u32::from(self.uv_set == 1);
        [ou, ov, self.rotation.to_bits(), su, sv, set, wu, wv]
    }
}
