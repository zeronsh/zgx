//! Windows: draw.wgsl's stages compiled ahead of time (`dx12_precompiled`)
//! are the bytecode wgpu's Direct3D 12 compiles itself with FXC, and draw
//! the same frame byte for byte. A process of its own (Cargo.toml): it
//! sets the environment wgpu's instance reads, `WGPU_DEBUG=0` for FXC's
//! release flags (the bytecode's) in a debug build, and an empty shader cache.
#![cfg(all(windows, zgx_wgpu_hal_patched))]
use std::sync::Arc;
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

const WIDTH: u32 = 120;
const HEIGHT: u32 = 80;

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}

/// Quads of every pipeline a frame draws with offscreen: opaque and
/// translucent plain boxes (`vs_basic`, `fs_basic`), and a rounded box with
/// a border and a shadow, an image and text (`vs`, `fs`).
fn scene() -> Scene {
    let mut scene = Scene::new(WIDTH as f32, HEIGHT as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene.append(
        scene.root(),
        NodeKind::Rect(Color(30, 40, 50, 255)),
        fixed(WIDTH as f32, HEIGHT as f32),
    );
    let translucent = scene.append(
        scene.root(),
        NodeKind::Rect(Color(200, 120, 40, 140)),
        fixed(40., 30.),
    );
    scene.set_transform(translucent, Transform { x: 70., y: 40. });
    let rounded = scene.append(
        scene.root(),
        NodeKind::Quad(QuadStyle {
            decoration: None,
            fill: Color(220, 30, 60, 255),
            radius: 9.,
            border_color: Color(40, 220, 90, 255),
            border_width: 2.5,
            shadow: Some(BoxShadow {
                color: Color(0, 0, 255, 128),
                offset: Transform { x: -4., y: 3. },
                blur_radius: 3.,
                spread: 1.,
            }),
            border_over: false,
            outline: None,
        }),
        fixed(30., 26.),
    );
    scene.set_transform(rounded, Transform { x: 14.5, y: 8.25 });
    let image = Arc::new(zgx::image::ImageData::new(2, 2, [255, 255, 0, 255].repeat(4)).unwrap());
    let picture = scene.append(scene.root(), NodeKind::Image(image), fixed(12., 12.));
    scene.set_transform(picture, Transform { x: 60., y: 8. });
    let text = scene.append(
        scene.root(),
        NodeKind::Text {
            text: "Quads العربية".into(),
            color: Color(255, 255, 255, 255),
            font_size: 13.,
        },
        fixed(110., 20.),
    );
    scene.set_transform(text, Transform { x: 4., y: 52. });
    scene
}

fn frame(precompiled: bool) -> Vec<u8> {
    zgx_gpu::debug_precompiled_shaders(precompiled);
    let mut gpu = GpuRenderer::new(WIDTH, HEIGHT).expect("an adapter");
    assert_eq!(gpu.debug_quads_precompiled(), precompiled);
    let mut scene = scene();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    gpu.readback().unwrap()
}

#[test]
fn precompiled_quads_are_wgpus_own_bytecode_and_draw_its_frame() {
    let cache = std::env::temp_dir().join(format!("zgx-precompiled-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    // SAFETY: the process's only test, before anything reads them.
    unsafe {
        std::env::set_var("LOCALAPPDATA", &cache);
        std::env::set_var("WGPU_DEBUG", "0");
        std::env::set_var("WGPU_DX12_COMPILER", "fxc");
    }
    let wgpus = frame(false);
    // Every stage the frame drew with, wgpu compiled to the same bytes and
    // kept them in its cache (the files hold FXC's bytecode whole).
    let kept: Vec<Vec<u8>> = std::fs::read_dir(cache.join("zgx\\shaders"))
        .expect("wgpu's shader cache")
        .map(|file| std::fs::read(file.unwrap().path()).unwrap())
        .collect();
    let found: Vec<&str> = zgx_gpu::debug_precompiled_stages()
        .into_iter()
        .filter(|(_, code)| {
            !code.is_empty()
                && kept
                    .iter()
                    .any(|file| file.windows(code.len()).any(|at| at == *code))
        })
        .map(|(entry, _)| entry)
        .collect();
    for entry in ["vs", "fs", "vs_basic", "fs_basic"] {
        assert!(
            found.contains(&entry),
            "`{entry}`'s bytecode isn't what wgpu compiled (found {found:?} in {} files)",
            kept.len()
        );
    }
    let precompiled = frame(true);
    assert!(
        precompiled == wgpus,
        "{} of {} bytes differ",
        precompiled
            .iter()
            .zip(&wgpus)
            .filter(|(a, b)| a != b)
            .count(),
        wgpus.len()
    );
    let _ = std::fs::remove_dir_all(&cache);
}
