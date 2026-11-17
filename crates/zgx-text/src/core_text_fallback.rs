//! The face macOS draws a character in that a run's face lacks, as Chrome
//! asks Core Text for it (Blink's `font_cache_mac.mm`,
//! `GetAlternateFontPlatformData`): system-ui at the size and weight, Core
//! Text's substitute for the character (`CTFontCreateForString`), then the
//! substitute's family again at the weight Blink gives Core Text, which is
//! where `.PingFang UI SC`'s Display face and `.CJK Symbols Fallback SC`'s
//! Medium come from. cosmic-text asks it before its fallback lists
//! (`FontSystem::set_platform_fallback`), which can't name those faces.
//! Their optical size is the text's, clamped to the axis, and their tracking
//! (`trak`) applies at it, as HarfBuzz shapes them in Chrome: both by
//! zgx-cosmic-text's instances (`Font::sized`).
use crate::core_text_glyphs::CGAffineTransform;
use cosmic_text::PlatformFace;
use std::ffi::c_void;

type CFTypeRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CFRange {
    location: isize,
    length: isize,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFRelease(object: CFTypeRef);
    fn CFGetTypeID(object: CFTypeRef) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFNumberCreate(allocator: CFTypeRef, kind: isize, value: *const c_void) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, kind: isize, value: *mut c_void) -> bool;
    fn CFDictionaryCreate(
        allocator: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFTypeRef;
    fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryCreateMutableCopy(
        allocator: CFTypeRef,
        capacity: isize,
        dictionary: CFTypeRef,
    ) -> CFTypeRef;
    fn CFDictionarySetValue(dictionary: CFTypeRef, key: CFTypeRef, value: CFTypeRef);
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    fn CFStringCreateWithCharacters(
        allocator: CFTypeRef,
        characters: *const u16,
        length: isize,
    ) -> CFTypeRef;
    fn CFStringGetCString(string: CFTypeRef, buffer: *mut u8, size: isize, encoding: u32) -> bool;
    fn CFCharacterSetIsLongCharacterMember(set: CFTypeRef, character: u32) -> bool;
    fn CFURLGetFileSystemRepresentation(
        url: CFTypeRef,
        resolve: bool,
        buffer: *mut u8,
        length: isize,
    ) -> bool;
}
#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    static kCTFontVariationAttribute: CFTypeRef;
    static kCTFontFamilyNameAttribute: CFTypeRef;
    static kCTFontTraitsAttribute: CFTypeRef;
    static kCTFontWeightTrait: CFTypeRef;
    static kCTFontSymbolicTrait: CFTypeRef;
    static kCTFontURLAttribute: CFTypeRef;
    static kCTFontVariationAxisIdentifierKey: CFTypeRef;
    static kCTFontVariationAxisDefaultValueKey: CFTypeRef;
    fn CTFontCreateUIFontForLanguage(kind: u32, size: f64, language: CFTypeRef) -> CFTypeRef;
    fn CTFontDescriptorCreateWithAttributes(attributes: CFTypeRef) -> CFTypeRef;
    fn CTFontCreateCopyWithAttributes(
        font: CFTypeRef,
        size: f64,
        matrix: *const c_void,
        attributes: CFTypeRef,
    ) -> CFTypeRef;
    fn CTFontCreateForString(font: CFTypeRef, string: CFTypeRef, range: CFRange) -> CFTypeRef;
    fn CTFontCreateWithFontDescriptor(
        descriptor: CFTypeRef,
        size: f64,
        matrix: *const CGAffineTransform,
    ) -> CFTypeRef;
    fn CTFontCopyFamilyName(font: CFTypeRef) -> CFTypeRef;
    fn CTFontCopyPostScriptName(font: CFTypeRef) -> CFTypeRef;
    fn CTFontCopyTraits(font: CFTypeRef) -> CFTypeRef;
    fn CTFontGetSymbolicTraits(font: CFTypeRef) -> u32;
    fn CTFontCopyCharacterSet(font: CFTypeRef) -> CFTypeRef;
    fn CTFontCopyAttribute(font: CFTypeRef, attribute: CFTypeRef) -> CFTypeRef;
    fn CTFontCopyVariation(font: CFTypeRef) -> CFTypeRef;
    fn CTFontCopyVariationAxes(font: CFTypeRef) -> CFTypeRef;
    fn CTFontGetGlyphsForCharacters(
        font: CFTypeRef,
        characters: *const u16,
        glyphs: *mut u16,
        count: isize,
    ) -> bool;
    fn CTFontGetAdvancesForGlyphs(
        font: CFTypeRef,
        orientation: u32,
        glyphs: *const u16,
        advances: *mut c_void,
        count: isize,
    ) -> f64;
}

