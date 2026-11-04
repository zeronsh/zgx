//! A window's zoom (`Ui::zoom`) is in its renderer's scale and not in its
//! host layer's points: what Core Animation places over the frames (a baked
//! loop's layer, a GPU view's) goes where the frames show the same box. What
//! the frames draw themselves goes by the scale, under partial repaints too.
//!
//! Its own binary without the test harness: layer presentation runs on the
//! main thread, which the harness keeps for itself.
use std::sync::Arc;
use zgx::decoration::{Background, Decoration, FillShader, ShaderUniforms};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

/// Each with the window the same 200 by 200 points of a 2x display.
const ZOOMS: [f64; 3] = [1., 1.1, 0.9];
const WINDOW: f32 = 200.;
const DISPLAY: f32 = 2.;
const PIXELS: u32 = 400;
const RED: Color = Color(255, 0, 0, 255);
const BLUE: Color = Color(0, 0, 255, 255);

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
fn scale(zoom: f64) -> f32 {
    DISPLAY * zoom as f32
}
/// The window's scene at `zoom`, over blue.
fn scene(zoom: f64) -> Scene {
    let viewport = WINDOW / zoom as f32;
    let mut scene = Scene::new(viewport, viewport);
    scene.set_pixel_scale(scale(zoom));
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    scene.append(root, NodeKind::Rect(BLUE), fixed(viewport, viewport));
    scene
}

/// The box of what isn't blue, in pixels: left, top, right, bottom.
fn painted(pixels: &[u8]) -> Option<(u32, u32, u32, u32)> {
    let mut found = None;
    for (at, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
        if pixel[0] > 20 {
            let (x, y) = (at as u32 % PIXELS, at as u32 / PIXELS);
            let (left, top, right, bottom) = found.unwrap_or((x, y, x, y));
            found = Some((left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1)));
        }
    }
    found
}
/// `painted` is `expected` points of the zoomed scene, within a pixel.
#[track_caller]
fn paints_at(what: &str, zoom: f64, pixels: &[u8], expected: (f32, f32, f32, f32)) {
    let scale = scale(zoom);
    let (left, top, right, bottom) = painted(pixels).unwrap_or_else(|| panic!("{what}: nothing"));
    let wanted = [expected.0, expected.1, expected.2, expected.3].map(|at| at * scale);
    for (got, wanted) in [left, top, right, bottom].into_iter().zip(wanted) {
        assert!(
            (got as f32 - wanted).abs() <= 1.5,
            "{what} at zoom {zoom}: painted {:?}, wanted {wanted:?} of {expected:?}",
            (left, top, right, bottom),
        );
    }
}

/// A box the frames draw themselves, partly repainted as it changes.
struct Frames {
    scene: Scene,
    gpu: GpuRenderer,
    zoom: f64,
}
impl Frames {
    fn new(zoom: f64) -> Option<Self> {
        let mut gpu = GpuRenderer::new(PIXELS, PIXELS).ok()?;
        gpu.set_scale_factor(scale(zoom));
        Some(Self {
            scene: scene(zoom),
            gpu,
            zoom,
        })
    }
    fn draw(&mut self) -> Vec<u8> {
        let damage = self.scene.flush().damage;
        self.gpu.render(&self.scene, &damage).unwrap();
        self.gpu.readback().unwrap()
    }
    #[track_caller]
    fn paints_at(&mut self, what: &str, expected: (f32, f32, f32, f32)) {
        let pixels = self.draw();
        // (The readback is BGRA or RGBA by backend: red and blue told apart
        // by which the background isn't.)
        let channel = if pixels[2] > pixels[0] { 0 } else { 2 };
        let red: Vec<u8> = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[channel], 0, 0, 255])
            .collect();
        paints_at(what, self.zoom, &red, expected);
    }
    /// A 20-point box at (40, 40), the frame with it drawn whole.
    fn boxed(&mut self, kind: NodeKind) -> NodeId {
        let root = self.scene.root();
        let id = self.scene.append(root, kind, fixed(20., 20.));
        self.scene.set_transform(id, Transform { x: 40., y: 40. });
        self.paints_at("drawn", (40., 40., 60., 60.));
        id
    }
}

