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
    /// a clear 100-point panel that blurs what's behind it at (`x`, 50).
    fn new(x: f32, scale: f32) -> Self {
        let pixels = (WIDTH * scale) as u32;
        let mut gpu = GpuRenderer::new(pixels, pixels).unwrap();
        gpu.set_scale_factor(scale);
        #[cfg(target_os = "macos")]
        gpu.set_direct_metal(false);
        let mut popup = GpuRenderer::new_with_context(1, 1, &gpu.context()).unwrap();
        popup.set_scale_factor(scale);
        #[cfg(target_os = "macos")]
        popup.set_direct_metal(false);
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
        scene.set_transform(popup_root, Transform { x, y: 50. });
        let panel = scene.append(
            popup_root,
            NodeKind::Panel {
                layout: Layout::Overlay,
                quad: QuadStyle {
                    fill: Color(0, 0, 0, 0),
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
    let window = Window::new(150., 1.);
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
    let window = Window::new(20., 1.);
    let [r, _, b, a] = window.pixel(90., 50.);
    assert!(r > 240 && b < 10 && a == 255, "over red {r} {b} {a}");
}

#[test]
fn without_a_backdrop_it_blurs_nothing() {
    let _gpu_fixture = gpu_fixture();
    let mut window = Window::new(150., 1.);
    window.popup.set_backdrop(None);
    window.popup.repaint_all();
    window.popup.render(&window.scene, &[]).unwrap();
    assert_eq!(window.pixel(50., 50.)[3], 0);
}

#[test]
fn it_blurs_at_the_window_s_scale() {
    let _gpu_fixture = gpu_fixture();
    let window = Window::new(150., 2.);
    let [r, _, b, a] = window.pixel(2., 50.);
    assert!(r > 200 && b < 60 && a == 255, "left {r} {b} {a}");
    let [r, _, b, a] = window.pixel(98., 50.);
    assert!(b > 200 && r < 60 && a == 255, "right {r} {b} {a}");
}

#[test]
fn past_the_window_s_edge_nothing_is_blurred() {
    let _gpu_fixture = gpu_fixture();
    // Half past the right edge (a popup's window may be).
    let window = Window::new(350., 1.);
    let [_, _, b, a] = window.pixel(10., 50.);
    assert!(b > 150 && a > 150, "inside {b} {a}");
    assert_eq!(window.pixel(95., 50.)[3], 0, "past it");
}

#[test]
fn a_new_frame_under_it_shows_when_taken_again() {
    let _gpu_fixture = gpu_fixture();
    let mut window = Window::new(20., 1.);
    assert!(window.pixel(90., 50.)[0] > 240);
    window.scene.set_kind(window.left, NodeKind::Rect(GREEN));
    window.draw();
    let [r, g, _, _] = window.pixel(90., 50.);
    assert!(g > 240 && r < 10, "green {r} {g}");
}
