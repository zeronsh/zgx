#![cfg(target_os = "macos")]
use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CFString, CFType};
use objc2_core_video::*;
use std::{ptr::NonNull, rc::Rc};
use zgx::{
    native_surface::NativeSurface,
    scene::{NodeKind, Rect, Scene, Style},
};
use zgx_gpu::GpuRenderer;
fn frame(nv12: bool) -> Rc<NativeSurface> {
    let empty = CFDictionary::<CFType, CFType>::empty();
    // SAFETY: reads two CoreVideo constants, immutable CFStrings it exports.
    let attributes = CFDictionary::<CFString, CFType>::from_slices(
        &[unsafe { kCVPixelBufferIOSurfacePropertiesKey }, unsafe {
            kCVPixelBufferMetalCompatibilityKey
        }],
        &[empty.as_ref(), CFBoolean::new(true).as_ref()],
    );
    let mut pointer = std::ptr::null_mut();
    // SAFETY: attribute keys and values of the types CoreVideo documents (a
    // dictionary, a boolean), and an out-pointer to a local it writes.
    assert_eq!(
        unsafe {
            CVPixelBufferCreate(
                None,
                4,
                4,
                if nv12 {
                    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
                } else {
                    kCVPixelFormatType_32BGRA
                },
                Some(attributes.as_opaque()),
                NonNull::from(&mut pointer),
            )
        },
        0
    );
    // SAFETY: a successful Create returns a non-null +1 reference, which
    // `CFRetained` takes over.
    let buffer = unsafe { CFRetained::from_raw(NonNull::new(pointer).unwrap()) };
    // SAFETY: the base address is used only while locked: each NV12 plane's
    // `rows * stride` bytes, or the 4x4 BGRA rows `stride` bytes apart, are
    // within the buffer. Unlocked, its pixels are written (CPU writes,
    // finished) and nothing changes them again, premultiplied (opaque) BGRA
    // or full-range NV12, IOSurface-backed and Metal-compatible as
    // `from_pixel_buffer` requires.
    unsafe {
        assert_eq!(
            CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::empty()),
            0
        );
        if nv12 {
            for plane in 0..2 {
                let rows = CVPixelBufferGetHeightOfPlane(&buffer, plane);
                let stride = CVPixelBufferGetBytesPerRowOfPlane(&buffer, plane);
                std::ptr::write_bytes(
                    CVPixelBufferGetBaseAddressOfPlane(&buffer, plane).cast::<u8>(),
                    128,
                    rows * stride,
                );
            }
        } else {
            let pointer = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRow(&buffer);
            for y in 0..4 {
                for x in 0..4 {
                    std::ptr::copy_nonoverlapping(
                        [20, 40, 220, 255].as_ptr(),
                        pointer.add(y * stride + x * 4),
                        4,
                    );
                }
            }
        }
        assert_eq!(
            CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::empty()),
            0
        );
        Rc::new(NativeSurface::from_pixel_buffer(buffer).unwrap())
    }
}
#[test]
fn metal_imports_native_bgra_and_nv12_reuses_frames_and_releases_on_teardown() {
    for nv12 in [false, true] {
        let frame = frame(nv12);
        let weak = Rc::downgrade(&frame);
        let mut scene = Scene::new(32., 32.);
        let node = scene.append(
            scene.root(),
            NodeKind::NativeSurface(frame),
            Style {
                width: Some(32.),
                height: Some(32.),
                ..Default::default()
            },
        );
        let mut gpu = GpuRenderer::new(32, 32).unwrap();
        let damage = scene.flush().damage;
        let stats = gpu.render(&scene, &damage).unwrap();
        assert_eq!(stats.image_uploads, 0);
        assert_eq!(stats.native_surface_imports, 1);
        assert_eq!(stats.native_surface_conversions, usize::from(nv12));
        let pixels = gpu.readback().unwrap();
        let pixel = &pixels[(16 * 32 + 16) * 4..(16 * 32 + 16) * 4 + 4];
        if nv12 {
            assert!(
                pixel[..3].iter().all(|c| (124..=132).contains(c)),
                "{pixel:?}"
            );
        } else {
            assert_eq!(pixel, &[220, 40, 20, 255]);
        }
        let stats = gpu.render(&scene, &[Rect::new(0., 0., 32., 32.)]).unwrap();
        assert_eq!(stats.native_surface_imports, 0);
        assert_eq!(stats.native_surface_conversions, 0);
        scene.remove(node);
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        drop(gpu);
        assert!(
            weak.upgrade().is_none(),
            "window teardown retained native frame"
        );
    }
}

