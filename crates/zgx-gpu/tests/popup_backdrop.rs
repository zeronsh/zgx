//! A popup in a window of its own blurs what its window last drew under it
//! (`GpuRenderer::set_backdrop`): the window's pixels, from where the popup's
//! box lies over them, as glass in the window would show.
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const RED: Color = Color(255, 0, 0, 255);
const BLUE: Color = Color(0, 0, 255, 255);
const GREEN: Color = Color(0, 255, 0, 255);
const WIDTH: f32 = 400.;

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
struct Window {
    scene: Scene,
    gpu: GpuRenderer,
    popup: GpuRenderer,
    left: NodeId,
    root: NodeId,
    scale: f32,
}
impl Window {
    /// Red on its left half, blue on its right; over them, as a popup root,
    /// a clear 100-point panel that blurs what's behind it at (`x`, `y`).
    fn new(x: f32, y: f32, scale: f32) -> Self {
        Self::shaped(x, y, scale, 0., false)
    }
    /// As `new`, the panel rounded by `radius`, the popup's frames drawn into
    /// its target and its drawable at once (`single_pass`, as a window's on
    /// Windows are), where blending can't mix the blur over its target.
    fn shaped(x: f32, y: f32, scale: f32, radius: f32, single_pass: bool) -> Self {
        let pixels = (WIDTH * scale) as u32;
        let mut gpu = GpuRenderer::new(pixels, pixels).unwrap();
        gpu.set_scale_factor(scale);
        #[cfg(target_os = "macos")]
        gpu.set_direct_metal(false);
        let mut popup = GpuRenderer::new_with_context(1, 1, &gpu.context()).unwrap();
        popup.set_scale_factor(scale);
        #[cfg(target_os = "macos")]
        popup.set_direct_metal(false);
        if single_pass {
            popup.debug_enable_single_pass();
        }
        let mut scene = Scene::new(WIDTH, WIDTH);
        scene.set_pixel_scale(scale);
        let root = scene.root();
        scene.set_kind(root, NodeKind::Container(Layout::Overlay));
        let left = scene.append(root, NodeKind::Rect(RED), fixed(WIDTH / 2., WIDTH));
        let right = scene.append(root, NodeKind::Rect(BLUE), fixed(WIDTH / 2., WIDTH));
        scene.set_transform(
            right,
            Transform {
                x: WIDTH / 2.,
                y: 0.,
            },
        );
        let popup_root = scene.append(
            root,
            NodeKind::Container(Layout::Overlay),
            fixed(100., 100.),
        );
        scene.set_transform(popup_root, Transform { x, y });
        let panel = scene.append(
            popup_root,
            NodeKind::Panel {
                layout: Layout::Overlay,
                quad: QuadStyle {
                    fill: Color(0, 0, 0, 0),
                    radius,
                    ..Default::default()
                },
            },
            fixed(100., 100.),
        );
        scene.set_effects(
            panel,
            Effects {
                opacity: 1.,
                blur_radius: 12.,
                ..Default::default()
            },
        );
        scene.set_popup(popup_root, true);
        let mut window = Self {
            scene,
            gpu,
            popup,
            left,
            root: popup_root,
            scale,
        };
        window.draw();
        window
    }
    /// The window's frame, then the popup's from it.
    fn draw(&mut self) {
        let damage = self.scene.flush().damage;
        self.gpu.render(&self.scene, &damage).unwrap();
        self.gpu.present().unwrap();
        let bounds = self.scene.popup_bounds(self.root).expect("a popup");
        self.popup.set_popup(self.root, bounds);
        self.popup.set_backdrop(Some(&self.gpu));
        self.popup.repaint_all();
        self.popup.render(&self.scene, &[]).unwrap();
    }
    /// The popup's pixel `x`, `y` points into its box: red, green, blue, alpha.
    fn pixel(&self, x: f32, y: f32) -> [u8; 4] {
        let width = self.popup.plane_rect().unwrap().width * self.scale;
        let bytes = self.popup.readback().unwrap();
        let at = (y * self.scale) as usize * width as usize + (x * self.scale) as usize;
        bytes[at * 4..at * 4 + 4].try_into().unwrap()
    }
}