fn what_the_frames_draw_goes_by_the_scale(zoom: f64) -> Option<()> {
    // Moved, animated transforms' way: its old place repainted, partly.
    let mut frames = Frames::new(zoom)?;
    let id = frames.boxed(NodeKind::Rect(RED));
    frames.scene.set_transform(id, Transform { x: 70., y: 50. });
    frames.paints_at("moved", (70., 50., 90., 70.));

    // Faded, as an animated opacity: a repaint of its box alone.
    frames.scene.set_effects(
        id,
        Effects {
            opacity: 0.5,
            ..Default::default()
        },
    );
    frames.paints_at("faded", (70., 50., 90., 70.));

    // Turned a quarter about its middle: a tall box lies flat.
    let mut frames = Frames::new(zoom)?;
    let id = frames.boxed(NodeKind::Rect(RED));
    frames.scene.set_style(id, fixed(20., 60.));
    frames.paints_at("tall", (40., 40., 60., 100.));
    frames.scene.set_local_transform(
        id,
        LocalTransform {
            rotate: std::f32::consts::FRAC_PI_2,
            origin: (0.5, 0.5),
            ..Default::default()
        },
    );
    frames.paints_at("turned", (20., 60., 80., 80.));

    // A shader's fill, its time stepping: the ring's own path, unbaked.
    let mut frames = Frames::new(zoom)?;
    let shader = FillShader::new(
        "fn fill(input: FillInput) -> vec4<f32> { return vec4(1.0, 0.0, 0.0, 1.0); }",
        None,
    );
    let id = frames.boxed(NodeKind::Panel {
        layout: Layout::Overlay,
        quad: QuadStyle {
            decoration: Some(Arc::new(Decoration {
                background: Some(Background::Shader(shader)),
                ..Default::default()
            })),
            ..Default::default()
        },
    });
    frames.scene.set_shader_uniforms(
        id,
        ShaderUniforms {
            time: 1.,
            ..Default::default()
        },
    );
    frames.paints_at("shader's next step", (40., 40., 60., 60.));
    frames.scene.set_transform(id, Transform { x: 70., y: 50. });
    frames.paints_at("shader moved", (70., 50., 90., 70.));

    // A layer's shader, recomposited as its uniforms step.
    let mut frames = Frames::new(zoom)?;
    let id = frames.boxed(NodeKind::Rect(RED));
    let shader = FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> { return input.color; }",
        None,
    );
    frames.scene.set_layer_shader(id, Some(shader));
    frames.paints_at("layer shader", (40., 40., 60., 60.));
    frames.scene.set_transform(id, Transform { x: 70., y: 50. });
    frames.paints_at("layer shader moved", (70., 50., 90., 70.));
    Some(())
}

#[cfg(target_os = "macos")]
mod layers {
    use super::*;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_quartz_core::CALayer;
    use std::rc::Rc;
    use zgx::{compose::prelude::*, widgets::Ui};
    use zgx_gpu::{
        viewport::{Viewport, ViewportFrame},
        wgpu,
    };

