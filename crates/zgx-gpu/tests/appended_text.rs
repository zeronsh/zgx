//! A paragraph growing at its end (a streamed reply) is drawn from the lines
//! it keeps (`GpuStats::kept_text_lines`) and damaged from the first line
//! that may change (`Scene::set_rich_text_unchanged_above`), and every frame
//! shows exactly the pixels of the same text drawn afresh: appends of words,
//! the rest of a word, punctuation, spaces, words too long for a line, emoji,
//! CJK, Arabic and Hebrew, combining marks, line breaks, and runs of other
//! weights, sizes, colours, underlines and backgrounds, each appended alone.
use std::sync::Arc;
use zgx::{
    rich_text::{Decoration, RichText, TextRun},
    scene::{Align, Color, Layout, NodeId, NodeKind, Rect, Scene, Style},
    text_layout::FontStyle,
};
use zgx_gpu::{GpuContext, GpuRenderer};

const W: f32 = 480.;
const H: f32 = 720.;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a, T>(&mut self, of: &'a [T]) -> &'a T {
        &of[(self.next() % of.len() as u64) as usize]
    }
}

/// What may be appended at once, by kind.
const WORDS: &[&str] = &[
    " tokens", " stream", " into", " one", " para", "graph", " which", " grows", " every",
    " frame", "s", "ing", " the", " a", " of", " 3.14", " x", " ok",
];
const PUNCTUATION: &[&str] = &[
    ",",
    ".",
    "!",
    "?",
    " (",
    ")",
    " \"quoted\"",
    " —",
    "...",
    ";",
];
/// Punctuation as prose has it, which words are read from (`line_words`).
const PROSE: &[&str] = &[",", ".", "!", "?", ";", ":", " —", "..."];
const SPACES: &[&str] = &[" ", "  ", " ", "   "];
const LONG: &[&str] = &[
    " https://example.com/a/very/long/path/that/keeps/going/without/any/spaces/at/all/to/wrap",
    " Supercalifragilisticexpialidociousandthensomemorelettersthatnolinecanhold",
];
const OTHER: &[&str] = &[
    " 👩‍💻",
    "😀",
    " 🇫🇷",
    " 漢字かな交じり文",
    "中文文本没有空格",
    " مرحبا بالعالم",
    " שלום עולם",
    "e\u{301}",
    " a\u{308}\u{301}",
    " naïve café",
];

/// A run's style, as appended: plain, bold, larger, recoloured, or (not
/// kept, drawn afresh) underlined or on a background.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Plain,
    Bold,
    Large,
    Red,
    Underlined,
    Chip,
}

fn run(look: Look, range: std::ops::Range<usize>) -> TextRun {
    let mut run = TextRun {
        range,
        font_size: 14.,
        color: Color(230, 230, 230, 255),
        ..Default::default()
    };
    match look {
        Look::Plain => {}
        Look::Bold => {
            run.font = FontStyle {
                weight: 700,
                ..FontStyle::default()
            }
        }
        Look::Large => run.font_size = 19.,
        Look::Red => run.color = Color(240, 90, 80, 255),
        Look::Underlined => {
            run.color = Color(120, 170, 255, 255);
            run.underline = Some(Decoration::new(1.));
        }
        Look::Chip => {
            run.background = Some(Color(60, 60, 70, 255));
            run.padding = [3., 1.];
            run.radius = 3.;
        }
    }
    run
}

/// A paragraph streamed in, as its runs: each appended piece goes on the
/// last run or starts a run of its own.
struct Stream {
    text: String,
    runs: Vec<(Look, usize)>,
}
impl Stream {
    fn rich(&self) -> NodeKind {
        let mut start = 0;
        let runs: Vec<TextRun> = self
            .runs
            .iter()
            .map(|&(look, end)| {
                let made = run(look, start..end);
                start = end;
                made
            })
            .collect();
        NodeKind::RichText {
            text: Arc::new(RichText::new(self.text.clone(), runs).unwrap()),
        }
    }
    fn append(&mut self, piece: &str, look: Option<Look>) {
        self.text.push_str(piece);
        let end = self.text.len();
        match (look, self.runs.last_mut()) {
            (None, Some(last)) => last.1 = end,
            (look, _) => self.runs.push((look.unwrap_or(Look::Plain), end)),
        }
    }
}