/// `kCTFontUIFontSystem`: the system's interface font (CSS's `system-ui`).
const UI_FONT_SYSTEM: u32 = 2;
/// `kCFNumberSInt32Type`, `kCFNumberFloat64Type`.
const SINT32: isize = 3;
const FLOAT64: isize = 6;
/// `kCFStringEncodingUTF8`.
const UTF8: u32 = 0x0800_0100;
/// `kCTFontOrientationHorizontal`.
const HORIZONTAL: u32 = 1;
/// The weight axis's tag, as Core Text identifies axes.
const WGHT: i32 = i32::from_be_bytes(*b"wght");
/// The optical size axis's tag.
const OPSZ: i32 = i32::from_be_bytes(*b"opsz");

/// `ZGX_LOG_FACES=1`: log the face each character falls back to.
static LOG_FACES: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| zgx::env::var_os("ZGX_LOG_FACES").is_some_and(|v| v != "0"));

/// A Core Foundation object this holds a reference to (none: null).
struct Owned(CFTypeRef);
impl Owned {
    fn new(object: CFTypeRef) -> Option<Self> {
        // (Made only if there is one: an `Owned` of null, dropped, would
        // release null, which Core Foundation traps on.)
        (!object.is_null()).then(|| Self(object))
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: made by a Create or Copy function, released once.
        unsafe { CFRelease(self.0) }
    }
}

fn number_i32(value: i32) -> Option<Owned> {
    // SAFETY: an SInt32 read from `value`.
    Owned::new(unsafe { CFNumberCreate(std::ptr::null(), SINT32, (&raw const value).cast()) })
}
fn number_f64(value: f64) -> Option<Owned> {
    // SAFETY: a Float64 read from `value`.
    Owned::new(unsafe { CFNumberCreate(std::ptr::null(), FLOAT64, (&raw const value).cast()) })
}
/// A number's value as a float, if `number` is one.
fn read_f64(number: CFTypeRef) -> Option<f64> {
    let mut value = 0f64;
    // SAFETY: read only if it is a CFNumber.
    (is(number, unsafe { CFNumberGetTypeID() })
        && unsafe { CFNumberGetValue(number, FLOAT64, (&raw mut value).cast()) })
    .then_some(value)
}
/// Whether `object` is a live object of the type `id` (not null).
fn is(object: CFTypeRef, id: usize) -> bool {
    // SAFETY: a type's ID, of a live Core Foundation object.
    !object.is_null() && unsafe { CFGetTypeID(object) } == id
}
fn dictionary(pairs: &[(CFTypeRef, CFTypeRef)]) -> Option<Owned> {
    let keys: Vec<CFTypeRef> = pairs.iter().map(|pair| pair.0).collect();
    let values: Vec<CFTypeRef> = pairs.iter().map(|pair| pair.1).collect();
    // SAFETY: as many keys as values, all live CF objects, retained by the
    // dictionary with the CFType callbacks.
    Owned::new(unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            pairs.len() as isize,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        )
    })
}
fn string(string: CFTypeRef) -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: a CFString into a buffer of its length.
    let read =
        unsafe { CFStringGetCString(string, buffer.as_mut_ptr(), buffer.len() as isize, UTF8) };
    let end = buffer.iter().position(|&b| b == 0)?;
    read.then(|| String::from_utf8_lossy(&buffer[..end]).into_owned())
}

