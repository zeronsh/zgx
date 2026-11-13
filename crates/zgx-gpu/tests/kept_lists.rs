//! A kept frame of a long scrolled list records only what can show (its
//! rows in view, and one out of view whose shadow reaches in), so keeping
//! it costs the same at 1,000 rows as at 7,000; what scrolling or a resize
//! brings into view is spliced or drawn whole, each frame as drawn afresh.
use zgx::{
    compose::prelude::*,
    reactive::Signal,
    scene::{BoxShadow, Color, Rect, Transform},
    widgets::Ui,
};
use zgx_gpu::{GpuRenderer, text::FontData};

const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");
const SIZE: (f32, f32) = (320., 240.);
const ROW: f32 = 24.;

/// What a frame shows: the list's offset, a value in row 3 (in view at the
/// top) and in row `far` (out of view), the inner list's offset in row 2,
/// and a blinking box beside the list (a caret: a leaf repainting alone).
#[derive(Clone)]
struct State {
    offset: Signal<f32>,
    near: Signal<u32>,
    far_value: Signal<u32>,
    inner: Signal<f32>,
    blink: Signal<u32>,
}

fn view(rows: usize, far: usize, state: State) -> View {
    let State {
        offset,
        near,
        far_value,
        inner,
        blink,
    } = state;
    let list = column().w_full().children((0..rows).map(move |i| {
        let mut row = row()
            .w_full()
            .h(ROW)
            .px(6.)
            .items_center()
            .bg(if i % 2 == 0 {
                Color(25, 34, 48, 255)
            } else {
                Color(33, 44, 60, 255)
            });
        // Out of view below, its shadow reaching up into it.
        if i == 12 {
            row = row.shadow(BoxShadow {
                color: Color(200, 80, 40, 255),
                offset: Transform { x: 0., y: -60. },
                blur_radius: 0.,
                spread: 0.,
            });
        }
        match i {
            2 => row.child(
                scroll(inner.clone())
                    .w(120.)
                    .h(ROW)
                    .child(column().children((0..6).map(|j| {
                        div()
                            .w(120.)
                            .h(ROW)
                            .bg(Color(60 + 30 * j as u8, 90, 140, 255))
                    }))),
            ),
            3 => {
                let near = near.clone();
                row.child(text_signal(move || format!("near {}", near.get())).text_size(13.))
            }
            12 => {
                let far_value = far_value.clone();
                row.child(div().w(30.).h(10.).reactive_style(move || {
                    Styles::new().bg(Color(90 + (far_value.get() % 7) as u8 * 20, 200, 90, 255))
                }))
            }
            i if i == far => {
                let far_value = far_value.clone();
                row.child(text_signal(move || format!("far {}", far_value.get())).text_size(13.))
            }
            i => row.child(text(format!("row {i}")).text_size(13.)),
        }
    }));
    // Past a tenth of the window (`KEPT_SHARE`): its frames draw from the
    // kept frame, patched.
    let blink_box = div()
        .absolute()
        .ml(264.)
        .mt(8.)
        .size(52., 200.)
        .reactive_style(move || {
            Styles::new().bg(match blink.get() % 2 {
                0 => Color(240, 240, 240, 255),
                _ => Color(20, 20, 20, 255),
            })
        });
    div()
        .w_full()
        .h_full()
        .bg(Color(12, 14, 20, 255))
        .child(scroll(offset).w(260.).h(200.).child(list))
        .child(blink_box)
}

struct Window {
    ui: Ui,
    gpu: GpuRenderer,
    /// The reference: the same scene drawn whole each frame, keeping none.
    whole: GpuRenderer,
    state: State,
    size: (f32, f32),
}
impl Window {
    fn new(rows: usize, far: usize, size: (f32, f32), at: (f32, u32, u32, f32, u32)) -> Self {
        let renderer = || {
            let mut gpu = GpuRenderer::new((size.0 * 2.) as u32, (size.1 * 2.) as u32).unwrap();
            gpu.set_scale_factor(2.);
            FontData::new(GEIST.to_vec())
                .unwrap()
                .install(&mut gpu.text_system().borrow_mut());
            gpu
        };
        let gpu = renderer();
        let mut whole = renderer();
        whole.debug_never_keep();
        let mut ui = Ui::new(size.0, size.1);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let state = State {
            offset: ui.signal(at.0),
            near: ui.signal(at.1),
            far_value: ui.signal(at.2),
            inner: ui.signal(at.3),
            blink: ui.signal(at.4),
        };
        ui.mount(view(rows, far, state.clone()));
        Window {
            ui,
            gpu,
            whole,
            state,
            size,
        }
    }
    fn draw(&mut self) -> zgx_gpu::GpuStats {
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap()
    }
    /// Drawn, and the same as the scene drawn whole afresh: whether it
    /// patched.
    fn frame(&mut self, _rows: usize, _far: usize, what: &str) -> bool {
        let stats = self.draw();
        let full = Rect::new(0., 0., self.size.0, self.size.1);
        self.whole.render(&self.ui.scene.borrow(), &[full]).unwrap();
        let (got, want) = (self.gpu.readback().unwrap(), self.whole.readback().unwrap());
        if got != want {
            let w = (self.size.0 * 2.) as usize;
            let (mut x0, mut y0, mut x1, mut y1, mut n) = (usize::MAX, usize::MAX, 0, 0, 0);
            let mut first = None;
            for (i, (a, b)) in got.chunks(4).zip(want.chunks(4)).enumerate() {
                if a != b {
                    let (x, y) = (i % w, i / w);
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                    n += 1;
                    first.get_or_insert((x, y, a.to_vec(), b.to_vec()));
                }
            }
            println!(
                "PROBE {what}: {n} px differ in x {x0}..={x1} y {y0}..={y1}; first {first:?} (got, want); stats {stats:?}"
            );
        }
        assert!(
            got == want,
            "{what}: differs from the scene drawn afresh (patched {})",
            stats.patched_items
        );
        stats.patched_items > 0
    }
    /// Caret blinks: leaves alone repainting, which a kept frame patches.
    fn blinks(&mut self, rows: usize, far: usize, n: usize, what: &str) -> usize {
        (0..n)
            .filter(|_| {
                self.state.blink.update(|b| *b += 1);
                self.frame(rows, far, what)
            })
            .count()
    }
}