/// A window of a heading and the paragraph, in a column, measured and drawn
/// by `renderer`.
fn window(renderer: &GpuRenderer, paragraph: NodeKind) -> (Scene, NodeId) {
    let mut scene = Scene::new(W, H);
    renderer.install_text(&mut scene);
    let column = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Column),
        Style {
            gap: 6.,
            padding: 8.,
            align: Align::Stretch,
            width: Some(W),
            ..Style::default()
        },
    );
    let heading = Stream {
        text: "A reply, streaming in".into(),
        runs: vec![(Look::Bold, 21)],
    };
    scene.append(
        column,
        heading.rich(),
        Style {
            text_wrap: true,
            ..Style::default()
        },
    );
    let text = scene.append(
        column,
        paragraph,
        Style {
            text_wrap: true,
            ..Style::default()
        },
    );
    (scene, text)
}

/// The paragraph as it is drawn afresh: a scene and renderer of its own,
/// its text prepared from nothing.
fn fresh(context: &GpuContext, paragraph: NodeKind) -> Vec<u8> {
    let mut renderer = GpuRenderer::new_with_context(W as u32, H as u32, context).unwrap();
    renderer.text_cache().borrow_mut().clear();
    let (mut scene, _) = window(&renderer, paragraph);
    scene.flush();
    renderer.render(&scene, &[Rect::new(0., 0., W, H)]).unwrap();
    renderer.readback().unwrap()
}

/// Streams `steps` pieces drawn from `kinds` (with a new run's look one in
/// `new_run` times, from `looks`), each frame drawn where it is damaged and
/// compared with the text drawn afresh. What it kept: lines, and frames
/// damaged below the paragraph's top.
fn stream(
    seed: u64,
    steps: usize,
    kinds: &[&[&str]],
    looks: &[Look],
    (new_run, at_spaces): (u64, bool),
) -> (usize, usize) {
    let mut rng = Rng(seed);
    let mut renderer = GpuRenderer::new(W as u32, H as u32).expect("a GPU");
    let fresh_context = GpuRenderer::new(W as u32, H as u32).unwrap().context();
    let mut text = Stream {
        text: "Tokens".into(),
        runs: vec![(Look::Plain, 6)],
    };
    let (mut scene, paragraph) = window(&renderer, text.rich());
    let report = scene.flush();
    renderer.render(&scene, &report.damage).unwrap();
    let (mut kept, mut below) = (0, 0);
    for step in 0..steps {
        let kind = kinds[(rng.next() % kinds.len() as u64) as usize];
        let piece = *rng.pick(kind);
        // (Kept, runs start at words: a word across runs, which punctuation
        // after spaces may make, is shaped whole, and so is its paragraph.)
        let word = piece.starts_with(' ') && piece[1..].starts_with(char::is_alphanumeric);
        let look =
            (rng.next().is_multiple_of(new_run) && (!at_spaces || word)).then(|| *rng.pick(looks));
        text.append(piece, look);
        let top = scene.bounds(paragraph).y;
        scene.set_leaf_kind(paragraph, text.rich());
        let report = scene.flush();
        if !report.damage.is_empty() && report.damage.iter().all(|d| d.y > top + 1.) {
            below += 1;
        }
        kept += renderer
            .render(&scene, &report.damage)
            .unwrap()
            .kept_text_lines;
        let drawn = renderer.readback().unwrap();
        let expected = fresh(&fresh_context, text.rich());
        let differing = drawn
            .chunks(4)
            .zip(expected.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            differing, 0,
            "seed {seed}: {differing} pixels differ after step {step}, {:?} appended to {:?}",
            piece, text.text
        );
    }
    (kept, below)
}