#[test]
fn a_popup_blurs_what_its_window_drew_under_it() {
    let _gpu_fixture = gpu_fixture();
    // Over the seam: red by its left, blue by its right, both at the seam.
    let window = Window::new(150., 50., 1.);
    let [r, _, b, a] = window.pixel(2., 50.);
    assert!(r > 200 && b < 60 && a == 255, "left {r} {b} {a}");
    let [r, _, b, a] = window.pixel(98., 50.);
    assert!(b > 200 && r < 60 && a == 255, "right {r} {b} {a}");
    let [r, _, b, a] = window.pixel(50., 50.);
    assert!(
        r > 60 && b > 60 && a == 255,
        "the seam, blurred: {r} {b} {a}"
    );

    // From where it lies: wholly over the red, only red.
    let window = Window::new(20., 50., 1.);
    let [r, _, b, a] = window.pixel(90., 50.);
    assert!(r > 240 && b < 10 && a == 255, "over red {r} {b} {a}");
}

#[test]
fn without_a_backdrop_it_blurs_nothing() {
    let _gpu_fixture = gpu_fixture();
    let mut window = Window::new(150., 50., 1.);
    window.popup.set_backdrop(None);
    window.popup.repaint_all();
    window.popup.render(&window.scene, &[]).unwrap();
    assert_eq!(window.pixel(50., 50.)[3], 0);
}

#[test]
fn it_blurs_at_the_window_s_scale() {
    let _gpu_fixture = gpu_fixture();
    let window = Window::new(150., 50., 2.);
    let [r, _, b, a] = window.pixel(2., 50.);
    assert!(r > 200 && b < 60 && a == 255, "left {r} {b} {a}");
    let [r, _, b, a] = window.pixel(98., 50.);
    assert!(b > 200 && r < 60 && a == 255, "right {r} {b} {a}");
}

#[test]
fn past_the_window_s_edge_the_glass_goes_on() {
    let _gpu_fixture = gpu_fixture();
    // Half past the right edge: all of it the window's blue, opaque, with
    // no transparency mixed in by the edge.
    let window = Window::new(350., 50., 1.);
    for x in [10., 45., 55., 95.] {
        let [r, _, b, a] = window.pixel(x, 50.);
        assert!(r < 10 && b > 240 && a == 255, "at {x}: {r} {b} {a}");
    }
    // Past the bottom, the same.
    let window = Window::new(20., 350., 1.);
    for y in [10., 45., 55., 95.] {
        let [r, _, b, a] = window.pixel(50., y);
        assert!(r > 240 && b < 10 && a == 255, "at {y}: {r} {b} {a}");
    }
    // Past a corner, and from the left and the top.
    let window = Window::new(350., 350., 2.);
    let [_, _, b, a] = window.pixel(95., 95.);
    assert!(b > 240 && a == 255, "the corner {b} {a}");
    let window = Window::new(-50., -50., 1.);
    let [r, _, _, a] = window.pixel(5., 5.);
    assert!(r > 240 && a == 255, "the far corner {r} {a}");
}

/// Past the window's edge too, where the crop carries the edge on: on both
/// ways of mixing the blur.
#[test]
fn outside_its_corners_it_shows_nothing_of_the_window() {
    let _gpu_fixture = gpu_fixture();
    for single_pass in [false, true] {
        for (x, y) in [(20., 50.), (350., 350.)] {
            let window = Window::shaped(x, y, 1., 12., single_pass);
            for (cx, cy) in [(0., 0.), (99., 0.), (0., 99.), (99., 99.)] {
                assert_eq!(
                    window.pixel(cx, cy),
                    [0, 0, 0, 0],
                    "corner {cx},{cy} at {x},{y}, single pass {single_pass}"
                );
            }
        }
    }
}

