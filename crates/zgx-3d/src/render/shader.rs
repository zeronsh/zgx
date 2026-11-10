//! The meshes' WGSL (`shader.wgsl`), one text with tagged lines, and the
//! variants cut from it.
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
/// (geometric specular anti-aliasing). (A cut-out or placed one is
/// textured; a line with two prefixes is for a variant with both.) Only
/// its code is kept, without comments or others' lines: naga reads every
/// byte of a shader, and most of these are others'.
// Made with a pipeline, rarely: kept out of the frame's code.
#[cold]
#[inline(never)]
pub(super) fn wgsl(variant: Variant, samples: u32) -> String {
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

/// Metallic-roughness shading (GGX, Smith, Schlick) under a sun, up to four
/// point lights and a hemisphere's ambient, mapped to display range by
/// Khronos's PBR Neutral, premultiplied.
pub(super) const SHADER: &str = include_str!("shader.wgsl");

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
