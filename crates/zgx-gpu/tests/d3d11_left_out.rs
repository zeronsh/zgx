//! What Direct3D 11 (`ZGX_D3D11=1`, the `d3d11` feature) leaves out of a
//! frame rather than fail it, and what still fails it
//! (`GpuRenderer::d3d11_leave_out`): a drop shadow is left out; a backdrop
//! blur and a layer shader reading the backdrop (glass) draw their box
//! opaque in its own colour, as the system's Reduce transparency draws
//! glass; a content blur and a blend mode fail the frame, naming it. Each
//! kind left out is named once. `ZGX_D3D11` is read once a process, so this
//! test is one of its own, setting it first.
#[cfg(windows)]
use zgx::scene::*;
#[cfg(windows)]
use zgx_gpu::GpuRenderer;

#[cfg(windows)]
const SIZE: u32 = 240;
#[cfg(windows)]
const FILL: Color = Color(30, 30, 40, 160);

/// Blue and orange stripes under a 120 by 80 box at (60, 80), translucent
/// with rounded corners, `set` given the box.
#[cfg(windows)]
fn draw(set: impl FnOnce(&mut Scene, NodeId)) -> Result<Vec<u8>, String> {
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    for i in 0..12 {
        let stripe = scene.append(
            root,
            NodeKind::Rect(if i % 2 == 0 {
                Color(30, 60, 200, 255)
            } else {
                Color(230, 130, 30, 255)
            }),
            Style {
                width: Some(SIZE as f32),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.set_transform(
            stripe,
            Transform {
                x: 0.,
                y: i as f32 * 20.,
            },
        );
    }
    let id = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                radius: 8.,
                fill: FILL,
                ..Default::default()
            },
        },
        Style {
            width: Some(120.),
            height: Some(80.),
            ..Default::default()
        },
    );
    scene.set_transform(id, Transform { x: 60., y: 80. });
    set(&mut scene, id);
    let report = scene.flush();
    // A frame failing panics in a debug build, and errs in a release one.
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gpu.render(&scene, &report.damage)
    }));
    match rendered {
        Ok(Ok(_)) => Ok(gpu.readback().unwrap()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(panic) => Err(panic
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "a panic".into())),
    }
}

/// The pixel at (`x`, `y`).
#[cfg(windows)]
fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    pixels[i..i + 4].try_into().unwrap()
}

/// Every pixel inside the box, clear of its corners, is its fill opaque.
#[cfg(windows)]
fn opaque_box(pixels: &[u8], what: &str) {
    for y in 90..150 {
        for x in 70..170 {
            let [r, g, b, a] = at(pixels, x, y);
            let near = |value: u8, want: u8| value.abs_diff(want) <= 1;
            assert!(
                near(r, FILL.0) && near(g, FILL.1) && near(b, FILL.2) && a == 255,
                "{what}: ({x}, {y}) is {:?}, not the box's colour opaque",
                [r, g, b, a]
            );
        }
    }
}

#[cfg(windows)]
fn main() {
    // SAFETY: Set before anything else runs, the test's only thread.
    unsafe { std::env::set_var("ZGX_D3D11", "1") };
    let plain = draw(|_, _| {}).expect("a plain frame");

    // A drop shadow: left out, the frame as without it.
    let shadowed = draw(|scene, id| {
        scene.set_drop_shadow(
            id,
            Some(DropShadow {
                offset: Transform { x: 0., y: 20. },
                blur: 0.,
                color: Color(0, 0, 0, 255),
            }),
        )
    })
    .expect("a frame with a drop shadow draws");
    // (Against the box a layer without one, as a drop shadow makes it.)
    let layered = draw(|scene, id| scene.set_isolated(id, true)).expect("a layer draws");
    assert!(
        shadowed == layered,
        "a drop shadow left out leaves the frame as it was"
    );

    // A backdrop blur: the box opaque in its own colour.
    let blurred = draw(|scene, id| {
        scene.set_effects(
            id,
            Effects {
                blur_radius: 12.,
                ..Default::default()
            },
        )
    })
    .expect("a frame with a backdrop blur draws");
    opaque_box(&blurred, "a backdrop blur");
    // Outside it, the stripes as they were.
    assert_eq!(at(&blurred, 20, 30), at(&plain, 20, 30));

    // A layer shader reading the backdrop (glass): the same.
    let glass = zgx::decoration::FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> {
    return mix(zgx_backdrop(input.p), input.color, 0.5);
}",
        None,
    );
    let read = draw(|scene, id| scene.set_layer_shader(id, Some(glass)))
        .expect("a frame with a layer shader reading the backdrop draws");
    opaque_box(&read, "a layer shader reading the backdrop");

    // A content blur and a blend mode: the frame fails, naming it.
    let blur = draw(|scene, id| scene.set_filter_blur(id, 4.)).unwrap_err();
    assert!(blur.contains("content blur"), "{blur}");
    let blend = draw(|scene, id| scene.set_blend(id, BlendMode::Multiply)).unwrap_err();
    assert!(blend.contains("blend mode"), "{blend}");

    let left_out = GpuRenderer::debug_d3d11_left_out();
    for kind in [
        "drop shadows",
        "backdrop blurs (drawn opaque)",
        "backdrop reads (glass drawn opaque)",
    ] {
        assert!(left_out.contains(&kind), "{kind} not named: {left_out:?}");
    }
    println!("d3d11_left_out: {left_out:?}");
}

#[cfg(not(windows))]
fn main() {}
