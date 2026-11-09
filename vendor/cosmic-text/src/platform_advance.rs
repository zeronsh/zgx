//! zgx: glyphs whose advance the platform sets, iOS's emoji: Core Text gives
//! an emoji an advance of its own at each point size (23 points at 17, 38 at
//! 34), not the font's one em. zgx-gpu registers those faces with a hook
//! that asks Core Text; their glyphs measure and lay out by it.
use std::sync::{OnceLock, RwLock};

/// What gives the advance, in points, of glyph `glyph` of face `id` at
/// `size` points; `None` keeps the font's own.
pub type Hook = fn(fontdb::ID, u16, f32) -> Option<f32>;

static HOOK: OnceLock<Hook> = OnceLock::new();
static FACES: RwLock<Vec<fontdb::ID>> = RwLock::new(Vec::new());

/// Faces `faces` take their advances from `hook` (the first hook stays).
/// Before their fonts are first used: a font made earlier keeps its own.
pub fn register(faces: &[fontdb::ID], hook: Hook) {
    let _ = HOOK.set(hook);
    if let Ok(mut known) = FACES.write() {
        known.extend_from_slice(faces);
    }
}

/// Whether face `id`'s advances come from the platform.
pub(crate) fn has(id: fontdb::ID) -> bool {
    FACES.read().is_ok_and(|known| known.contains(&id))
}

/// The platform's advance of glyph `glyph` of face `id` at `size` points.
pub(crate) fn advance(id: fontdb::ID, glyph: u16, size: f32) -> Option<f32> {
    HOOK.get().and_then(|hook| hook(id, glyph, size))
}

#[cfg(test)]
mod tests {
    use crate::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};

    #[test]
    fn a_registered_faces_glyphs_measure_and_lay_out_by_the_platforms_advance() {
        // Only at an odd size, so other tests' text in the face is untouched.
        const SIZE: f32 = 13.37;
        fn hook(_: fontdb::ID, _: u16, size: f32) -> Option<f32> {
            (size == SIZE).then_some(7.)
        }
        let mut fonts = FontSystem::new();
        let common = ["Helvetica", "Arial", "DejaVu Sans", "Liberation Sans", "Noto Sans"];
        let Some(face) = fonts.db().faces().find_map(|face| {
            let (family, _) = face.families.first()?;
            common.contains(&family.as_str()).then(|| family.clone())
        }) else {
            return;
        };
        // Every face of the family: the one the text takes is among them.
        let ids: Vec<_> = (fonts.db().faces())
            .filter(|f| f.families.first().is_some_and(|(family, _)| *family == face))
            .map(|f| f.id)
            .collect();
        super::register(&ids, hook);
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(SIZE, 20.));
        let attrs = Attrs::new().family(Family::Name(&face));
        buffer.set_text("aaa", &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut fonts, false);
        let run = buffer.layout_runs().next().unwrap();
        assert!(run.glyphs.iter().all(|glyph| ids.contains(&glyph.font_id)));
        let xs: Vec<f32> = run.glyphs.iter().map(|glyph| glyph.x).collect();
        assert_eq!(xs, [0., 7., 14.]);
        assert_eq!(run.line_w, 21.);
    }
}
