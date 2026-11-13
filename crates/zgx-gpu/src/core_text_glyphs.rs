//! Glyphs swash can't draw, drawn by Core Text (macOS): faces whose shapes
//! are only in Apple's `hvgl` table, as macOS 26's PingFang keeps them
//! (zgx-cosmic-text's `Font::needs_platform_outlines`). The glyph comes out
//! as swash's would: an alpha mask at the cache key's size and subpixel
//! offset, placed from the pen position, for the atlas to take like any
//! other. Text in faces swash draws never comes here.
use cosmic_text::{CacheKey, CacheKeyFlags, FontSystem, Placement, SwashContent, SwashImage};
use std::{cell::RefCell, collections::HashMap, ffi::c_void};

type CFTypeRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
#[repr(C)]
struct CGAffineTransform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataCreate(allocator: CFTypeRef, bytes: *const u8, length: isize) -> CFTypeRef;
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    fn CFStringGetCString(string: CFTypeRef, buffer: *mut u8, size: isize, encoding: u32) -> bool;
    fn CFRetain(object: CFTypeRef) -> CFTypeRef;
    fn CFNumberCreate(allocator: CFTypeRef, kind: isize, value: *const c_void) -> CFTypeRef;
    fn CFStringCreateWithBytes(
        allocator: CFTypeRef,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: bool,
    ) -> CFTypeRef;
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: CFTypeRef,
        path: *const u8,
        length: isize,
        directory: bool,
    ) -> CFTypeRef;
    fn CFRelease(object: CFTypeRef);
}
#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontManagerCreateFontDescriptorsFromData(data: CFTypeRef) -> CFTypeRef;
    fn CTFontManagerCreateFontDescriptorsFromURL(url: CFTypeRef) -> CFTypeRef;
    fn CTFontDescriptorCreateWithNameAndSize(name: CFTypeRef, size: f64) -> CFTypeRef;
    fn CTFontDescriptorCreateCopyWithVariation(
        original: CFTypeRef,
        axis: CFTypeRef,
        value: f64,
    ) -> CFTypeRef;
    fn CTFontCreateWithFontDescriptor(
        descriptor: CFTypeRef,
        size: f64,
        matrix: *const CGAffineTransform,
    ) -> CFTypeRef;
    fn CTFontCopyPostScriptName(font: CFTypeRef) -> CFTypeRef;
    fn CTFontGetBoundingRectsForGlyphs(
        font: CFTypeRef,
        orientation: u32,
        glyphs: *const u16,
        rects: *mut CGRect,
        count: isize,
    ) -> CGRect;
    fn CTFontDrawGlyphs(
        font: CFTypeRef,
        glyphs: *const u16,
        positions: *const CGPoint,
        count: usize,
        context: CFTypeRef,
    );
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CFTypeRef,
        bitmap_info: u32,
    ) -> CFTypeRef;
    fn CGContextSetGrayFillColor(context: CFTypeRef, gray: f64, alpha: f64);
    fn CGColorSpaceCreateDeviceRGB() -> CFTypeRef;
    fn CGContextFillRect(context: CFTypeRef, rect: CGRect);
    fn CGContextScaleCTM(context: CFTypeRef, sx: f64, sy: f64);
    fn CGContextSetRGBFillColor(context: CFTypeRef, r: f64, g: f64, b: f64, a: f64);
    fn CGContextSetShouldAntialias(context: CFTypeRef, on: bool);
    fn CGContextSetTextDrawingMode(context: CFTypeRef, mode: i32);
    fn CGContextSetLineWidth(context: CFTypeRef, width: f64);
    fn CGContextSetRGBStrokeColor(context: CFTypeRef, r: f64, g: f64, b: f64, a: f64);
    fn CGContextSetShouldSmoothFonts(context: CFTypeRef, on: bool);
    fn CGContextSetAllowsFontSubpixelPositioning(context: CFTypeRef, on: bool);
    fn CGContextSetShouldSubpixelPositionFonts(context: CFTypeRef, on: bool);
    fn CGContextSetAllowsFontSubpixelQuantization(context: CFTypeRef, on: bool);
    fn CGContextSetShouldSubpixelQuantizeFonts(context: CFTypeRef, on: bool);
}
/// `kCGImageAlphaOnly`: a context of one alpha byte a pixel.
const ALPHA_ONLY: u32 = 7;
/// `kCTFontOrientationHorizontal`.
const HORIZONTAL: u32 = 1;
/// `kCFStringEncodingUTF8`.
const UTF8: u32 = 0x0800_0100;
/// `kCFNumberSInt32Type`.
const SINT32: isize = 3;
/// The weight axis's tag, as Core Text identifies axes.
const WGHT: i32 = i32::from_be_bytes(*b"wght");
/// Core Text fonts kept, by face, size, slant and weight: each glyph a size draws
/// alike, so a run of them makes one font. A zoom gives sizes enough to
/// clear it now and then.
const FONTS: usize = 64;