    /// A host as a window's layer is: its points the display's.
    fn host() -> objc2::rc::Retained<CALayer> {
        let host = CALayer::new();
        host.setGeometryFlipped(true);
        host.setBounds(CGRect::new(
            CGPoint::new(0., 0.),
            CGSize::new(f64::from(WINDOW), f64::from(WINDOW)),
        ));
        host
    }
    /// `frame` is at `at`, pixels of the window, in the host's points, and
    /// `size` pixels big: a sheet's cell has a pixel to spare (`bake_loop`),
    /// so up to `spare` more, whole.
    #[track_caller]
    fn placed(what: &str, zoom: f64, frame: CGRect, at: (f64, f64), size: (f64, f64), spare: f64) {
        let point = f64::from(DISPLAY);
        let got = (frame.origin.x * point, frame.origin.y * point);
        let big = (frame.size.width * point, frame.size.height * point);
        let fits = |big: f64, size: f64| {
            (big - big.round()).abs() < 1e-3 && big > size - 1e-3 && big < size + spare + 1e-3
        };
        assert!(
            (got.0 - at.0).abs() < 1e-3
                && (got.1 - at.1).abs() < 1e-3
                && fits(big.0, size.0)
                && fits(big.1, size.1),
            "{what} at zoom {zoom}: at pixels {got:?} size {big:?}, wanted {at:?} size {size:?}",
        );
    }
    fn sublayer_at(host: &CALayer, z: f64) -> Option<CGRect> {
        // SAFETY: this thread made the layer; nothing else changes its list.
        let sublayers = unsafe { host.sublayers() }?;
        sublayers
            .iter()
            .find(|layer| layer.zPosition() == z && !layer.isHidden())
            .map(|layer| layer.frame())
    }

    struct Loop {
        host: objc2::rc::Retained<CALayer>,
        gpu: GpuRenderer,
        scene: Scene,
        node: NodeId,
    }
    impl Loop {
        /// A loop's 20-point box at (`x`, `y`) of the scene at `zoom`, playing.
        fn new(zoom: f64, x: f32, y: f32) -> Self {
            let host = host();
            let mut gpu = GpuRenderer::debug_for_layer(host.clone(), PIXELS, PIXELS).unwrap();
            gpu.set_scale_factor(scale(zoom));
            gpu.set_zoom(zoom);
            let mut scene = scene(zoom);
            gpu.install_text(&mut scene);
            let root = scene.root();
            let node = scene.append(root, NodeKind::Rect(RED), fixed(20., 20.));
            scene.set_transform(node, Transform { x, y });
            let mut window = Self {
                host,
                gpu,
                scene,
                node,
            };
            // The first frame makes the layer the loops go on.
            window.show();
            let frames = (0..30).map(|at| (1. - at as f32 / 60., 0.)).collect();
            let baked = BakedLoop::new(frames, 30., Box::new(|| {}));
            window.scene.set_baked_loop(node, Some(Rc::new(baked)));
            window.show();
            assert!(!window.gpu.debug_loop_sheets().is_empty(), "no sheet");
            window
        }
        fn show(&mut self) {
            let damage = self.scene.flush().damage;
            self.gpu.render(&self.scene, &damage).unwrap();
            self.gpu.present().unwrap();
        }
        fn frame(&self) -> CGRect {
            sublayer_at(&self.host, 0.5).expect("the loop's layer")
        }
    }

    pub fn a_loop_s_layer_lies_where_the_frames_would_draw_it(zoom: f64) {
        let pixels = |at: f32| f64::from((at * scale(zoom)).floor());
        let size = f64::from(20. * scale(zoom));
        // Far from the window's corner, as the offset grows with the distance.
        let mut window = Loop::new(zoom, 40., 120.);
        placed(
            "a loop",
            zoom,
            window.frame(),
            (pixels(40.), pixels(120.)),
            (size, size),
            3.,
        );
        // Its layer moves, nothing drawn again.
        let sheets = window.gpu.debug_loop_sheets();
        window
            .scene
            .set_transform(window.node, Transform { x: 60., y: 130. });
        window.show();
        assert_eq!(window.gpu.debug_loop_sheets(), sheets);
        placed(
            "a moved loop",
            zoom,
            window.frame(),
            (pixels(60.), pixels(130.)),
            (size, size),
            3.,
        );
        let scale = f64::from(scale(zoom));
        // What reads the window finds it there too.
        let (width, _, read) = window.gpu.shown_pixels().unwrap();
        let (x, y) = ((70. * scale) as usize, (140. * scale) as usize);
        let at = (y * width as usize + x) * 4;
        assert!(
            read[at + 2] > 120,
            "read at zoom {zoom}: red {}",
            read[at + 2]
        );
    }

