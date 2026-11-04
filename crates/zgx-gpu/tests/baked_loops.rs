//! Declared loops played by a layer (`loops.rs`): what leaves the window's
//! frames, what stays in them, and what draws a sheet again.
//!
//! Its own binary without the test harness: layer presentation runs on the
//! main thread, which the harness keeps for itself.
#[cfg(target_os = "macos")]
use std::{cell::Cell, rc::Rc};
#[cfg(target_os = "macos")]
use zgx::scene::*;
#[cfg(target_os = "macos")]
use zgx_gpu::GpuRenderer;

#[cfg(target_os = "macos")]
const RED: Color = Color(255, 0, 0, 255);
#[cfg(target_os = "macos")]
const BLUE: Color = Color(0, 0, 255, 255);

#[cfg(target_os = "macos")]
struct Window {
    scene: Scene,
    gpu: GpuRenderer,
    /// The looping node, and how many times its loop was refused.
    node: NodeId,
    refused: Rc<Cell<u32>>,
}
#[cfg(target_os = "macos")]
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
#[cfg(target_os = "macos")]
impl Window {
    /// A 200-point window over blue; `holder` makes what holds the loop's
    /// 20-point red box at (`x`, `y`) of it.
    fn new(
        x: f32,
        y: f32,
        holder: impl FnOnce(&mut Scene, NodeId) -> NodeId,
        after: impl FnOnce(&mut Scene, NodeId),
    ) -> Self {
        let host = objc2_quartz_core::CALayer::new();
        let mut gpu = GpuRenderer::debug_for_layer(host, 400, 400).unwrap();
        gpu.set_scale_factor(2.);
        let mut scene = Scene::new(200., 200.);
        gpu.install_text(&mut scene);
        let root = scene.root();
        scene.set_kind(root, NodeKind::Container(Layout::Overlay));
        scene.append(root, NodeKind::Rect(BLUE), fixed(200., 200.));
        let parent = holder(&mut scene, root);
        let node = scene.append(parent, NodeKind::Rect(RED), fixed(20., 20.));
        scene.set_transform(node, Transform { x, y });
        after(&mut scene, root);
        let mut window = Self {
            scene,
            gpu,
            node,
            refused: Rc::default(),
        };
        // The first frame makes the layer the loops go on.
        window.show();
        window.bake();
        window.show();
        window
    }
    fn plain(x: f32, y: f32) -> Self {
        Self::new(x, y, |_, root| root, |_, _| {})
    }
    fn bake(&mut self) {
        let refused = self.refused.clone();
        let frames = (0..30).map(|at| (1. - at as f32 / 60., 0.)).collect();
        let baked = BakedLoop::new(
            frames,
            30.,
            Box::new(move || refused.set(refused.get() + 1)),
        );
        self.scene.set_baked_loop(self.node, Some(Rc::new(baked)));
    }
    fn show(&mut self) {
        let damage = self.scene.flush().damage;
        self.gpu.render(&self.scene, &damage).unwrap();
        self.gpu.present().unwrap();
    }
    /// The frame's pixel at (`x`, `y`) points, red and blue.
    fn pixel(&mut self, x: usize, y: usize) -> (u8, u8) {
        let (width, _, pixels) = self.gpu.debug_bare_pixels().expect("a surface shown");
        let at = (y * 2 * width as usize + x * 2) * 4;
        (pixels[at + 2], pixels[at])
    }
    fn played(&self) -> bool {
        !self.gpu.debug_loop_sheets().is_empty()
    }
}

#[cfg(target_os = "macos")]
fn a_loop_leaves_the_frames_and_nothing_presents_while_it_plays() {
    let mut window = Window::plain(40., 40.);
    assert!(window.played(), "no sheet");
    assert_eq!(window.refused.get(), 0);
    // The frames hold what's under it, for the layer over them.
    assert_eq!(window.pixel(50, 50), (0, 255));
    assert!(window.gpu.debug_loops_tagged_as_frames());
    // A hidden or covered window does as much: nothing asks for a frame.
    let drawn = window.gpu.debug_metal_frames();
    for _ in 0..5 {
        window.show();
    }
    assert_eq!(window.gpu.debug_metal_frames(), drawn);
}