/// Words, spaces and punctuation in runs of other weights, sizes and
/// colours: drawn from the lines kept, damaged from where they change.
#[test]
fn a_paragraph_grown_from_its_kept_lines_draws_as_drawn_afresh() {
    let looks = [Look::Plain, Look::Bold, Look::Large, Look::Red];
    for seed in [1, 7, 42] {
        let (kept, below) = stream(
            seed,
            120,
            &[WORDS, WORDS, WORDS, PROSE, SPACES],
            &looks,
            (5, true),
        );
        eprintln!("seed {seed}: {kept} lines kept, {below} frames damaged below its top");
        assert!(kept > 100, "seed {seed}: {kept} lines kept");
        assert!(
            below > 20,
            "seed {seed}: {below} frames damaged below its top"
        );
    }
}

/// Everything at once: words too long for a line, emoji, CJK, right-to-left
/// text, combining marks, underlines and backgrounds, which draw some
/// frames afresh, and the rest as they were.
#[test]
fn a_paragraph_grown_by_anything_draws_as_drawn_afresh() {
    let looks = [
        Look::Plain,
        Look::Bold,
        Look::Large,
        Look::Red,
        Look::Underlined,
        Look::Chip,
    ];
    for seed in [3, 11, 99, 2024] {
        stream(
            seed,
            70,
            &[WORDS, WORDS, PUNCTUATION, SPACES, LONG, OTHER, OTHER],
            &looks,
            (6, false),
        );
    }
}

/// A line break makes it two paragraphs, which are laid out whole: still
/// as drawn afresh.
#[test]
fn a_paragraph_grown_past_a_line_break_draws_as_drawn_afresh() {
    stream(
        5,
        50,
        &[WORDS, WORDS, SPACES, &["\n", " and then\n"]],
        &[Look::Plain],
        (1000, false),
    );
}

#[path = "reply/mod.rs"]
mod reply;

/// Zeron's reply (`reply`): prose, prices, math and code in padded chips,
/// bold, a link, appended a piece at a time, each fading in as a run of its
/// own and merging back into its style's run once faded, two frames of
/// fading (recolouring alone) for each appended: each frame as drawn
/// afresh, most from the lines kept and damaged below the paragraph's top.
#[test]
fn a_streamed_reply_fading_in_draws_as_drawn_afresh() {
    let size = (W, H);
    let mut renderer = GpuRenderer::new(W as u32, H as u32).expect("a GPU");
    let fresh_context = GpuRenderer::new(W as u32, H as u32).unwrap().context();
    let mut stream = reply::Reply::new(5);
    stream.append();
    let (mut scene, paragraph) = reply::window(&renderer, size, stream.rich());
    let report = scene.flush();
    renderer.render(&scene, &report.damage).unwrap();
    let (mut kept, mut below, mut frames) = (0, 0, 0);
    let mut step = 0;
    while !stream.done() || step % 3 != 0 {
        match step % 3 {
            0 => stream.append(),
            _ => stream.tick(),
        }
        step += 1;
        let top = scene.bounds(paragraph).y;
        let rich = stream.rich();
        scene.set_leaf_kind(
            paragraph,
            NodeKind::RichText {
                text: Arc::new(rich.clone()),
            },
        );
        let report = scene.flush();
        if !report.damage.is_empty() && report.damage.iter().all(|d| d.y > top + 1.) {
            below += 1;
        }
        kept += renderer
            .render(&scene, &report.damage)
            .unwrap()
            .kept_text_lines;
        frames += 1;
        let drawn = renderer.readback().unwrap();
        let expected = {
            let mut full =
                GpuRenderer::new_with_context(W as u32, H as u32, &fresh_context).unwrap();
            full.text_cache().borrow_mut().clear();
            let (mut scene, _) = reply::window(&full, size, rich);
            scene.flush();
            full.render(&scene, &[Rect::new(0., 0., W, H)]).unwrap();
            full.readback().unwrap()
        };
        let differing = drawn
            .chunks(4)
            .zip(expected.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            differing, 0,
            "{differing} pixels differ at frame {step}, {} bytes shown",
            stream.shown
        );
    }
    eprintln!("{frames} frames: {kept} lines kept, {below} damaged below its top");
    assert!(kept > frames * 4, "{kept} lines kept in {frames} frames");
    assert!(
        below * 3 > frames * 2,
        "{below} of {frames} damaged below its top"
    );
}
