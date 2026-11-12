//! What Direct3D 11 (`ZGX_D3D11=1`, the `d3d11` feature) leaves out of a
//! frame rather than fail it, and what still fails it
//! (`GpuRenderer::d3d11_leave_out`): a drop shadow, a backdrop blur (not
//! its box opaque), a layer shader reading the backdrop, a content blur and
//! a blend mode all draw, and none of them is left out. `ZGX_D3D11` is read once a process, so this
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

/// Whether every pixel inside the box, clear of its corners, is its fill
/// opaque (as Direct3D 11 once drew a backdrop blur).
#[cfg(windows)]
fn opaque_box(pixels: &[u8]) -> bool {
    (90..150).all(|y| {
        (70..170).all(|x| {
            let [r, g, b, a] = at(pixels, x, y);
            let near = |value: u8, want: u8| value.abs_diff(want) <= 1;
            near(r, FILL.0) && near(g, FILL.1) && near(b, FILL.2) && a == 255
        })
    })
}

#[cfg(windows)]
fn main() {
    // SAFETY: Set before anything else runs, the test's only thread.
    unsafe { std::env::set_var("ZGX_D3D11", "1") };
    let plain = draw(|_, _| {}).expect("a plain frame");

    // A drop shadow: drawn (d3d11_parity compares it with Direct3D 12's).
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
    assert!(shadowed != layered, "the drop shadow drew nothing");

    // A backdrop blur draws (d3d11_parity compares it with Direct3D 12's):
    // translucent over the stripes it blurs, not the box opaque.
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
    assert!(
        !opaque_box(&blurred),
        "the backdrop blur drew as the opaque box"
    );
    assert_ne!(blurred, plain, "the backdrop blur drew nothing");
    // Outside it, the stripes as they were.
    assert_eq!(at(&blurred, 20, 30), at(&plain, 20, 30));

    // A layer shader reading the backdrop (glass): drawn over a copy of it
    // (d3d11_parity compares it with Direct3D 12's), so not the opaque box.
    let glass = zgx::decoration::FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> {
    return mix(zgx_backdrop(input.p), input.color, 0.5);
}",
        None,
    );
    let read = draw(|scene, id| scene.set_layer_shader(id, Some(glass)))
        .expect("a frame with a layer shader reading the backdrop draws");
    assert_ne!(read, plain, "the glass drew nothing");
    let opaque = (90..150).all(|y| {
        (70..170).all(|x| {
            let [r, g, b, a] = at(&read, x, y);
            [r, g, b] == [FILL.0, FILL.1, FILL.2] && a == 255
        })
    });
    assert!(!opaque, "the glass drew as the opaque box");
    // Glass reading at an offset, its reach past the box (as zgx-fx's
    // glass reads): a stripe away, so neither the box as painted (its layer
    // without the shader, `layered`) nor the glass reading straight under it.
    let offset = zgx::decoration::FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> {
    return mix(zgx_backdrop(input.p + vec2(0., 10.)), input.color, 0.5);
}",
        None,
    );
    let shifted = draw(|scene, id| {
        scene.set_layer_shader(id, Some(offset));
        scene.set_layer_extent(id, 0., 12.);
    })
    .expect("a frame with glass reading at an offset draws");
    assert_ne!(
        shifted, layered,
        "the offset glass drew as the box as painted"
    );
    assert_ne!(shifted, read, "the offset glass read straight under it");

    // A content blur and a blend mode draw (d3d11_parity compares them
    // with Direct3D 12's).
    let blurred =
        draw(|scene, id| scene.set_filter_blur(id, 4.)).expect("a frame with a content blur draws");
    assert_ne!(blurred, plain, "the content blur drew nothing");
    let blended = draw(|scene, id| scene.set_blend(id, BlendMode::Multiply))
        .expect("a frame with a blend mode draws");
    assert_ne!(blended, plain, "the blend mode drew nothing");

    // A layer shader with inputs (a pass): left out, its layer composited
    // as painted (Direct3D 11 runs no passes yet), named once.
    let with_pass = zgx::decoration::FillShader::with_inputs(
        "fn layer(input: LayerInput) -> vec4<f32> {
    return vec4(zgx_count(vec2(0.5)).r, 0.0, 0.0, 1.0);
}",
        None,
        zgx::decoration::ShaderInputs {
            passes: vec![zgx::decoration::ShaderPass {
                name: "count".into(),
                source: "fn run(input: PassInput) -> vec4<f32> { return vec4(1.0); }".into(),
                size: zgx::decoration::PassSize::Pixels(4, 4),
                format: zgx::decoration::PassFormat::Rgba16Float,
                feedback: false,
                steps: 1,
                once: false,
            }],
            ..Default::default()
        },
    );
    let passed = draw(|scene, id| scene.set_layer_shader(id, Some(with_pass)))
        .expect("a frame with a layer shader with inputs draws");
    assert_eq!(
        passed, layered,
        "a layer shader with inputs draws its layer as painted"
    );

    // A GPU view draws (`view_host`): its painter's colour, on the views'
    // own Direct3D 12 device on Direct3D 11's adapter (on a machine without
    // a GPU, WARP, which wgpu's own list of adapters leaves out).
    {
        use zgx::{compose::prelude::*, widgets::Ui};
        use zgx_gpu::viewport::{Viewport, ViewportFrame, view};
        struct Clear;
        impl Viewport for Clear {
            fn render(&mut self, frame: &mut ViewportFrame<'_>) {
                frame
                    .encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("clear"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: frame.target,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.2,
                                    g: 0.8,
                                    b: 0.4,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
            }
        }
        let mut gpu = GpuRenderer::new(64, 64).unwrap();
        let painter = view(Clear);
        let mut ui = Ui::new(64., 64.);
        ui.mount(gpu_view("view", &painter).size(64., 64.));
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage)
            .expect("a frame with a GPU view draws");
        let pixels = gpu.readback().unwrap();
        let middle = &pixels[(32 * 64 + 32) * 4..][..4];
        assert!(
            middle[1] > 180 && middle[0] < 90 && middle[2] < 140,
            "the GPU view didn't draw its painter's colour: {middle:?}"
        );
    }

    let left_out = GpuRenderer::debug_d3d11_left_out();
    assert!(
        !left_out.contains(&"images a GPU view draws"),
        "a GPU view was left out: {left_out:?}"
    );
    let kind = "layer shaders with inputs (passes, pictures, GPU views): drawn as painted";
    assert_eq!(left_out, [kind], "only shaders with inputs are left out");
    println!("d3d11_left_out: {left_out:?}");
}

#[cfg(not(windows))]
fn main() {}