/// A Core Foundation object this holds a reference to.
struct Owned(CFTypeRef);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: made by a Create or Copy function (or retained), released
        // once.
        unsafe { CFRelease(self.0) }
    }
}

#[derive(Default)]
struct Cache {
    /// Each face's descriptor (none: Core Text can't read it).
    faces: HashMap<cosmic_text::fontdb::ID, Option<Owned>>,
    /// Fonts made from them.
    fonts: HashMap<(cosmic_text::fontdb::ID, u32, bool, u16), Owned>,
}
thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// `key`'s glyph through Core Text, if its face's shapes are where only
/// the platform draws them (`Font::needs_platform_outlines`).
pub(crate) fn glyph_image(fonts: &mut FontSystem, key: CacheKey) -> Option<SwashImage> {
    let font = fonts.get_font(key.font_id, key.font_weight)?;
    if !font.needs_platform_outlines() {
        return None;
    }
    draw(fonts, key)
}

/// `key`'s glyph through Core Text whatever its face; for tests to set
/// against swash's.
#[doc(hidden)]
pub fn core_text_glyph(fonts: &mut FontSystem, key: CacheKey) -> Option<SwashImage> {
    draw(fonts, key)
}

/// The descriptor of face `id`, found by its PostScript name: the system's
/// face of that name (macOS's own fonts, PingFang among them, are Core
/// Text's already), else one of its file's faces, else one of the faces Core
/// Text reads from its data. (Core Text reads no face from PingFang's data,
/// though it does from its file.)
fn descriptor(fonts: &mut FontSystem, id: cosmic_text::fontdb::ID) -> Option<Owned> {
    let font = fonts.get_font(id, cosmic_text::fontdb::Weight::NORMAL)?;
    let face = fonts.db().face(id)?;
    let name = face.post_script_name.clone();
    let path = match &face.source {
        cosmic_text::fontdb::Source::File(path)
        | cosmic_text::fontdb::Source::SharedFile(path, _) => Some(path.clone()),
        cosmic_text::fontdb::Source::Binary(_) => None,
    };
    // SAFETY: each object made is owned (`Owned`), or borrowed from an
    // array while the array lives; the bytes passed outlive each call.
    unsafe {
        let cf_name = Owned(CFStringCreateWithBytes(
            std::ptr::null(),
            name.as_ptr(),
            name.len() as isize,
            UTF8,
            false,
        ));
        if cf_name.0.is_null() {
            return None;
        }
        // Core Text gives another font for a name it doesn't know: kept only
        // if its name is the face's.
        // (A hidden system face, `.SFNS-Regular`, isn't found by name: Core Text
        // logs that it gives Times, so its file is read instead.)
        if !name.starts_with('.') {
            let by_name = Owned(CTFontDescriptorCreateWithNameAndSize(cf_name.0, 0.));
            if !by_name.0.is_null() && named(by_name.0, &name) {
                return Some(Owned(CFRetain(by_name.0)));
            }
        }
        let all = match path.as_deref().and_then(|path| path.to_str()) {
            Some(path) => CTFontManagerCreateFontDescriptorsFromURL(
                Owned(CFURLCreateFromFileSystemRepresentation(
                    std::ptr::null(),
                    path.as_ptr(),
                    path.len() as isize,
                    false,
                ))
                .0,
            ),
            None => {
                let data = font.data();
                let bytes = Owned(CFDataCreate(
                    std::ptr::null(),
                    data.as_ptr(),
                    data.len() as isize,
                ));
                CTFontManagerCreateFontDescriptorsFromData(bytes.0)
            }
        };
        if all.is_null() {
            return None;
        }
        let all = Owned(all);
        (0..CFArrayGetCount(all.0))
            .map(|i| CFArrayGetValueAtIndex(all.0, i))
            .find(|candidate| named(*candidate, &name))
            .map(|found| Owned(CFRetain(found)))
    }
}

