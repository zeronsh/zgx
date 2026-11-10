//! The meshes' WGSL (`shader/*.wgsl`, a part a feature), one text with
//! tagged lines, and the variants cut from it.
use super::Variant;
/// The shader, sampling textures or not, skinning or not, lit by an
/// environment or not, drawing `samples` a pixel: its lines marked `//T `
/// are a textured pipeline's alone (its maps' bindings, samples and normal
/// mapping), `//S ` a skinned one's (its joints), `//M ` a morphing one's
/// (its targets), `//P ` either's (the frame's joints and weights), `//E `
/// an environment-lit one's (its cube and diffuse light), `//U ` a placed
/// one's (its maps' placements and second coordinates), `//C ` a colored
/// one's (its vertex colours), `//G ` one light passes through's, `//L ` a
/// coated one's, `//R ` one reflecting as its specular says and `//Q ` any
/// other's, `//W ` any of those three's (their numbers), `//K ` a
/// cut-out one's and `//N ` any other's, and `//A ` a multisampled one's
/// (geometric specular anti-aliasing), `//X ` a clustered one's and `//Y `
/// any other's (`shader/lights.wgsl`), `//H ` one shadowed by the sun's
/// cascades and `//I ` any other's (`shader/shadows.wgsl`). (A cut-out or placed one is
/// textured; a line with two prefixes is for a variant with both.) Only
/// its code is kept, without comments or others' lines: naga reads every
/// byte of a shader, and most of these are others'.
// Made with a pipeline, rarely: kept out of the frame's code.
#[cold]
#[inline(never)]
pub(super) fn wgsl(variant: Variant, samples: u32) -> String {
    wgsl_with(variant, samples, None)
}
/// `wgsl`'s shader with `custom` code (a mesh's shader, the world's surface
/// layers): its lines marked `//V ` are a custom one's alone, `//O ` any
/// other's, and `//D ` one whose mesh's shader cuts it out (`opacity`).
#[cold]
#[inline(never)]
pub(super) fn wgsl_with(variant: Variant, samples: u32, custom: Option<&Custom>) -> String {
    let on = |tag: u8| match tag {
        b'T' => variant.textured(),
        b'S' => variant.skinned(),
        b'M' => variant.morphing(),
        b'P' => variant.skinned() || variant.morphing(),
        b'E' => variant.lit(),
        b'U' => variant.placed(),
        b'C' => variant.colored(),
        b'G' => variant.transmitting(),
        b'L' => variant.coated(),
        b'R' => variant.specular(),
        b'Q' => !variant.specular(),
        b'W' => variant.transmitting() || variant.coated() || variant.specular(),
        b'K' => variant.masked(),
        b'N' => !variant.masked(),
        b'A' => samples > 1,
        b'X' => variant.clustered(),
        b'Y' => !variant.clustered(),
        b'H' => variant.cascaded(),
        b'I' => !variant.cascaded(),
        b'V' => custom.is_some(),
        b'O' => custom.is_none(),
        b'D' => custom.is_some_and(cuts),
        _ => false,
    };
    // The kept chunks, copied whole.
    let (text, chunks) = shader_chunks();
    let mut source = String::with_capacity(SHADER.len() / 2);
    for (tags, range) in chunks {
        if tags.iter().all(|&tag| tag == 0 || on(tag)) {
            source.push_str(&text[range.clone()]);
        }
    }
    if let Some(custom) = custom {
        source.push_str(code_of(custom));
    }
    source
}
/// The shader's code, and its chunks by the prefixes they need (0s:
/// none): runs of lines needing the same ones, comments and blank lines
/// left out, as ranges of the code. Read once, with the first pipeline: a
/// variant then copies its chunks whole instead of reading every byte of
/// every line again. (One buffer and one list: two allocations.)
pub(super) type Chunks = (String, Vec<([u8; 3], std::ops::Range<usize>)>);
pub(super) fn shader_chunks() -> &'static Chunks {
    static CHUNKS: std::sync::OnceLock<Chunks> = std::sync::OnceLock::new();
    CHUNKS.get_or_init(|| {
        let mut text = String::with_capacity(SHADER.len());
        let mut chunks: Vec<([u8; 3], std::ops::Range<usize>)> = Vec::with_capacity(256);
        for line in SHADER.lines() {
            let (mut tags, mut count, mut rest) = ([0; 3], 0, line);
            while let [b'/', b'/', tag, b' ', ..] = *rest.as_bytes() {
                // (Three at most: more needs a wider index.)
                tags[count] = tag;
                count += 1;
                rest = &rest[4..];
            }
            let code = rest.trim_ascii_start();
            if code.is_empty() || code.starts_with("//") {
                continue;
            }
            let start = text.len();
            text.push_str(rest);
            text.push('\n');
            match chunks.last_mut() {
                Some((same, range)) if *same == tags => range.end = text.len(),
                _ => chunks.push((tags, start..text.len())),
            }
        }
        (text, chunks)
    })
}