    /// The caret goes in the host's points already (`apply_caret`): what
    /// reads the window finds it where its layer shows.
    pub fn a_read_finds_the_caret_where_its_layer_shows(zoom: f64) {
        let mut window = Loop::new(zoom, 40., 120.);
        window.gpu.set_caret(Some(zgx_gpu::Caret {
            rect: (10., 10., 2., 20.),
            color: [0., 1., 0., 1.],
            shown: true,
        }));
        let (width, _, read) = window.gpu.shown_pixels().unwrap();
        // Its first and last columns: 20 and 23, at 2 pixels a host point.
        let green = |x: usize| read[(30 * width as usize + x) * 4 + 1] == 255;
        assert!(
            !green(19) && green(20) && green(23) && !green(24),
            "the caret at zoom {zoom}",
        );
    }

    /// A window's zoom changing moves the layer with its scale.
    pub fn a_new_zoom_moves_the_layer() {
        let mut window = Loop::new(1., 40., 120.);
        placed("before", 1., window.frame(), (80., 240.), (40., 40.), 3.);
        window.gpu.set_scale_factor(scale(1.25));
        window.gpu.set_zoom(1.25);
        window.scene.resize(WINDOW / 1.25, WINDOW / 1.25);
        window.scene.set_pixel_scale(scale(1.25));
        window.show();
        placed("after", 1.25, window.frame(), (100., 300.), (50., 50.), 3.);
    }

    struct Fill;
    impl Viewport for Fill {
        fn render(&mut self, frame: &mut ViewportFrame<'_>) {
            frame
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("fill"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: frame.target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
        }
    }

    pub fn a_gpu_view_s_layer_lies_where_the_frames_would_draw_it(zoom: f64) {
        let host = host();
        let mut gpu = GpuRenderer::debug_for_layer(host.clone(), PIXELS, PIXELS).unwrap();
        gpu.set_scale_factor(scale(zoom));
        gpu.set_zoom(zoom);
        let view = zgx_gpu::viewport::view(Fill);
        view.set_opaque(true);
        let viewport = WINDOW / zoom as f32;
        let mut ui = Ui::new(viewport, viewport);
        ui.scene.borrow_mut().set_pixel_scale(scale(zoom));
        ui.mount(
            div().size(viewport, viewport).child(
                gpu_view("Model", &view)
                    .absolute()
                    .ml(40.)
                    .mt(120.)
                    .size(30., 20.),
            ),
        );
        // A frame to take its layer, the view's next to show it.
        for _ in 0..3 {
            ui.prepare_frame();
            let damage = ui.scene.borrow_mut().flush().damage;
            gpu.render(&ui.scene.borrow(), &damage).unwrap();
            gpu.present().unwrap();
            view.invalidate();
            ui.prepare_frame();
            gpu.render_planes(&ui.scene.borrow()).unwrap();
        }
        let scale = f64::from(scale(zoom));
        // SAFETY: this thread made the layer; nothing else changes its list.
        let sublayers = unsafe { host.sublayers() }.expect("sublayers");
        let frame = sublayers
            .iter()
            .filter(|layer| !layer.isHidden())
            .map(|layer| layer.frame())
            .find(|frame| frame.size.height != f64::from(WINDOW) && frame.size.height > 1.)
            .expect("the view's layer");
        let at = (40. * scale, 120. * scale);
        placed(
            "a GPU view",
            zoom,
            frame,
            at,
            (30. * scale, 20. * scale),
            0.,
        );
    }
}

fn main() {
    for zoom in ZOOMS {
        if what_the_frames_draw_goes_by_the_scale(zoom).is_none() {
            eprintln!("zoomed_layers: no adapter; frames not checked");
        }
    }
    #[cfg(target_os = "macos")]
    {
        for zoom in ZOOMS {
            layers::a_loop_s_layer_lies_where_the_frames_would_draw_it(zoom);
            layers::a_read_finds_the_caret_where_its_layer_shows(zoom);
            layers::a_gpu_view_s_layer_lies_where_the_frames_would_draw_it(zoom);
        }
        layers::a_new_zoom_moves_the_layer();
    }
}