#[cfg(target_os = "macos")]
fn a_read_of_the_frame_shows_the_loop_as_the_window_does() {
    let mut window = Window::plain(40., 40.);
    // Not in the frame itself; in what reads it, red over the blue.
    assert_eq!(window.pixel(50, 50), (0, 255));
    let (width, _, pixels) = window.gpu.shown_pixels().unwrap();
    let at = (100 * width as usize + 100) * 4;
    assert!(pixels[at + 2] > 120, "red {}", pixels[at + 2]);
    // Outside its box, the frame as it was.
    let beside = (100 * width as usize + 160) * 4;
    assert_eq!((pixels[beside + 2], pixels[beside]), (0, 255));
    // A region read from its middle finds it there too.
    let frame = window.gpu.shown_frame().unwrap();
    let (_, _, part) = frame.read(100, 100, 4, 4).unwrap();
    assert!(part[2] > 120, "red {}", part[2]);

    // The caret, shown or not.
    let caret = |shown| zgx_gpu::Caret {
        rect: (10., 10., 2., 20.),
        color: [0., 1., 0., 1.],
        shown,
    };
    window.gpu.set_caret(Some(caret(true)));
    let (width, _, pixels) = window.gpu.shown_pixels().unwrap();
    assert_eq!(pixels[(30 * width as usize + 22) * 4 + 1], 255, "the caret");
    window.gpu.set_caret(Some(caret(false)));
    let (width, _, pixels) = window.gpu.shown_pixels().unwrap();
    assert_eq!(pixels[(30 * width as usize + 22) * 4 + 1], 0, "hidden");
}

#[cfg(target_os = "macos")]
fn what_a_layer_would_show_wrongly_stays_in_the_frames() {
    // A clip cutting it.
    let clipped = |scene: &mut Scene, root: NodeId| {
        let clip = scene.append(
            root,
            NodeKind::Container(Layout::Overlay),
            Style {
                clip: true,
                ..fixed(50., 200.)
            },
        );
        scene.set_transform(clip, Transform { x: 0., y: 0. });
        clip
    };
    let mut window = Window::new(40., 40., clipped, |_, _| {});
    assert_eq!(window.refused.get(), 1, "cut by a clip");
    assert_eq!(window.pixel(45, 50), (255, 0), "drawn in the frames");

    // A fade's band reaching it; its interior doesn't.
    let faded = |scene: &mut Scene, root: NodeId| {
        scene.append(
            root,
            NodeKind::Container(Layout::Overlay),
            Style {
                clip: true,
                fade_edges: [60., 0.],
                ..fixed(200., 200.)
            },
        )
    };
    let window = Window::new(40., 40., faded, |_, _| {});
    assert_eq!(window.refused.get(), 1, "in a fade's band");
    let window = Window::new(40., 100., faded, |_, _| {});
    assert!(window.played(), "inside a fade, its bands clear of it");

    // Painted over by what follows it.
    let mut window = Window::new(
        40.,
        40.,
        |_, root| root,
        |scene, root| {
            let over = scene.append(root, NodeKind::Rect(BLUE), fixed(100., 100.));
            scene.set_transform(over, Transform { x: 0., y: 0. });
        },
    );
    assert_eq!(window.refused.get(), 1, "painted over");
    assert!(!window.played());
    assert_eq!(window.pixel(50, 50), (0, 255));

    // Holding something.
    let mut window = Window::plain(40., 40.);
    let panel = NodeKind::Panel {
        layout: Layout::Overlay,
        quad: QuadStyle {
            fill: RED,
            ..Default::default()
        },
    };
    window.scene.set_kind(window.node, panel);
    window.show();
    assert!(window.played(), "a panel plays");
    window
        .scene
        .append(window.node, NodeKind::Rect(BLUE), fixed(4., 4.));
    window.show();
    assert_eq!(window.refused.get(), 1, "holds a child");
}

#[cfg(target_os = "macos")]
fn a_new_look_or_scale_draws_the_sheet_again() {
    let mut window = Window::plain(40., 40.);
    let first = window.gpu.debug_loop_sheets();
    window.show();
    assert_eq!(window.gpu.debug_loop_sheets(), first, "nothing changed");
    // Moving draws nothing again: its layer moves.
    window
        .scene
        .set_transform(window.node, Transform { x: 60., y: 40. });
    window.show();
    assert_eq!(window.gpu.debug_loop_sheets(), first, "a move");
    // Its look: a theme's colour, a hover's.
    window.scene.set_kind(window.node, NodeKind::Rect(BLUE));
    window.show();
    let second = window.gpu.debug_loop_sheets();
    assert_ne!(second, first, "a new look");
    // Its scale: a window moved to another display.
    window.gpu.set_scale_factor(1.);
    window
        .scene
        .set_transform(window.node, Transform { x: 61., y: 40. });
    window.show();
    assert_ne!(window.gpu.debug_loop_sheets(), second, "a new scale");
    assert!(window.gpu.debug_loops_tagged_as_frames());
}