#[cfg(feature = "shaders")]
use crate::material::Custom;
/// Custom code: none without the `shaders` feature.
#[cfg(not(feature = "shaders"))]
pub(super) enum Custom {}
#[cfg(feature = "shaders")]
fn cuts(custom: &Custom) -> bool {
    custom.opacity
}
#[cfg(feature = "shaders")]
fn code_of(custom: &Custom) -> &str {
    &custom.code
}
#[cfg(not(feature = "shaders"))]
fn cuts(custom: &Custom) -> bool {
    match *custom {}
}
#[cfg(not(feature = "shaders"))]
fn code_of(custom: &Custom) -> &str {
    match *custom {}
}
/// The shader's parts (`shader/*.wgsl`), in order, as one text; with the
/// `shaders` feature, custom materials' inputs (`material`) after them (a
/// build without it doesn't read their lines).
macro_rules! parts {
    ($($name:literal),* $(,)?) => {
        /// Metallic-roughness shading (GGX, Smith, Schlick) under a sun, up
        /// to eight point or spot lights and a hemisphere's or an
        /// environment's ambient, mapped to display range, premultiplied:
        /// its parts joined, read as one text with tagged lines.
        #[cfg(not(feature = "shaders"))]
        pub(super) const SHADER: &str = concat!($(include_str!(concat!("shader/", $name, ".wgsl"))),*);
        #[cfg(feature = "shaders")]
        pub(super) const SHADER: &str = concat!(
            $(include_str!(concat!("shader/", $name, ".wgsl")),)*
            include_str!("shader/material.wgsl")
        );
        /// Each part's name and text.
        #[cfg(test)]
        const PARTS: &[(&str, &str)] = &[
            $(($name, include_str!(concat!("shader/", $name, ".wgsl"))),)*
            #[cfg(feature = "shaders")]
            ("material", include_str!("shader/material.wgsl")),
        ];
    };
}
// A feature's WGSL is a part of its own, listed here. Its lines carry its
// feature's tag (`wgsl`'s `on`), so a variant without the feature keeps
// today's text (`every_variants_text_is_todays`); WGSL declares in any
// order, so a part of functions and bindings goes anywhere, and only what
// the stages call it from (a tagged line in `vertex` or `fragment`) has a
// place. Comments and blank lines cost nothing: they're dropped once.
parts![
    "frame", "maps", "skin", "vertex", "caster", "light", "display", "fragment", "lights",
    "shadows",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_untextured_shader_binds_no_texture_an_unskinned_no_joints_and_an_unlit_no_cube() {
        let source = |change: fn(&mut Variant)| {
            let mut variant = Variant::default();
            change(&mut variant);
            wgsl(variant, 1)
        };
        let plain = source(|_| {});
        let textured = source(|v| *v = v.with(Variant::TEXTURED, true));
        let skinned = source(|v| *v = v.with(Variant::SKINNED, true));
        let lit = source(|v| *v = v.with(Variant::LIT, true));
        let morphing = source(|v| *v = v.with(Variant::MORPHING, true));
        let masked = source(|v| {
            *v = v.with(Variant::TEXTURED, true);
            *v = v.with(Variant::MASKED, true);
        });
        // The plain shader names no map, joints or environment's cube.
        for line in plain.lines() {
            for name in [
                "_texture",
                "@group(1)",
                "palette",
                "environment_cube",
                "morphs",
                "placements",
                "second_uv",
                "vertex_color",
                "behind",
                "layers.",
                "coat",
                "discard",
            ] {
                assert!(!line.contains(name), "{line}");
            }
        }
        assert!(textured.contains("@group(1) @binding(2) var normal_texture"));
        assert!(!textured.contains("palette") && !textured.contains("second_uv"));
        assert!(skinned.contains("var<storage, read> palette"));
        assert!(!skinned.contains("normal_texture"));
        assert!(lit.contains("@group(0) @binding(3) var environment_cube: texture_cube<f32>"));
        assert!(!lit.contains("normal_texture"));
        assert!(morphing.contains("var<storage, read> morphs"));
        assert!(morphing.contains("var<storage, read> palette"));
        assert!(masked.contains("discard;") && masked.contains("-> ShadowOut"));
        let placed = source(|v| {
            *v = v.with(Variant::TEXTURED, true);
            *v = v.with(Variant::PLACED, true);
        });
        assert!(placed.contains("var<uniform> placements"));
        let colored = source(|v| *v = v.with(Variant::COLORED, true));
        assert!(colored.contains("@location(15) vertex_color: vec4<f32>"));
        let glass = source(|v| {
            *v = v.with(Variant::TEXTURED, true);
            *v = v.with(Variant::TRANSMITTING, true);
        });
        assert!(glass.contains("var thickness_texture: texture_2d<f32>"));
        assert!(!textured.contains("thickness_texture"));
        let coated = source(|v| {
            *v = v.with(Variant::TEXTURED, true);
            *v = v.with(Variant::COATED, true);
        });
        assert!(coated.contains("var<uniform> layers: Layers"));
        assert!(coated.contains("coat_lit") && !coated.contains("behind"));
        assert!(glass.contains("var<uniform> layers: Layers") && !glass.contains("coat_lit"));
        let specular = source(|v| {
            *v = v.with(Variant::TEXTURED, true);
            *v = v.with(Variant::SPECULAR, true);
        });
        assert!(specular.contains("lit = shade_specular(") && !specular.contains("lit = shade("));
        assert!(textured.contains("lit = shade(") && !textured.contains("shade_specular"));
    }

    /// FNV-1a over the text of every variant whose bits are under `below`
    /// (one sample a pixel and four), in order.
    fn variants_text(below: u32) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for bits in 0..below {
            for samples in [1, 4] {
                for byte in wgsl(Variant(bits as u16), samples).bytes().chain([0]) {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        hash
    }
    /// The bits `variant.rs` declares (`const NAME: u16 = 1 << n;`).
    fn declared_bits() -> Vec<u32> {
        include_str!("variant.rs")
            .lines()
            .filter_map(|line| {
                line.split_once(": u16 = 1 << ")?
                    .1
                    .strip_suffix(';')?
                    .parse()
                    .ok()
            })
            .collect()
    }

    #[test]
    fn every_variant_bit_is_covered() {
        // A new bit joins `Variant::ALL`, and with it the text and the
        // validation checks below (and their digest moves on purpose).
        let bits = declared_bits();
        let all = bits.iter().fold(0u32, |all, bit| all | 1 << bit);
        assert_eq!(all, u32::from(Variant::ALL), "{bits:?}");
        assert_eq!(all & (all + 1), 0, "bits from 0 up, none skipped: {bits:?}");
    }

    #[test]
    fn every_variants_text_is_todays() {
        // What the shader was before it came from several files, byte for
        // byte, for every variant of the bits it had then (0 to 10). A part
        // a feature adds keeps its lines behind its own tag, so a world that
        // turns nothing on parses what it did; change this only with a
        // change to today's shading.
        let before = variants_text(1 << 11);
        assert_eq!(before, 0x0a4b_2237_f883_7a85, "{before:#018x}");
        // Those of bits 0 to 12 (clustered lights', cascades') as they were
        // before custom materials (bit 13), whose code they don't parse.
        let lit = variants_text(1 << 13);
        assert_eq!(lit, 0x3d4d_9104_4d81_6ec5, "{lit:#018x}");
        // And every variant of every bit there is now, as it is now.
        let all = variants_text(u32::from(Variant::ALL) + 1);
        assert_eq!(all, 0xbc9a_c07f_4884_1b65, "{all:#018x}");
    }

    /// `bits` as a variant a mesh can have: a cut-out, placed, glass, coated
    /// or specular one is textured (`variant_of`).
    fn possible(bits: u16) -> Variant {
        let needs = Variant::MASKED
            | Variant::PLACED
            | Variant::TRANSMITTING
            | Variant::COATED
            | Variant::SPECULAR;
        Variant(bits).with(Variant::TEXTURED, bits & (needs | Variant::TEXTURED) != 0)
    }
    /// `variant`'s shader at `samples` parsed and validated by naga, as the
    /// device's compiler is given it.
    fn validates(variant: Variant, samples: u32) -> Result<(), String> {
        use zgx_gpu::wgpu::naga;
        let text = wgsl(variant, samples);
        let module = naga::front::wgsl::parse_str(&text).map_err(|e| e.emit_to_string(&text))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|e| e.emit_to_string(&text))?;
        Ok(())
    }

    #[test]
    fn every_variant_bit_and_pair_of_bits_validates() {
        // Each bit alone and with each other: what one feature's lines do to
        // another's. (`every_variant_validates` tries them all, in release.)
        let bits = declared_bits();
        let mut tried = 0;
        for (i, &a) in bits.iter().enumerate() {
            for &b in &bits[i..] {
                let variant = possible((1 << a | 1 << b) as u16);
                for samples in [1, 4] {
                    validates(variant, samples)
                        .unwrap_or_else(|e| panic!("{variant:?} at {samples}: {e}"));
                    tried += 1;
                }
            }
        }
        validates(Variant::default(), 4).unwrap();
        assert_eq!(tried, bits.len() * (bits.len() + 1));
    }

    #[test]
    #[ignore = "every variant: run in release (cargo test --release -- --ignored)"]
    fn every_variant_validates() {
        for bits in 0..=Variant::ALL {
            for samples in [1, 4] {
                validates(possible(bits), samples)
                    .unwrap_or_else(|e| panic!("{:?} at {samples}: {e}", possible(bits)));
            }
        }
    }

    #[test]
    fn each_part_ends_its_last_line() {
        // Else its last line and the next part's first would be one.
        for (name, text) in PARTS {
            assert!(text.ends_with('\n'), "{name}");
        }
        let joined: String = PARTS.iter().map(|(_, text)| *text).collect();
        assert_eq!(joined, SHADER);
    }

    #[test]
    fn specular_anti_aliasing_is_a_multisampled_shaders_alone() {
        let (one, four) = (wgsl(Variant::default(), 1), wgsl(Variant::default(), 4));
        assert!(!one.contains("dpdx(facing)"));
        assert!(four.contains("\nlet turning_x = dpdx(facing);"));
    }

    #[test]
    fn a_shader_keeps_its_code_alone() {
        let skinned = Variant::default().with(Variant::SKINNED, true);
        let code = wgsl(skinned, 4);
        assert!(
            code.len() < SHADER.len() / 2,
            "{} {}",
            code.len(),
            SHADER.len()
        );
        assert!(code.lines().all(|l| {
            let l = l.trim_start();
            !l.is_empty() && !l.starts_with("//")
        }));
        assert!(code.contains("var<storage, read> palette"));
        assert!(code.contains("fn fragment"));
        // A line's prefixes go with it, those on and those off.
        assert!(!code.contains("//S ") && !code.contains("//T "));
    }
}