/// The weight Blink gives Core Text for a CSS weight (`ToCTFontWeight`,
/// `font_matcher_mac.mm`): each hundred's step, 450 taking 500's.
fn core_text_weight(weight: u16) -> f64 {
    const STEPS: [f64; 9] = [-0.8, -0.6, -0.4, 0.0, 0.23, 0.3, 0.4, 0.56, 0.62];
    STEPS[usize::from(weight.saturating_sub(50) / 100).min(8)]
}

/// A font's weight trait.
fn weight_trait(font: CFTypeRef) -> Option<f64> {
    // SAFETY: a live font's traits, a dictionary read by a constant key.
    let traits = Owned::new(unsafe { CTFontCopyTraits(font) })?;
    if !is(traits.0, unsafe { CFDictionaryGetTypeID() }) {
        return None;
    }
    read_f64(unsafe { CFDictionaryGetValue(traits.0, kCTFontWeightTrait) })
}

/// Core Text's font for `c` in system-ui at `weight` and `size`, as Blink
/// picks it, and `c` as UTF-16.
fn fallback_font(c: char, weight: u16, size: f64) -> Option<(Owned, Vec<u16>)> {
    // SAFETY: Core Text and Core Foundation calls on objects made here,
    // each released once (`Owned`), null results checked.
    unsafe {
        // system-ui at the weight (Blink's `MatchSystemUIFont`): its wght
        // axis, which SF has.
        let ui = Owned::new(CTFontCreateUIFontForLanguage(
            UI_FONT_SYSTEM,
            size,
            std::ptr::null(),
        ))?;
        let tag = number_i32(WGHT)?;
        let value = number_f64(f64::from(weight))?;
        let axes = dictionary(&[(tag.0, value.0)])?;
        let attributes = dictionary(&[(kCTFontVariationAttribute, axes.0)])?;
        let descriptor = Owned::new(CTFontDescriptorCreateWithAttributes(attributes.0))?;
        let base = Owned::new(CTFontCreateCopyWithAttributes(
            ui.0,
            size,
            std::ptr::null(),
            descriptor.0,
        ))?;
        // Core Text's substitute for the character.
        let mut units = [0u16; 2];
        let units = c.encode_utf16(&mut units).to_vec();
        let text = Owned::new(CFStringCreateWithCharacters(
            std::ptr::null(),
            units.as_ptr(),
            units.len() as isize,
        ))?;
        let range = CFRange {
            location: 0,
            length: units.len() as isize,
        };
        // With no language, as Blink asks it (a language picks Chrome's
        // per-script standard font first: `standard_families`).
        let substitute = Owned::new(CTFontCreateForString(base.0, text.0, range))?;
        let name = Owned::new(CTFontCopyPostScriptName(substitute.0))?;
        if string(name.0).is_none_or(|name| name == "LastResort" || name == ".LastResort") {
            return None;
        }
        // Its family again at Blink's weight, where it differs, if that face
        // holds the character.
        let wanted = core_text_weight(weight);
        let substitute_weight = weight_trait(substitute.0).unwrap_or(0.);
        if (substitute_weight - wanted).abs() < 1e-3 {
            return Some((substitute, units));
        }
        let family = Owned::new(CTFontCopyFamilyName(substitute.0))?;
        let symbolic = number_i32(CTFontGetSymbolicTraits(base.0) as i32)?;
        let weight_value = number_f64(wanted)?;
        let traits = dictionary(&[
            (kCTFontSymbolicTrait, symbolic.0),
            (kCTFontWeightTrait, weight_value.0),
        ])?;
        let attributes = dictionary(&[
            (kCTFontFamilyNameAttribute, family.0),
            (kCTFontTraitsAttribute, traits.0),
        ])?;
        let descriptor = Owned::new(CTFontDescriptorCreateWithAttributes(attributes.0))?;
        let Some(best) = Owned::new(CTFontCreateWithFontDescriptor(
            descriptor.0,
            size,
            std::ptr::null(),
        )) else {
            return Some((substitute, units));
        };
        let differs = weight_trait(best.0).is_some_and(|w| (w - substitute_weight).abs() >= 1e-3)
            || CTFontGetSymbolicTraits(best.0) != CTFontGetSymbolicTraits(substitute.0);
        let holds = Owned::new(CTFontCopyCharacterSet(best.0))
            .is_some_and(|set| CFCharacterSetIsLongCharacterMember(set.0, u32::from(c)));
        Some((if differs && holds { best } else { substitute }, units))
    }
}