#[cfg(target_os = "macos")]
fn a_sheet_goes_with_its_node_and_its_loop() {
    let mut window = Window::plain(40., 40.);
    window.scene.remove(window.node);
    window.show();
    assert!(!window.played(), "removed");

    // With the subtree holding it.
    let holder = |scene: &mut Scene, root: NodeId| {
        scene.append(
            root,
            NodeKind::Container(Layout::Overlay),
            fixed(200., 200.),
        )
    };
    let mut window = Window::new(40., 40., holder, |_, _| {});
    assert!(window.played());
    let parent = window.scene.parent(window.node).unwrap();
    window.scene.remove(parent);
    window.show();
    assert!(!window.played(), "its subtree removed");
    assert!(window.scene.baked_loops().is_empty());

    let mut window = Window::plain(40., 40.);
    window.scene.set_baked_loop(window.node, None);
    window.show();
    assert!(!window.played(), "ended");
    assert_eq!(window.pixel(50, 50), (255, 0), "back in the frames");
}

/// A loop's layer goes where layout now puts its box: a row's first text
/// arriving above it moves it down, a frame's damage reaching only what
/// changed. The frames show the row in its new place; the layer must too.
#[cfg(target_os = "macos")]
fn a_loop_moved_by_layout_above_it_moves_its_layer(drawables: bool) {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    let host = objc2_quartz_core::CALayer::new();
    host.setGeometryFlipped(true);
    host.setBounds(CGRect::new(CGPoint::new(0., 0.), CGSize::new(200., 200.)));
    let mut gpu = GpuRenderer::debug_for_layer(host.clone(), 400, 400).unwrap();
    gpu.set_scale_factor(2.);
    gpu.debug_present_drawable(drawables);
    let mut scene = Scene::new(200., 200.);
    gpu.install_text(&mut scene);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    scene.append(root, NodeKind::Rect(BLUE), fixed(200., 200.));
    // A column at (30, 20): what it has written, then a lane holding the loop.
    let column = scene.append(root, NodeKind::Container(Layout::Column), fixed(100., 150.));
    scene.set_transform(column, Transform { x: 30., y: 20. });
    let written = scene.append(column, NodeKind::Rect(BLUE), fixed(100., 0.));
    let lane = scene.append(
        column,
        NodeKind::Container(Layout::Column),
        fixed(100., 30.),
    );
    let spinner = scene.append(lane, NodeKind::Rect(RED), fixed(20., 20.));
    let show = |scene: &mut Scene, gpu: &mut GpuRenderer| {
        let damage = scene.flush().damage;
        gpu.render(scene, &damage).unwrap();
        gpu.present().unwrap();
    };
    show(&mut scene, &mut gpu);
    let frames = (0..30).map(|at| (1. - at as f32 / 60., 0.)).collect();
    let baked = BakedLoop::new(frames, 30., Box::new(|| {}));
    scene.set_baked_loop(spinner, Some(Rc::new(baked)));
    show(&mut scene, &mut gpu);
    show(&mut scene, &mut gpu);
    assert!(!gpu.debug_loop_sheets().is_empty(), "no sheet");
    let layer = || {
        // SAFETY: this thread made the layer; nothing else changes its list.
        let sublayers = unsafe { host.sublayers() }.expect("sublayers");
        (sublayers.iter())
            .find(|layer| layer.zPosition() == 0.5 && !layer.isHidden())
            .expect("the loop's layer")
            .frame()
            .origin
    };
    let at = layer();
    assert_eq!((at.x, at.y), (30., 20.), "where it starts");
    // The first text lands: 26 points written above the lane.
    scene.set_style(written, fixed(100., 26.));
    show(&mut scene, &mut gpu);
    assert_eq!(scene.bounds(spinner).y, 46., "layout moved it");
    let at = layer();
    assert_eq!((at.x, at.y), (30., 46.), "its layer with it");
    // And by a word more with nothing else changed but what's written.
    scene.set_style(written, fixed(100., 40.));
    show(&mut scene, &mut gpu);
    let at = layer();
    assert_eq!((at.x, at.y), (30., 60.), "again");
}

#[cfg(target_os = "macos")]
fn a_large_loop_stays_in_the_frames() {
    let mut window = Window::plain(40., 40.);
    window.scene.set_style(window.node, fixed(180., 150.));
    window.show();
    assert!(!window.played(), "past the sheet's limit");
    assert_eq!(window.refused.get(), 1);
}

