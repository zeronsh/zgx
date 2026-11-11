//! The GPU-view scenes `d3d11_parity.rs` compares between Direct3D 11's view
//! host and Direct3D 12, drawn here on this platform's own renderer: each
//! scene's view must draw (`support/view_painters.rs` asserts it), at 1x
//! and 2x, and draw the same twice.
#[path = "support/view_painters.rs"]
mod view_painters;

#[test]
fn every_view_scene_draws_and_draws_the_same_again() {
    for scale in [1., 2.] {
        let frames = view_painters::view_scene_frames(scale);
        if frames.is_empty() {
            return; // no GPU, and none required
        }
        assert_eq!(frames.len(), view_painters::VIEW_SCENE_NAMES.len());
        // `ZGX_VIEW_SCENES_DIR`: each frame as a PNG there, to look at.
        if let Some(dir) = std::env::var_os("ZGX_VIEW_SCENES_DIR") {
            let (w, h) = view_painters::VIEW_SCENE_SIZE;
            for (index, frame) in frames.iter().enumerate() {
                let path = std::path::Path::new(&dir).join(format!("{index}-{scale}x.png"));
                image::save_buffer(
                    &path,
                    frame,
                    (w * scale) as u32,
                    (h * scale) as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
        }
        let again = view_painters::view_scene_frames(scale);
        for ((name, first), second) in view_painters::VIEW_SCENE_NAMES
            .iter()
            .zip(&frames)
            .zip(&again)
        {
            assert!(
                first == second,
                "{name} at {scale}x drew differently the second time"
            );
        }
    }
}