/// `font`'s weight axis's value: as set, else the axis's default; none
/// without the axis.
fn wght(font: CFTypeRef) -> Option<f64> {
    // SAFETY: a live font's variation and axes, read by constant keys and
    // the axis's tag.
    unsafe {
        let tag = number_i32(WGHT)?;
        // (Each object of the type it's read as: Apple Color Emoji's UI
        // face gave one Core Foundation trapped on, read as another.)
        let dictionary = CFDictionaryGetTypeID();
        if let Some(variation) = Owned::new(CTFontCopyVariation(font))
            && is(variation.0, dictionary)
            && let Some(value) = read_f64(CFDictionaryGetValue(variation.0, tag.0))
        {
            return Some(value);
        }
        let axes = Owned::new(CTFontCopyVariationAxes(font))?;
        if !is(axes.0, CFArrayGetTypeID()) {
            return None;
        }
        (0..CFArrayGetCount(axes.0)).find_map(|i| {
            let axis = CFArrayGetValueAtIndex(axes.0, i);
            if !is(axis, dictionary) {
                return None;
            }
            let id = read_f64(CFDictionaryGetValue(
                axis,
                kCTFontVariationAxisIdentifierKey,
            ))?;
            (id as i32 == WGHT)
                .then(|| {
                    read_f64(CFDictionaryGetValue(
                        axis,
                        kCTFontVariationAxisDefaultValueKey,
                    ))
                })
                .flatten()
        })
    }
}

/// The face Core Text, as Chrome asks it, draws `c` in at `weight` and
/// `size` (pixels; 0 for 12), for cosmic-text's fallback
/// (`FontSystem::set_platform_fallback`).
pub(crate) fn face(c: char, weight: u16, size: u16) -> Option<PlatformFace> {
    let size = if size == 0 { 12. } else { f64::from(size) };
    let (font, _) = fallback_font(c, weight, size)?;
    // SAFETY: the font's name and file, read once each.
    let (name, path) = unsafe {
        let name = Owned::new(CTFontCopyPostScriptName(font.0))?;
        let url = Owned::new(CTFontCopyAttribute(font.0, kCTFontURLAttribute))?;
        let mut buffer = [0u8; 1024];
        if !CFURLGetFileSystemRepresentation(
            url.0,
            true,
            buffer.as_mut_ptr(),
            buffer.len() as isize,
        ) {
            return None;
        }
        let end = buffer.iter().position(|&b| b == 0)?;
        (
            string(name.0)?,
            std::path::PathBuf::from(String::from_utf8_lossy(&buffer[..end]).into_owned()),
        )
    };
    let wght = wght(font.0).map_or(weight, |w| w.round().clamp(1., 1000.) as u16);
    // `ZGX_LOG_FACES=1`: each character's face as it is first asked for
    // (once a character, weight and size: cosmic-text keeps the answer).
    if *LOG_FACES {
        eprintln!(
            "zgx: {c} (U+{:04X}) at {size} px, weight {weight}: {name} (wght {wght}), {}",
            u32::from(c),
            path.display()
        );
    }
    Some(PlatformFace {
        post_script_name: name,
        path,
        weight: wght,
    })
}

/// Core Text's own measure of `c` in the face it falls back to at `weight`
/// and `size`: that face's PostScript name and the character's advance in
/// pixels, tracking included. For tests, against what zgx shapes.
#[doc(hidden)]
pub fn core_text_advance(c: char, weight: u16, size: f32) -> Option<(String, f64)> {
    let (font, units) = fallback_font(c, weight, f64::from(size))?;
    // Blink's last step (`FontPlatformDataFromCTFont`): the face's optical
    // size is the text's, which Core Text clamps to the axis; its other
    // axes as they are.
    let font = with_optical(font, f64::from(size))?;
    let mut glyphs = vec![0u16; units.len()];
    // SAFETY: the font's glyph for the character's units, and its advance.
    unsafe {
        if !CTFontGetGlyphsForCharacters(
            font.0,
            units.as_ptr(),
            glyphs.as_mut_ptr(),
            units.len() as isize,
        ) {
            return None;
        }
        let advance = CTFontGetAdvancesForGlyphs(
            font.0,
            HORIZONTAL,
            glyphs.as_ptr(),
            std::ptr::null_mut(),
            1,
        );
        let name = Owned::new(CTFontCopyPostScriptName(font.0))?;
        Some((string(name.0)?, advance))
    }
}