/// Kept at the top, the frame holds the rows in view (and the one whose
/// shadow reaches in), not the list: as many items at 7,000 rows as at
/// 1,000.
#[test]
fn a_long_lists_kept_frame_holds_what_shows_alone() {
    let kept = |rows: usize| {
        let mut window = Window::new(rows, rows - 1, SIZE, (0., 0, 0, 0., 0));
        window.draw();
        window.blinks(rows, rows - 1, 3, "blinking");
        window.gpu.debug_kept_items().expect("a frame kept")
    };
    let (small, large) = (kept(1000), kept(7000));
    assert_eq!(
        small, large,
        "the same items kept at 1,000 rows as at 7,000"
    );
    assert!(small < 120, "{small} items kept: what shows, not the list");
}

/// Kept at one offset, scrolled to another (drawn whole, the kept frame let
/// go), then paused: blinking there keeps one again and patches, and a
/// value in a row out of view changing keeps it patching (nothing of it
/// shows), each frame as drawn afresh.
#[test]
fn a_frame_kept_before_a_scroll_patches_after_it_as_drawn_afresh() {
    let (rows, far) = (300, 250);
    let mut window = Window::new(rows, far, SIZE, (0., 0, 0, 0., 0));
    window.draw();
    assert!(window.blinks(rows, far, 3, "blinking at the top") >= 2);
    // Scrolled: the first frame's splice of the list (what moved) is too big
    // to pay, so `render_kept` refuses it and lets the frame kept at the top
    // go (`recycle_kept`); drawn whole after frames that patched, it keeps
    // one of its own, at its offset, which the next scrolled frame refuses
    // and lets go the same way. Nothing kept at the top survives the scroll,
    // and none is kept while it scrolls: `would_patch` keeps one again once
    // frames after the pause would patch.
    for step in 1..=6 {
        window.state.offset.set(step as f32 * 37.5);
        assert!(
            !window.frame(rows, far, "scrolling"),
            "a scrolled frame patches nothing"
        );
        if step >= 2 {
            assert_eq!(
                window.gpu.debug_kept_items(),
                None,
                "none kept while scrolling"
            );
        }
    }
    // (The scroll refused the kept frame, splicing the whole list: keeping
    // again backs off first, `would_patch`.)
    let patched = window.blinks(rows, far, 24, "blinking after the scroll");
    assert!(
        patched >= 8,
        "{patched} of 24 blinks patched after the scroll"
    );
    let mut patched = 0;
    for value in 1..6 {
        window.state.far_value.set(value);
        window.state.blink.update(|b| *b += 1);
        patched += usize::from(window.frame(rows, far, "a row out of view changing"));
    }
    assert!(
        patched >= 3,
        "{patched} of 5 frames patched with a row out of view changing"
    );
    // In view: drawn right, patched or not.
    window.state.near.set(9);
    window.frame(rows, far, "a row in view changing");
}

/// A row out of view whose shadow reaches into it, changing: drawn right.
#[test]
fn a_row_out_of_view_whose_shadow_shows_is_drawn_right() {
    let (rows, far) = (300, 250);
    let mut window = Window::new(rows, far, SIZE, (0., 0, 0, 0., 0));
    window.draw();
    window.blinks(rows, far, 3, "blinking");
    // Row 12 (out of view, its shadow 60 points up into it) holds the box
    // whose colour follows `far_value`.
    for value in 1..5 {
        window.state.far_value.set(value);
        window.frame(rows, far, "the shadowed row changing");
    }
}

/// A window resized after a frame was kept: drawn as afresh.
#[test]
fn a_window_resized_after_a_frame_is_kept_is_drawn_as_afresh() {
    let (rows, far) = (300, 250);
    let mut window = Window::new(rows, far, SIZE, (0., 0, 0, 0., 0));
    window.draw();
    window.blinks(rows, far, 3, "blinking");
    let size = (SIZE.0, SIZE.1 + 120.);
    window
        .gpu
        .resize((size.0 * 2.) as u32, (size.1 * 2.) as u32);
    window
        .whole
        .resize((size.0 * 2.) as u32, (size.1 * 2.) as u32);
    window.ui.scene.borrow_mut().resize(size.0, size.1);
    window.size = size;
    window.frame(rows, far, "resized");
    window.blinks(rows, far, 4, "blinking resized");
}

/// A scroll view inside a row of the list, scrolled while the list is
/// still: drawn as afresh, and blinking after it patches.
#[test]
fn a_scroll_view_in_a_row_scrolled_is_drawn_as_afresh() {
    let (rows, far) = (300, 250);
    let mut window = Window::new(rows, far, SIZE, (0., 0, 0, 0., 0));
    window.draw();
    window.blinks(rows, far, 3, "blinking");
    for step in 1..=4 {
        window.state.inner.set(step as f32 * 17.25);
        window.frame(rows, far, "the inner list scrolling");
    }
    let patched = window.blinks(rows, far, 5, "blinking after the inner scroll");
    assert!(patched >= 2, "{patched} of 5 blinks patched");
}
