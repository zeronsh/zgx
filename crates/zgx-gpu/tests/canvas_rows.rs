//! A canvas's sized texts changing together (a terminal's rows: runs of
//! text, colour and weight, each as wide as its row) are damaged as one: a
//! frame drawn from its damage alone has exactly the pixels of the same
//! frame drawn whole, as rows change, take other lines and scroll with what
//! holds them, in the same frame, their text reaching past their canvases.
use zgx::{
    canvas::{CanvasText, TextAnchor, TextBaseline},
    compose::prelude::*,
    reactive::Signal,
    scene::Rect,
    widgets::Ui,
};
use zgx_gpu::GpuRenderer;

const W: u32 = 320;
const H: u32 = 200;
const ROWS: usize = 14;
const LINE: f32 = 16.;
const CELL: f32 = 8.;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A row's runs: the column each starts at, its text, colour and weight.
type Runs = Vec<(usize, String, u32, bool)>;

fn runs(rng: &mut Rng, step: u64) -> Runs {
    // From near the start to past the canvas's right edge (25 cells).
    let mut column = rng.below(18) as usize;
    let mut runs = Vec::new();
    for _ in 0..1 + rng.below(4) {
        let length = 1 + rng.below(9) as usize;
        let text: String = (0..length)
            .map(|at| (b'a' + ((step as usize + at * 7 + column) % 26) as u8) as char)
            .collect();
        let colour = [0xe0e0e0, 0xff6060, 0x60ff60, 0x7090ff][rng.below(4) as usize];
        runs.push((column, text, colour, rng.below(3) == 0));
        column += length + rng.below(3) as usize;
    }
    runs
}

/// The same rows twice over: each a canvas at its line, in content that
/// scrolls (`set_scroll_copy`) inside a clipping view.
struct Rows {
    _view: zgx::compose::ViewHandle,
    ui: Ui,
    runs: Vec<Signal<Runs>>,
    lines: Vec<Signal<usize>>,
    scroll: Signal<f32>,
}

fn rows(seed: u64) -> Rows {
    let mut ui = Ui::new(W as f32, H as f32);
    let mut rng = Rng(seed);
    let runs: Vec<_> = (0..ROWS).map(|_| ui.signal(runs(&mut rng, 0))).collect();
    let lines: Vec<_> = (0..ROWS).map(|line| ui.signal(line)).collect();
    let scroll = ui.signal(0.);
    let mut content = div()
        .id("content")
        .absolute()
        .left(0.)
        .top(0.)
        .w_full()
        .h(ROWS as f32 * LINE)
        .reactive_style({
            let scroll = scroll.clone();
            move || Styles::new().translate(0., -scroll.get())
        });
    for (shown, line) in runs.iter().zip(&lines) {
        let (shown, line) = (shown.clone(), line.clone());
        content = content.child(
            canvas(move |(width, _)| {
                let mut canvas = Canvas::new();
                for (column, text, colour, bold) in shown.get() {
                    canvas.text(CanvasText {
                        text: text.into(),
                        x: column as f32 * CELL,
                        y: 0.,
                        size: 13.,
                        color: rgb(colour),
                        font: zgx::text_layout::FontStyle {
                            weight: if bold { 700 } else { 400 },
                            ..Default::default()
                        },
                        anchor: TextAnchor::Start,
                        baseline: TextBaseline::Top,
                        // As wide as the row, as a terminal sizes its runs.
                        extent: Some((width, LINE)),
                    });
                }
                canvas
            })
            .absolute()
            .left(4.)
            .top(0.)
            // Narrower than the view: runs reach past it, painted where
            // the drawing's own damage doesn't reach.
            .w(25. * CELL)
            .h(LINE)
            .reactive_style(move || Styles::new().translate(0., line.get() as f32 * LINE)),
        );
    }
    let view = ui.mount(
        div()
            .w(W as f32)
            .h(H as f32)
            .bg(rgb(0x101418))
            .overflow_hidden()
            .child(content),
    );
    let content = view.find("content").unwrap();
    ui.scene.borrow_mut().set_scroll_copy(content, true);
    Rows {
        _view: view,
        ui,
        runs,
        lines,
        scroll,
    }
}

impl Rows {
    /// Step `step`'s changes: some rows' runs, some rows to other lines,
    /// and the content scrolled by whole lines, any of them together.
    fn change(&self, step: u64) {
        let mut rng = Rng(0x243f_6a88_85a3_08d3 ^ (step + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        for _ in 0..rng.below(5) {
            let row = rng.below(ROWS as u64) as usize;
            self.runs[row].set(runs(&mut rng, step));
        }
        if rng.below(3) == 0 {
            // Two rows trade lines, as a terminal's slots take others.
            let (a, b) = (
                rng.below(ROWS as u64) as usize,
                rng.below(ROWS as u64) as usize,
            );
            let (la, lb) = (self.lines[a].get(), self.lines[b].get());
            self.lines[a].set(lb);
            self.lines[b].set(la);
        }
        if rng.below(3) == 0 {
            let lines = rng.below(3) as f32;
            let at = (self.scroll.get() + LINE * (lines - 1.)).clamp(0., LINE * 3.);
            self.scroll.set(at);
        }
    }
}

#[test]
fn sized_rows_drawn_from_their_damage_match_rows_drawn_whole() {
    let live = rows(7);
    let whole = rows(7);
    let mut damaged = GpuRenderer::new(W, H).expect("GPU adapter");
    let context = damaged.context();
    let mut full = GpuRenderer::new_with_context(W, H, &context).unwrap();
    damaged.install_text(&mut live.ui.scene.borrow_mut());
    full.install_text(&mut whole.ui.scene.borrow_mut());
    let window = [Rect::new(0., 0., W as f32, H as f32)];
    let mut partial = 0;
    let mut scrolled = 0;
    for step in 0..80 {
        if step > 0 {
            live.change(step);
            whole.change(step);
        }
        live.ui.try_prepare_frame().unwrap();
        let report = live.ui.scene.borrow_mut().flush();
        let scene = live.ui.scene.borrow();
        if report
            .damage
            .iter()
            .all(|rect| rect.width * rect.height < (W * H) as f32 / 2.)
        {
            partial += 1;
        }
        scrolled += usize::from(!scene.scroll_moves().is_empty());
        damaged.render(&scene, &report.damage).unwrap();
        drop(scene);
        whole.ui.try_prepare_frame().unwrap();
        whole.ui.scene.borrow_mut().flush();
        full.render(&whole.ui.scene.borrow(), &window).unwrap();
        let (got, expected) = (damaged.readback().unwrap(), full.readback().unwrap());
        let differing: Vec<usize> = (0..got.len() / 4)
            .filter(|&at| got[at * 4..at * 4 + 4] != expected[at * 4..at * 4 + 4])
            .collect();
        assert!(
            differing.is_empty(),
            "step {step}: {} pixels differ, first at {:?}, damage {:?}",
            differing.len(),
            differing
                .first()
                .map(|at| (at % W as usize, at / W as usize)),
            report.damage
        );
    }
    // Drawn from damage less than the window, and scrolled by copies.
    assert!(partial > 40, "{partial} partial frames");
    assert!(scrolled > 5, "{scrolled} scrolled frames");
}
