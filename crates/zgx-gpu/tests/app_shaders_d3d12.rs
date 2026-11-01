//! An app's own shaders on Direct3D 12 with FXC: their fragment stage
//! compiled by FXC in a process of its own and handed to wgpu as bytecode
//! (`dx12_fill`, `fxc_child`) draws exactly what wgpu's own build (naga,
//! then FXC in the process) draws, byte for byte. The shaders between them
//! touch every binding an app's shader can: a fill reads its uniforms and the
//! regions buffer, a layer shader samples the layer's texture through the
//! sampler, and blended it reads the backdrop. A wgpu update moving the
//! registers of its root signature fails here, rather than drawing wrong.
//! The in-process build is also the path taken where wgpu offers no
//! passthrough. Run with an empty shader cache (CI's `LOCALAPPDATA`), so the
//! child compiles every shader.
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use zgx::decoration::{Background, Decoration, FillShader};
#[cfg(windows)]
use zgx::scene::*;
#[cfg(windows)]
use zgx_gpu::GpuRenderer;

#[cfg(windows)]
const SIZE: u32 = 96;

#[cfg(windows)]
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}

/// The scene, its shaders made anew (each renderer builds them again).
#[cfg(windows)]
fn scene() -> Scene {
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    // A fill reading its uniforms (`params`) and position.
    let fill = FillShader::new(
        "fn fill(input: FillInput) -> vec4<f32> {
            let t = fract((input.p.x + input.p.y) / 7.0 + input.params[0].x);
            return vec4<f32>(0.2 + 0.7 * t, input.params[1].y, 0.9 - 0.6 * t, 1.0);
        }",
        None,
    );
    let filled = scene.append(
        scene.root(),
        NodeKind::Quad(QuadStyle {
            decoration: Some(Arc::new(Decoration {
                background: Some(Background::Shader(fill)),
                ..Default::default()
            })),
            ..Default::default()
        }),
        fixed(60., 40.),
    );
    scene.set_shader_uniforms(
        filled,
        zgx::decoration::ShaderUniforms {
            time: 0.,
            params: [[0.35, 0., 0., 0.], [0., 0.6, 0., 0.], [0.; 4], [0.; 4]],
        },
    );
    // A layer shader sampling its layer, and the same blended over the
    // backdrop.
    let layer_shader = || {
        FillShader::new(
            "fn layer(input: LayerInput) -> vec4<f32> {
                return vec4(input.color.bgr * 0.8 + 0.1, input.color.a);
            }",
            None,
        )
    };
    for (index, blend) in [(0., None), (1., Some(BlendMode::Multiply))] {
        let layer = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(40., 40.),
        );
        scene.set_transform(
            layer,
            Transform {
                x: 10. + 40. * index,
                y: 50.,
            },
        );
        scene.append(
            layer,
            NodeKind::Rect(Color(220, 120, 40, 255)),
            fixed(30., 30.),
        );
        scene.append(
            layer,
            NodeKind::Rect(Color(40, 160, 220, 200)),
            fixed(18., 36.),
        );
        scene.set_layer_shader(layer, Some(layer_shader()));
        if let Some(blend) = blend {
            scene.set_blend(layer, blend);
        }
    }
    scene
}

#[cfg(windows)]
fn frame(in_process: bool) -> Vec<u8> {
    zgx_gpu::set_app_shaders_in_process(in_process);
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let report = scene.flush();
    gpu.render(&scene, &report.damage).unwrap();
    gpu.readback().unwrap()
}

#[cfg(windows)]
#[test]
fn an_apps_shaders_built_through_a_child_draw_as_wgpu_builds_them() {
    let before = zgx_gpu::app_shaders_compiled_in_child();
    let started = std::time::Instant::now();
    let wgpu = frame(true);
    let in_process = started.elapsed();
    assert_eq!(
        zgx_gpu::app_shaders_compiled_in_child(),
        before,
        "built in the process: no child"
    );
    let started = std::time::Instant::now();
    let child = frame(false);
    let through_child = started.elapsed();
    let compiled = zgx_gpu::app_shaders_compiled_in_child() - before;
    // The first draw's cost: each frame makes its device and builds the
    // three shaders (`--nocapture` shows it).
    println!(
        "first frame with 3 app shaders: {in_process:?} built in the process, \
         {through_child:?} through children ({compiled} compiled)"
    );
    // The fill, the layer shader and the blended layer shader (with an
    // empty cache; a warm one reads them back).
    if std::env::var_os("ZGX_EXPECT_COLD_CACHE").is_some() {
        assert_eq!(compiled, 3, "each shader compiled in a child");
    }
    assert!(wgpu.iter().any(|&b| b != 0), "the frame drew something");
    let differing = wgpu
        .chunks(4)
        .zip(child.chunks(4))
        .position(|(a, b)| a != b);
    assert_eq!(
        differing,
        None,
        "pixel {:?} differs: wgpu {:?}, through a child {:?}",
        differing.map(|i| (i as u32 % SIZE, i as u32 / SIZE)),
        differing.map(|i| &wgpu[i * 4..i * 4 + 4]),
        differing.map(|i| &child[i * 4..i * 4 + 4]),
    );
}
