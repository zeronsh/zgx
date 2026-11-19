//! Glyphs drawn by Core Text (macOS) for the faces only Core Text can read
//! (Apple's `hvgl`, as macOS 26's PingFang keeps them: zgx-cosmic-text's
//! `Font::needs_platform_outlines`); every other face's are swash's,
//! thickened as CoreGraphics does (`text::glyph_image`). The glyph comes out
//! as swash's would: an alpha mask at the cache key's size and subpixel
//! offset, placed from the pen position, for the atlas to take like any
//! other. Its strokes are thickened as macOS's font smoothing thickens them
//! for the text's colour (`CacheKeyFlags::SMOOTHING`, `text::smoothing`).
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
pub(crate) struct CGAffineTransform {
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
    fn CFGetTypeID(object: CFTypeRef) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: CFTypeRef, kind: isize, value: *mut c_void) -> bool;
    fn CFPreferencesCopyAppValue(key: CFTypeRef, application: CFTypeRef) -> CFTypeRef;
    static kCFPreferencesCurrentApplication: CFTypeRef;
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
    fn CGContextSetShouldAntialias(context: CFTypeRef, on: bool);
    fn CGContextSetShouldSmoothFonts(context: CFTypeRef, on: bool);
    fn CGContextSetAllowsFontSmoothing(context: CFTypeRef, on: bool);
    fn CGContextSetAllowsFontSubpixelPositioning(context: CFTypeRef, on: bool);
    fn CGContextSetShouldSubpixelPositionFonts(context: CFTypeRef, on: bool);
    fn CGContextSetAllowsFontSubpixelQuantization(context: CFTypeRef, on: bool);
    fn CGContextSetShouldSubpixelQuantizeFonts(context: CFTypeRef, on: bool);
    fn CGContextClearRect(context: CFTypeRef, rect: CGRect);
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
/// The optical size axis's.
const OPSZ: i32 = i32::from_be_bytes(*b"opsz");
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
    /// Fonts made from them, by face, size, slant, weight and optical size.
    fonts: HashMap<(cosmic_text::fontdb::ID, u32, bool, u16, u16), Owned>,
}
thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    /// The bitmap glyphs are drawn into, kept: made, set up and released
    /// for each glyph, a context costs as much as the glyph's drawing.
    static CANVAS: RefCell<Option<Canvas>> = const { RefCell::new(None) };
}
/// A one-byte-a-pixel context over `pixels`, `width` by `height`, set up to
/// draw glyphs as macOS draws its text: each glyph cleared for and drawn in
/// its bottom left corner (Core Graphics' origin), then copied out.
struct Canvas {
    // Dropped (released) before `pixels`, which it draws into.
    context: Owned,
    pixels: Vec<u8>,
    width: usize,
    height: usize,
}
impl Canvas {
    /// One at least `width` by `height`, grown in steps of 64 so a few
    /// sizes make all of them.
    fn at_least(width: usize, height: usize) -> Option<Canvas> {
        let (width, height) = (width.next_multiple_of(64), height.next_multiple_of(64));
        let mut pixels = vec![0u8; width * height];
        // SAFETY: `pixels` is the context's `width` by `height` bytes, kept
        // beside it in `Canvas` and dropped after it.
        unsafe {
            let context = CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                width,
                height,
                8,
                width,
                std::ptr::null(),
                ALPHA_ONLY,
            );
            if context.is_null() {
                return None;
            }
            CGContextSetAllowsFontSmoothing(context, true);
            CGContextSetShouldAntialias(context, true);
            CGContextSetAllowsFontSubpixelPositioning(context, true);
            CGContextSetShouldSubpixelPositionFonts(context, true);
            CGContextSetAllowsFontSubpixelQuantization(context, false);
            CGContextSetShouldSubpixelQuantizeFonts(context, false);
            Some(Canvas {
                context: Owned(context),
                pixels,
                width,
                height,
            })
        }
    }
}
/// The largest canvas kept between glyphs, a side in pixels: a glyph past
/// it (a heading zoomed far in) draws in one of its own, let go after.
const KEPT_CANVAS: usize = 256;

/// `key`'s glyph through Core Text, as macOS draws its own text; none for a
/// colour face's (emoji: swash draws their bitmaps and layers) or an inline
/// box's (it draws nothing), or where Core Text can't read the face.
pub(crate) fn glyph_image(fonts: &mut FontSystem, key: CacheKey) -> Option<SwashImage> {
    if key.flags.contains(CacheKeyFlags::INLINE_BOX) {
        return None;
    }
    let font = fonts.get_font(key.font_id, key.font_weight)?;
    // Only the faces swash can't draw: asked of every glyph swash draws
    // nothing for (a space), Core Text for the rest drew nothing more.
    if !font.needs_platform_outlines() {
        return None;
    }
    draw(fonts, key)
}

