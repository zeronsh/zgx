//! A frame the host shows as it is (`ScriptHost::surface`): a
//! `ui.image { src = "host:<name>" }` draws it from its own storage while the
//! host sets one, follows each replacement, and shows the host's image once
//! it's let go. A background reads pixels, so it shows the image throughout.
#![cfg(target_os = "macos")]
mod common;
use common::Harness;
use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CFString, CFType};
use objc2_core_video::*;
use std::{ptr::NonNull, rc::Rc, sync::Arc};
use zgx::{image::ImageData, native_surface::NativeSurface, scene::NodeKind};

/// A 4x4 BGRA frame, IOSurface-backed and Metal-compatible, its pixels clear.
fn frame() -> Rc<NativeSurface> {
    let empty = CFDictionary::<CFType, CFType>::empty();
    // SAFETY: reads two CoreVideo constants, immutable CFStrings it exports.
    let attributes = CFDictionary::<CFString, CFType>::from_slices(
        &[unsafe { kCVPixelBufferIOSurfacePropertiesKey }, unsafe {
            kCVPixelBufferMetalCompatibilityKey
        }],
        &[empty.as_ref(), CFBoolean::new(true).as_ref()],
    );
    let mut pointer = std::ptr::null_mut();
    // SAFETY: attribute keys and values of the types CoreVideo documents, and
    // an out-pointer to a local it writes.
    assert_eq!(
        unsafe {
            CVPixelBufferCreate(
                None,
                4,
                4,
                kCVPixelFormatType_32BGRA,
                Some(attributes.as_opaque()),
                NonNull::from(&mut pointer),
            )
        },
        0
    );
    // SAFETY: a successful Create returns a non-null +1 reference.
    let buffer = unsafe { CFRetained::from_raw(NonNull::new(pointer).unwrap()) };
    // SAFETY: the base address is written only while locked, `rows * stride`
    // bytes within the buffer; unlocked, nothing changes it again: clear,
    // premultiplied BGRA, as `from_pixel_buffer` requires.
    unsafe {
        assert_eq!(
            CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::empty()),
            0
        );
        let stride = CVPixelBufferGetBytesPerRow(&buffer);
        std::ptr::write_bytes(
            CVPixelBufferGetBaseAddress(&buffer).cast::<u8>(),
            0,
            4 * stride,
        );
        CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::empty());
        Rc::new(NativeSurface::from_pixel_buffer(buffer).unwrap())
    }
}

/// The frames and images the scene draws, by their ids.
fn drawn(h: &Harness) -> (Vec<u64>, Vec<u64>) {
    let scene = h.ui.scene.borrow();
    let (mut frames, mut images) = (Vec::new(), Vec::new());
    let mut walk = vec![scene.root()];
    while let Some(id) = walk.pop() {
        match scene.kind(id) {
            NodeKind::NativeSurface(frame) => frames.push(frame.id()),
            NodeKind::Image(image) => images.push(image.id()),
            _ => {}
        }
        walk.extend_from_slice(scene.children(id));
    }
    (frames, images)
}

#[test]
fn a_host_frame_shows_as_it_is_follows_its_replacement_and_gives_way_to_its_image() {
    let h = Harness::inline(
        r#"return ui.column { w = 8, h = 8, ui.image { src = "host:clip", w = 4, h = 4 } }"#,
    );
    let image = Arc::new(ImageData::new(4, 4, vec![255; 64]).unwrap());
    h.host.image("clip", Some(image.clone()));
    h.frames(1);
    assert_eq!(
        drawn(&h),
        (vec![], vec![image.id()]),
        "the image, before a frame"
    );

    let first = frame();
    h.host.surface("clip", Some(first.clone()));
    h.frames(1);
    assert_eq!(drawn(&h).0, vec![first.id()], "the frame as it is");

    let second = frame();
    h.host.surface("clip", Some(second.clone()));
    h.frames(1);
    assert_eq!(drawn(&h).0, vec![second.id()], "its replacement");
    drop(first);

    h.host.surface("clip", None);
    h.frames(1);
    assert_eq!(
        drawn(&h),
        (vec![], vec![image.id()]),
        "the image, once let go"
    );
    assert_eq!(Rc::strong_count(&second), 1, "a frame let go is still held");
    assert!(h.host.hosted_surface("clip").is_none());
}

#[test]
fn a_background_or_a_dithered_picture_shows_the_hosts_image_not_its_frame() {
    let h = Harness::inline(
        r#"return ui.column { w = 8, h = 8,
            ui.image { src = "host:clip", w = 4, h = 4, dither = 1 },
            ui.div { w = 4, h = 4, bg_image = "host:clip" } }"#,
    );
    let image = Arc::new(ImageData::new(4, 4, vec![255; 64]).unwrap());
    h.host.image("clip", Some(image));
    h.host.surface("clip", Some(frame()));
    h.frames(1);
    assert!(
        drawn(&h).0.is_empty(),
        "a picture that reads pixels drew the frame"
    );
}