/// Whether a font of `descriptor` has PostScript name `name`.
///
/// # Safety
/// `descriptor` is a live `CTFontDescriptor`.
unsafe fn named(descriptor: CFTypeRef, name: &str) -> bool {
    // SAFETY: the font and its name are owned and released here.
    unsafe {
        let probe = CTFontCreateWithFontDescriptor(descriptor, 12., std::ptr::null());
        if probe.is_null() {
            return false;
        }
        let probe = Owned(probe);
        let found = CTFontCopyPostScriptName(probe.0);
        if found.is_null() {
            return false;
        }
        let found = Owned(found);
        let mut buffer = [0u8; 256];
        if !CFStringGetCString(found.0, buffer.as_mut_ptr(), buffer.len() as isize, UTF8) {
            return false;
        }
        let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
        buffer[..end] == *name.as_bytes()
    }
}

/// The Core Text font for `key`: its face at its size, slanted as swash
/// slants a synthetic italic (14 degrees).
fn font_for(cache: &mut Cache, fonts: &mut FontSystem, key: CacheKey) -> Option<CFTypeRef> {
    let italic = key.flags.contains(CacheKeyFlags::FAKE_ITALIC);
    let slot = (key.font_id, key.font_size_bits, italic, key.font_weight.0);
    if let Some(font) = cache.fonts.get(&slot) {
        return Some(font.0);
    }
    let descriptor = cache
        .faces
        .entry(key.font_id)
        .or_insert_with(|| descriptor(fonts, key.font_id))
        .as_ref()?;
    let skew = CGAffineTransform {
        a: 1.,
        b: 0.,
        c: if italic { 14f64.to_radians().tan() } else { 0. },
        d: 1.,
        tx: 0.,
        ty: 0.,
    };
    let size = f64::from(f32::from_bits(key.font_size_bits));
    // SAFETY: the descriptor lives in the cache; what is made is owned.
    let font = unsafe {
        // At the weight it was shaped at, as swash sets `wght` (PingFang is
        // one variable face, Medium by default, Regular at 400): Core Text
        // leaves a face without the axis as it is.
        let tag = WGHT;
        let axis = Owned(CFNumberCreate(
            std::ptr::null(),
            SINT32,
            (&raw const tag).cast(),
        ));
        let weighted = Owned(CTFontDescriptorCreateCopyWithVariation(
            descriptor.0,
            axis.0,
            f64::from(key.font_weight.0),
        ));
        let at = match weighted.0.is_null() {
            true => descriptor.0,
            false => weighted.0,
        };
        CTFontCreateWithFontDescriptor(at, size, &skew)
    };
    if font.is_null() {
        return None;
    }
    if cache.fonts.len() >= FONTS {
        cache.fonts.clear();
    }
    cache.fonts.insert(slot, Owned(font));
    Some(font)
}