/// Whether macOS thickens text as its font smoothing does: unless the user
/// turned it off (`defaults write -g AppleFontSmoothing -int 0`), as Core
/// Text's own drawing reads it. Read once.
pub(crate) fn font_smoothing_allowed() -> bool {
    static ALLOWED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ALLOWED.get_or_init(|| {
        let key = b"AppleFontSmoothing";
        // SAFETY: the key and the value are owned and released here; the
        // value is read as a number only if it is one.
        unsafe {
            let key = Owned(CFStringCreateWithBytes(
                std::ptr::null(),
                key.as_ptr(),
                key.len() as isize,
                UTF8,
                false,
            ));
            if key.0.is_null() {
                return true;
            }
            let value = CFPreferencesCopyAppValue(key.0, kCFPreferencesCurrentApplication);
            if value.is_null() {
                return true;
            }
            let value = Owned(value);
            if CFGetTypeID(value.0) != CFNumberGetTypeID() {
                return true;
            }
            let mut level = 1i32;
            // Only an explicit 0 turns it off.
            !CFNumberGetValue(value.0, SINT32, (&raw mut level).cast()) || level != 0
        }
    })
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
        // if its name is the face's. Not for the system's hidden faces
        // (`.SFNS-Regular`): asked for by name, Core Text gives Times and
        // logs that it did; their file has them.
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
    let slot = (
        key.font_id,
        key.font_size_bits,
        italic,
        key.font_weight.0,
        key.font_optical,
    );
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
        // one variable face, Medium by default, Regular at 400), and at its
        // optical size where it was shaped at one, as swash sets `opsz`:
        // Core Text leaves a face without the axis as it is.
        let vary = |descriptor: CFTypeRef, tag: i32, value: f64| {
            let axis = Owned(CFNumberCreate(
                std::ptr::null(),
                SINT32,
                (&raw const tag).cast(),
            ));
            Owned(CTFontDescriptorCreateCopyWithVariation(
                descriptor, axis.0, value,
            ))
        };
        let weighted = vary(descriptor.0, WGHT, f64::from(key.font_weight.0));
        let at = match weighted.0.is_null() {
            true => descriptor.0,
            false => weighted.0,
        };
        let sized = (key.font_optical > 0).then(|| vary(at, OPSZ, f64::from(key.font_optical)));
        let at = match &sized {
            Some(sized) if !sized.0.is_null() => sized.0,
            _ => at,
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
        let (w, h) = (width as usize, height as usize);
        let data = CANVAS.with(|kept| {
            let mut kept = kept.borrow_mut();
            let mut own = None;
            let canvas = if w > KEPT_CANVAS || h > KEPT_CANVAS {
                own.insert(Canvas::at_least(w, h)?)
            } else {
                if kept.as_ref().is_none_or(|c| c.width < w || c.height < h) {
                    let (was_w, was_h) = kept.as_ref().map_or((0, 0), |c| (c.width, c.height));
                    *kept = None;
                    *kept = Some(Canvas::at_least(w.max(was_w), h.max(was_h))?);
                }
                kept.as_mut()?
            };
            let context = canvas.context.0;
            // SAFETY: the context lives in the canvas, its pixels beside it;
            // what it draws stays in the corner cleared for it.
            unsafe {
                let corner = CGRect {
                    origin: CGPoint::default(),
                    size: CGSize {
                        width: f64::from(width),
                        height: f64::from(height),
                    },
                };
                CGContextClearRect(context, corner);
                // Font smoothing thickens strokes, the more the lighter the
                // fill, as macOS's own text is thickened by its colour's (four
                // steps, plus one: `text::smoothing`; 0 not smoothed). An
                // alpha-only context keeps only the coverage, so the fill
                // colour says how much.
                let smoothing = (key.flags & CacheKeyFlags::SMOOTHING).bits() >> 6;
                CGContextSetShouldSmoothFonts(context, smoothing > 0);
                let level = smoothing.saturating_sub(1).min(4);
                CGContextSetGrayFillColor(context, f64::from(level) * 0.25, 1.);
                // The pen where its offset puts it, from the box's corner.
                let at = CGPoint {
                    x: dx - f64::from(left),
                    y: dy - f64::from(bottom),
                };
                CTFontDrawGlyphs(font, &glyph, &at, 1, context);
            }
            // Its rows, top first: the corner's are the canvas's last `h`.
            let mut data = Vec::with_capacity(w * h);
            for row in canvas.height - h..canvas.height {
                let start = row * canvas.width;
                data.extend_from_slice(&canvas.pixels[start..start + w]);
            }
            Some(data)
        })?;
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