#[cfg(target_os = "macos")]
fn loops_that_look_alike_share_a_sheet() {
    let mut window = Window::plain(40., 40.);
    let twin = window
        .scene
        .append(window.scene.root(), NodeKind::Rect(RED), fixed(20., 20.));
    window
        .scene
        .set_transform(twin, Transform { x: 100., y: 40. });
    let frames = (0..30).map(|at| (1. - at as f32 / 60., 0.)).collect();
    let baked = BakedLoop::new(frames, 30., Box::new(|| {}));
    window.scene.set_baked_loop(twin, Some(Rc::new(baked)));
    window.show();
    let sheets = window.gpu.debug_loop_sheets();
    assert_eq!(sheets.len(), 2);
    assert_eq!(sheets[0], sheets[1], "one surface");
    // One going leaves the other's.
    window.scene.remove(twin);
    window.show();
    assert_eq!(window.gpu.debug_loop_sheets(), sheets[..1]);
}

/// A frame drawn twice stays one frame to a loop: a layer whose content grows
/// past its corner draws the frame again (`render_once`), and that second
/// pass, finding the loop where the first put it, must not cost the layer the
/// move. No later frame would send it again.
#[cfg(target_os = "macos")]
fn a_frame_drawn_again_for_a_shifted_layer_still_moves_a_loop() {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    let host = objc2_quartz_core::CALayer::new();
    host.setGeometryFlipped(true);
    host.setBounds(CGRect::new(CGPoint::new(0., 0.), CGSize::new(200., 200.)));
    let mut gpu = GpuRenderer::debug_for_layer(host.clone(), 400, 400).unwrap();
    gpu.set_scale_factor(2.);
    let mut scene = Scene::new(200., 200.);
    gpu.install_text(&mut scene);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    scene.append(root, NodeKind::Rect(BLUE), fixed(200., 200.));
    // Elsewhere in the window, a layer holding a child that will pass its edge.
    let group = scene.append(root, NodeKind::Container(Layout::Overlay), fixed(40., 40.));
    scene.set_transform(group, Transform { x: 150., y: 150. });
    scene.set_isolated(group, true);
    let child = scene.append(group, NodeKind::Rect(BLUE), fixed(20., 20.));
    let column = scene.append(root, NodeKind::Container(Layout::Column), fixed(100., 150.));
    scene.set_transform(column, Transform { x: 30., y: 20. });
    let written = scene.append(column, NodeKind::Rect(BLUE), fixed(100., 0.));
    let spinner = scene.append(column, NodeKind::Rect(RED), fixed(20., 20.));
    let show = |scene: &mut Scene, gpu: &mut GpuRenderer| {
        let damage = scene.flush().damage;
        gpu.render(scene, &damage).unwrap();
        gpu.present().unwrap();
    };
    show(&mut scene, &mut gpu);
    let frames = (0..30).map(|at| (1. - at as f32 / 60., 0.)).collect();
    let baked = BakedLoop::new(frames, 30., Box::new(|| {}));
    scene.set_baked_loop(spinner, Some(Rc::new(baked)));
    show(&mut scene, &mut gpu);
    show(&mut scene, &mut gpu);
    assert!(!gpu.debug_loop_sheets().is_empty(), "no sheet");
    let layer = || {
        // SAFETY: this thread made the layer; nothing else changes its list.
        let sublayers = unsafe { host.sublayers() }.expect("sublayers");
        (sublayers.iter())
            .find(|layer| layer.zPosition() == 0.5 && !layer.isHidden())
            .expect("the loop's layer")
            .frame()
            .origin
    };
    let at = layer();
    assert_eq!((at.x, at.y), (30., 20.), "where it starts");
    // In one frame: the loop's row moves, and the layer draws from a new corner.
    scene.set_style(written, fixed(100., 26.));
    scene.set_transform(child, Transform { x: -10., y: 0. });
    show(&mut scene, &mut gpu);
    assert_eq!(scene.bounds(spinner).y, 46., "layout moved it");
    let at = layer();
    assert_eq!((at.x, at.y), (30., 46.), "its layer with it");
    // And the frame after, with nothing moving: still there.
    show(&mut scene, &mut gpu);
    let at = layer();
    assert_eq!((at.x, at.y), (30., 46.), "and stays");
}

fn main() {
    #[cfg(target_os = "macos")]
    {
        a_loop_leaves_the_frames_and_nothing_presents_while_it_plays();
        a_read_of_the_frame_shows_the_loop_as_the_window_does();
        what_a_layer_would_show_wrongly_stays_in_the_frames();
        a_new_look_or_scale_draws_the_sheet_again();
        a_sheet_goes_with_its_node_and_its_loop();
        a_large_loop_stays_in_the_frames();
        a_loop_moved_by_layout_above_it_moves_its_layer(false);
        a_loop_moved_by_layout_above_it_moves_its_layer(true);
        a_frame_drawn_again_for_a_shifted_layer_still_moves_a_loop();
        loops_that_look_alike_share_a_sheet();
    }
}