fn draw(fonts: &mut FontSystem, key: CacheKey) -> Option<SwashImage> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let font = font_for(&mut cache, fonts, key)?;
        let glyph = key.glyph_id;
        let (dx, dy) = match key.flags.contains(CacheKeyFlags::PIXEL_FONT) {
            true => (key.x_bin.as_float().round(), key.y_bin.as_float().round()),
            false => (key.x_bin.as_float(), key.y_bin.as_float()),
        };
        let (dx, dy) = (f64::from(dx), f64::from(dy));
        let mut rect = CGRect::default();
        // SAFETY: one glyph, one rect.
        unsafe { CTFontGetBoundingRectsForGlyphs(font, HORIZONTAL, &glyph, &mut rect, 1) };
        if rect.size.width <= 0. || rect.size.height <= 0. {
            return None;
        }
        // Whole pixels round its box (y up), a pixel of room each side for
        // antialiasing, as swash's outlines have.
        let left = (rect.origin.x + dx).floor() as i32 - 1;
        let right = (rect.origin.x + rect.size.width + dx).ceil() as i32 + 1;
        let bottom = (rect.origin.y + dy).floor() as i32 - 1;
        let top = (rect.origin.y + rect.size.height + dy).ceil() as i32 + 1;
        let (width, height) = ((right - left) as u32, (top - bottom) as u32);
        let mut data = vec![0u8; width as usize * height as usize];
        // SAFETY: `data` is the context's `width` by `height` bytes and
        // outlives it; the context is released before `data` is moved.
        unsafe {
            let context = CGBitmapContextCreate(
                data.as_mut_ptr().cast(),
                width as usize,
                height as usize,
                8,
                width as usize,
                std::ptr::null(),
                ALPHA_ONLY,
            );
            if context.is_null() {
                return None;
            }
            let context = Owned(context);
            CGContextSetGrayFillColor(context.0, 1., 1.);
            CGContextSetShouldAntialias(context.0, true);
            CGContextSetShouldSmoothFonts(context.0, false);
            CGContextSetAllowsFontSubpixelPositioning(context.0, true);
            CGContextSetShouldSubpixelPositionFonts(context.0, true);
            CGContextSetAllowsFontSubpixelQuantization(context.0, false);
            CGContextSetShouldSubpixelQuantizeFonts(context.0, false);
            // The pen where its offset puts it, from the box's corner.
            let at = CGPoint {
                x: dx - f64::from(left),
                y: dy - f64::from(bottom),
            };
            CTFontDrawGlyphs(font, &glyph, &at, 1, context.0);
        }
        let mut image = SwashImage::new();
        image.content = SwashContent::Mask;
        image.placement = Placement {
            left,
            top,
            width,
            height,
        };
        image.data = data;
        Some(image)
    })
}

static SMOOTHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Whether glyphs draw with macOS's font smoothing (Chrome's default, `-webkit-font-smoothing:
/// auto`: Core Graphics dilates the stems and Skia blends the mask through its
/// luminance tables) rather than antialiased (what `-webkit-font-smoothing:
/// antialiased` gives, and Anara's web build sets: the plain coverage, with no
/// table). Set it once, before any text draws.
pub fn set_font_smoothing(on: bool) {
    SMOOTHING.store(on, std::sync::atomic::Ordering::Relaxed);
}
pub(crate) fn font_smoothing() -> bool {
    SMOOTHING.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether Core Text draws glyphs of `face` through Chrome's recipe: any
/// face but a colour one (emoji draw their own colours through swash).
/// The stroke Chrome adds to the system font at a weight past Medium and
/// before Semibold (501 to 599) under 17.5 points: font synthesis' fake bold,
/// Skia's `extra = size * lerp(size, 9..36, 1/24..1/32)` in points, on top of
/// the interpolated outlines (measured at 12 to 17 px: ink within 1%). `key`'s
/// size is in points.
fn synthetic_bold_width(fonts: &mut FontSystem, key: CacheKey) -> Option<f64> {
    let weight = key.font_weight.0;
    let size = f64::from(f32::from_bits(key.font_size_bits));
    if !(501..=599).contains(&weight) || size >= 17.5 {
        return None;
    }
    let system = fonts
        .db()
        .face(key.font_id)
        .is_some_and(|face| face.post_script_name.starts_with(".SFNS-"));
    system.then(|| {
        let t = ((size - 9.) / 27.).clamp(0., 1.);
        size * (1. / 24. + (1. / 32. - 1. / 24.) * t)
    })
}

/// `key`'s glyph as Chrome draws it on macOS: Skia draws it through Core
/// Graphics, black on white with font smoothing on, in a colour context at
/// the point size under the device scale (so the font's optical size and
/// tracking follow the point size), and blends it through a coverage table
/// by the text's luminance (`key.flags`, `chrome_mask_tables`). Measured
/// against Chrome's own pixels, this reproduces them to a level or two
/// where swash's outlines are 13% lighter. `scale`: device pixels a point;
/// `key`'s size is in device pixels.
pub(crate) fn chrome_glyph(
    fonts: &mut FontSystem,
    key: CacheKey,
    scale: f32,
) -> Option<SwashImage> {
    let mut key = key;
    let k = f64::from(scale.max(1.));
    key.font_size_bits = (f32::from_bits(key.font_size_bits) / scale.max(1.)).to_bits();
    let table = crate::chrome_mask_tables::for_luminance_bucket(key.flags.luminance_bucket());
    let smoothing = font_smoothing();
    let identity = !smoothing;
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let font = font_for(&mut cache, fonts, key)?;
        let glyph = key.glyph_id;
        let (dx, dy) = (
            f64::from(key.x_bin.as_float()),
            f64::from(key.y_bin.as_float()),
        );
        let mut rect = CGRect::default();
        // SAFETY: one glyph, one rect.
        unsafe { CTFontGetBoundingRectsForGlyphs(font, HORIZONTAL, &glyph, &mut rect, 1) };
        if rect.size.width <= 0. || rect.size.height <= 0. {
            return None;
        }
        let left = (k * rect.origin.x + dx).floor() as i32 - 2;
        let right = (k * (rect.origin.x + rect.size.width) + dx).ceil() as i32 + 2;
        let bottom = (k * rect.origin.y + dy).floor() as i32 - 2;
        let top = (k * (rect.origin.y + rect.size.height) + dy).ceil() as i32 + 2;
        let (width, height) = ((right - left) as usize, (top - bottom) as usize);
        let mut data = vec![0u8; width * height * 4];
        // SAFETY: `data` is the context's `width` by `height` pixels of four
        // bytes and outlives it; the context is released before it is read.
        unsafe {
            let space = Owned(CGColorSpaceCreateDeviceRGB());
            // `kCGImageAlphaNoneSkipFirst | kCGBitmapByteOrder32Little`.
            let context = CGBitmapContextCreate(
                data.as_mut_ptr().cast(),
                width,
                height,
                8,
                width * 4,
                space.0,
                2 | (2 << 12),
            );
            if context.is_null() {
                return None;
            }
            let context = Owned(context);
            CGContextSetRGBFillColor(context.0, 1., 1., 1., 1.);
            CGContextFillRect(
                context.0,
                CGRect {
                    origin: CGPoint { x: 0., y: 0. },
                    size: CGSize {
                        width: width as f64,
                        height: height as f64,
                    },
                },
            );
            CGContextSetRGBFillColor(context.0, 0., 0., 0., 1.);
            CGContextSetShouldAntialias(context.0, true);
            CGContextSetShouldSmoothFonts(context.0, smoothing);
            CGContextSetAllowsFontSubpixelPositioning(context.0, true);
            CGContextSetShouldSubpixelPositionFonts(context.0, true);
            CGContextSetAllowsFontSubpixelQuantization(context.0, false);
            CGContextSetShouldSubpixelQuantizeFonts(context.0, false);
            let at = CGPoint {
                x: (dx - f64::from(left)) / k,
                y: (dy - f64::from(bottom)) / k,
            };
            CGContextScaleCTM(context.0, k, k);
            if let Some(width) = synthetic_bold_width(fonts, key) {
                CGContextSetRGBStrokeColor(context.0, 0., 0., 0., 1.);
                CGContextSetLineWidth(context.0, width);
                CGContextSetTextDrawingMode(context.0, 2);
            }
            CTFontDrawGlyphs(font, &glyph, &at, 1, context.0);
        }
        let alpha: Vec<u8> = data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| {
                let gray = (u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2])) / 3;
                if identity {
                    255 - gray as u8
                } else {
                    table[usize::from(255 - gray as u8)]
                }
            })
            .collect();
        let mut image = SwashImage::new();
        image.content = SwashContent::Mask;
        image.placement = Placement {
            left,
            top,
            width: width as u32,
            height: height as u32,
        };
        image.data = alpha;
        Some(image)
    })
}