#[test]
fn native_surface_component_retains_nodes_fits_padding_and_releases_bindings() {
    use zgx::{compose::prelude::*, widgets::Ui};
    let mut ui = Ui::new(100., 80.);
    let source = ui.signal(frame(false));
    let read = source.clone();
    let view = ui.mount(
        native_surface_signal("frame", move || read.get())
            .w(80.)
            .h(60.)
            .p(10.),
    );
    ui.prepare_frame();
    let viewport = ui.scene.borrow().children(view.node())[0];
    let image = ui.scene.borrow().children(viewport)[0];
    assert_eq!(
        ui.scene.borrow().bounds(viewport),
        Rect::new(10., 10., 60., 40.)
    );
    assert_eq!(
        ui.scene.borrow().bounds(image),
        Rect::new(20., 10., 40., 40.)
    );
    ui.scene.borrow_mut().flush();
    source.set(frame(true));
    ui.prepare_frame();
    assert_eq!(ui.scene.borrow_mut().flush().layout_nodes, 0);
    assert_eq!(ui.scene.borrow().children(viewport), &[image]);
    ui.remove(view.node());
    source.set(frame(false));
    assert!(!ui.scene.borrow().contains(image));
}
unsafe extern "C" {
    fn CVPixelBufferGetIOSurface(buffer: *const std::ffi::c_void) -> *const std::ffi::c_void;
}
#[link(name = "IOSurface", kind = "framework")]
unsafe extern "C" {
    fn IOSurfaceGetID(surface: *const std::ffi::c_void) -> u32;
    fn IOSurfaceLookup(id: u32) -> *const std::ffi::c_void;
}
/// Whether the IOSurface `id` still exists.
fn surface_exists(id: u32) -> bool {
    unsafe extern "C" {
        fn CFRelease(object: *const std::ffi::c_void);
    }
    // SAFETY: Lookup returns a +1 reference or null.
    unsafe {
        let surface = IOSurfaceLookup(id);
        if !surface.is_null() {
            CFRelease(surface);
        }
        !surface.is_null()
    }
}
/// A window may go idle on the frame that removed a surface: no frame comes
/// after to let CoreVideo's cache of it go, so going idle does. (A video
/// player's last picture otherwise stayed for the window's life.)
#[test]
fn a_surface_gone_from_the_scene_is_let_go_as_the_window_rests() {
    let frame = frame(false);
    // SAFETY: The buffer is IOSurface-backed; its surface lives as long.
    let id = unsafe {
        IOSurfaceGetID(CVPixelBufferGetIOSurface(
            std::ptr::from_ref(frame.pixel_buffer()).cast(),
        ))
    };
    let mut scene = Scene::new(32., 32.);
    let node = scene.append(
        scene.root(),
        NodeKind::NativeSurface(frame),
        Style {
            width: Some(32.),
            height: Some(32.),
            ..Default::default()
        },
    );
    let mut gpu = GpuRenderer::new(32, 32).unwrap();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    // Shown, resting keeps it: the window may draw it again.
    gpu.trim();
    assert!(surface_exists(id), "let go of a surface still shown");
    scene.remove(node);
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    // As a window rests, a while after its last frame: CoreVideo's cache
    // keeps a texture a second, whatever a flush asks.
    std::thread::sleep(std::time::Duration::from_millis(1200));
    gpu.trim();
    assert!(
        !surface_exists(id),
        "a window at rest kept a surface no node shows"
    );
}
/// A window that shows only surfaces (a video player, nothing else drawn as
/// a picture) lets go of each it no longer shows as it draws on, not only
/// once it rests: playing, it shows a new one every frame.
#[test]
fn a_surface_replaced_is_let_go_while_the_window_draws_on() {
    let id_of = |frame: &NativeSurface| unsafe {
        // SAFETY: The buffer is IOSurface-backed; its surface lives as long.
        IOSurfaceGetID(CVPixelBufferGetIOSurface(
            std::ptr::from_ref(frame.pixel_buffer()).cast(),
        ))
    };
    let first = frame(false);
    let id = id_of(&first);
    let mut scene = Scene::new(32., 32.);
    let style = Style {
        width: Some(32.),
        height: Some(32.),
        ..Default::default()
    };
    let node = scene.append(scene.root(), NodeKind::NativeSurface(first), style);
    let mut gpu = GpuRenderer::new(32, 32).unwrap();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    scene.set_kind(node, NodeKind::NativeSurface(frame(false)));
    // Frames on, a while past CoreVideo's second, and no rest between.
    let until = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    while std::time::Instant::now() < until {
        scene.flush();
        gpu.render(&scene, &[Rect::new(0., 0., 32., 32.)]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    assert!(
        !surface_exists(id),
        "a window drawing on kept a surface no node shows"
    );
    // It polled and flushed to let go of it, and stops a second and a half
    // after: a window drawing the same surfaces does nothing more.
    let draw_for = |gpu: &mut GpuRenderer, scene: &mut Scene, millis| {
        let until = std::time::Instant::now() + std::time::Duration::from_millis(millis);
        while std::time::Instant::now() < until {
            scene.flush();
            gpu.render(scene, &[Rect::new(0., 0., 32., 32.)]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    let collected = gpu.debug_cache_stats().surface_collections;
    assert!(collected > 0, "it let go of a surface without collecting");
    draw_for(&mut gpu, &mut scene, 600);
    let stopped = gpu.debug_cache_stats().surface_collections;
    draw_for(&mut gpu, &mut scene, 500);
    assert_eq!(
        gpu.debug_cache_stats().surface_collections,
        stopped,
        "it went on collecting past a second and a half"
    );
}

/// A readback's pixel buffers go back at rest, but for the one still held.
/// CoreVideo's texture cache keeps a texture of each buffer the pool still
/// has, and the buffer with it: flushed before the pool (each frame), it
/// keeps them, so a paused video player held two (7 MB at 1280 by 720) for
/// as long as it was paused. Read as this process's IOSurface memory: one
/// given back can still be looked up for a while, and the footprint as a
/// whole moves by more with the allocator's own returns.
#[test]
fn a_readbacks_buffers_go_back_at_rest_but_the_one_held() {
    unsafe extern "C" {
        fn getpid() -> i32;
    }
    let surfaces = || {
        let out = std::process::Command::new("footprint")
            // SAFETY: getpid has no preconditions.
            .args(["-p", &unsafe { getpid() }.to_string()])
            .output()
            .expect("footprint");
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let line = text
            .lines()
            .find(|line| line.trim_end().ends_with(" IOSurface"))
            .unwrap_or_else(|| panic!("no IOSurface line in {text}"));
        let mut words = line.split_whitespace();
        let (number, unit) = (words.next().unwrap(), words.next().unwrap());
        let scale = match unit {
            "B" => 1.,
            "KB" => 1024.,
            "MB" => 1024. * 1024.,
            "GB" => 1024. * 1024. * 1024.,
            _ => panic!("{line}"),
        };
        (number.parse::<f64>().unwrap() * scale) as i64
    };
    let (width, height) = (1280u32, 720u32);
    let frame_bytes = (width * height * 4) as i64;
    let whole = [Rect::new(0., 0., width as f32, height as f32)];
    let mut scene = Scene::new(width as f32, height as f32);
    scene.flush();
    let mut gpu = GpuRenderer::new(width, height).unwrap();
    let mut staging = zgx_gpu::SurfaceReadback::default();
    // Frames drawn as a window shows them: each held until the next is.
    let mut held: std::collections::VecDeque<zgx_gpu::PixelBuffer> = Default::default();
    for _ in 0..6 {
        gpu.render(&scene, &whole).unwrap();
        held.push_back(gpu.readback_surface(&mut staging).unwrap());
        if held.len() > 2 {
            held.pop_front();
        }
    }
    // Paused: the last one shown still held, the rest given up.
    let shown = held.pop_back().unwrap();
    drop(held);
    // At rest (a player's comes 5 s after its last frame).
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let before = surfaces();
    assert!(before >= 2 * frame_bytes, "{before} bytes of surfaces held");
    staging.release_unused();
    let after = surfaces();
    assert!(
        after < 2 * frame_bytes,
        "{after} bytes of surfaces held at rest (from {before}), a frame being {frame_bytes}"
    );
    drop(shown);
}