/// `font` with its optical size axis at `size` (Core Text clamps it to the
/// axis), its other axes kept; itself without the axis.
fn with_optical(font: Owned, size: f64) -> Option<Owned> {
    // SAFETY: a live font's variation, copied and set by the axis's tag;
    // objects made here released once.
    unsafe {
        let tag = number_i32(OPSZ)?;
        let value = number_f64(size)?;
        if !has_axis(font.0, OPSZ) {
            return Some(font);
        }
        // Its variation as Core Text set it (a face at its defaults has
        // none), the optical size set in a copy.
        let current = match Owned::new(CTFontCopyVariation(font.0)) {
            Some(current) => current,
            None => dictionary(&[])?,
        };
        let variation = Owned::new(CFDictionaryCreateMutableCopy(
            std::ptr::null(),
            0,
            current.0,
        ))?;
        CFDictionarySetValue(variation.0, tag.0, value.0);
        let attributes = dictionary(&[(kCTFontVariationAttribute, variation.0)])?;
        let descriptor = Owned::new(CTFontDescriptorCreateWithAttributes(attributes.0))?;
        Owned::new(CTFontCreateCopyWithAttributes(
            font.0,
            size,
            std::ptr::null(),
            descriptor.0,
        ))
    }
}

/// Whether `font` has the axis `tag`.
fn has_axis(font: CFTypeRef, tag: i32) -> bool {
    // SAFETY: a live font's axes, read by a constant key.
    unsafe {
        let Some(axes) = Owned::new(CTFontCopyVariationAxes(font)) else {
            return false;
        };
        if !is(axes.0, CFArrayGetTypeID()) {
            return false;
        }
        (0..CFArrayGetCount(axes.0)).any(|i| {
            let axis = CFArrayGetValueAtIndex(axes.0, i);
            is(axis, CFDictionaryGetTypeID())
                && read_f64(CFDictionaryGetValue(
                    axis,
                    kCTFontVariationAxisIdentifierKey,
                ))
                .is_some_and(|id| id as i32 == tag)
        })
    }
}

/// Chrome's standard font for `locale`'s script on macOS
/// (`locale_settings_mac.grd`'s `IDS_STANDARD_FONT_FAMILY_*`), which Blink
/// tries for a character the text's families lack before Core Text's
/// substitute (`FontFallbackList::GetFontData`'s `-webkit-standard`, by the
/// content language's script): Japanese, Korean, simplified and traditional
/// Chinese (Hong Kong and Macau take traditional), else Times.
pub(crate) fn standard_families(locale: &str) -> &'static [&'static str] {
    let mut subtags = locale.split(['-', '_']).map(str::to_ascii_lowercase);
    let language = subtags.next().unwrap_or_default();
    let rest: Vec<String> = subtags.collect();
    let has = |tag: &str| rest.iter().any(|subtag| subtag == tag);
    match language.as_str() {
        "ja" => &["Hiragino Kaku Gothic ProN"],
        "ko" => &["Apple SD Gothic Neo"],
        "zh" if has("hant") || has("tw") || has("hk") || has("mo") => &["PingFang TC", "Heiti TC"],
        "zh" => &["PingFang SC", "STHeiti"],
        _ => &["Times"],
    }
}

/// Install Core Text's fallback on `fonts` (`FontSystem::set_platform_fallback`),
/// with the standard font of the language text is in first (the font
/// system's locale, which `FontSystem::set_language` sets), as zgx's own
/// font system has it:
/// for tests that make one of another locale.
#[doc(hidden)]
pub fn install_core_text_fallback(fonts: &mut cosmic_text::FontSystem) {
    fonts.set_platform_fallback(Some(Box::new(face)), standard_families);
}