#[test]
fn wholly_past_the_window_it_takes_the_nearest_pixels() {
    let _gpu_fixture = gpu_fixture();
    let window = Window::new(500., 50., 1.);
    let [r, _, b, a] = window.pixel(50., 50.);
    assert!(r < 10 && b > 240 && a == 255, "{r} {b} {a}");
}

#[test]
fn a_new_frame_under_it_shows_when_taken_again() {
    let _gpu_fixture = gpu_fixture();
    let mut window = Window::new(20., 50., 1.);
    assert!(window.pixel(90., 50.)[0] > 240);
    window.scene.set_kind(window.left, NodeKind::Rect(GREEN));
    window.draw();
    let [r, g, _, _] = window.pixel(90., 50.);
    assert!(g > 240 && r < 10, "green {r} {g}");
}

/// Past a rounded panel's curve nothing of the window shows: the popup's own
/// target stays (clear here; its shadow, in an app), however the blur is
/// drawn back. Copied from the window's frame, each corner held a wedge of it,
/// opaque, over the popup's shadow.
#[test]
fn a_rounded_popup_keeps_its_own_pixels_past_its_curve() {
    let _gpu_fixture = gpu_fixture();
    for single_pass in [false, true] {
        for scale in [1., 2.] {
            let window = Window::shaped(50., 50., scale, 24., single_pass);
            let case = format!("single_pass {single_pass}, scale {scale}");
            for (x, y) in [(1., 1.), (98., 1.), (1., 98.), (98., 98.)] {
                assert_eq!(window.pixel(x, y)[3], 0, "{case}: corner {x},{y}");
            }
            // Inside the curve it blurs the window's red, as ever.
            let [r, _, b, a] = window.pixel(50., 50.);
            assert!(r > 200 && b < 60 && a == 255, "{case}: middle {r} {b} {a}");
            let [r, _, _, a] = window.pixel(10., 10.);
            assert!(r > 200 && a == 255, "{case}: inside the curve {r} {a}");
        }
    }
}

/// A popup whose window the system shadows draws none of its panel's own
/// (`GpuRenderer::set_unshadowed`): under a translucent fill, zgx's shadow,
/// drawn whole under the box, darkened what the system blurs behind it.
#[test]
fn an_unshadowed_panel_shows_its_fill_alone() {
    let _gpu_fixture = gpu_fixture();
    let mut scene = Scene::new(WIDTH, WIDTH);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    let popup_root = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(100., 100.),
    );
    scene.set_transform(popup_root, Transform { x: 50., y: 50. });
    let panel = scene.append(
        popup_root,
        NodeKind::Quad(QuadStyle {
            fill: Color(255, 255, 255, 64),
            radius: 8.,
            shadow: Some(BoxShadow {
                color: Color(0, 0, 0, 128),
                offset: Transform { x: 0., y: 8. },
                blur_radius: 8.,
                spread: 0.,
            }),
            ..Default::default()
        }),
        fixed(100., 100.),
    );
    scene.set_popup(popup_root, true);
    let _ = scene.flush();
    let mut popup = GpuRenderer::new(1, 1).unwrap();
    #[cfg(target_os = "macos")]
    popup.set_direct_metal(false);
    let mut alpha = |unshadowed: Option<NodeId>| {
        popup.set_popup(popup_root, scene.bounds(panel));
        popup.set_unshadowed(unshadowed);
        popup.repaint_all();
        popup.render(&scene, &[]).unwrap();
        let width = popup.plane_rect().unwrap().width as usize;
        let bytes = popup.readback().unwrap();
        bytes[(50 * width + 50) * 4 + 3]
    };
    let shadowed = alpha(None);
    assert!(shadowed > 100, "the shadow under the fill: {shadowed}");
    let alone = alpha(Some(panel));
    assert!((60..=68).contains(&alone), "the fill alone: {alone}");
    assert!(alpha(None) > 100, "shadowed again");
}
