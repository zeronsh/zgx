//! A streamed reply's paragraph as Zeron's transcript shows it, shared by
//! `appended_text` (drawn as drawn afresh) and `streamed_reply_flat` (its
//! cost a frame): prose with prices and math in inline code chips (padded,
//! rounded backgrounds), **bold**, a link (underlined) and more code,
//! appended 24 bytes a frame, each piece a run of its own fading in (its
//! colour's alpha eased up over `FADE` frames, a frame in each a recolour
//! alone), and pieces merging back into their style's run once faded.
#![allow(dead_code)]
use std::sync::Arc;
use zgx::{
    rich_text::{Decoration, RichText, TextRun},
    scene::{Align, Color, Layout, NodeId, NodeKind, Scene, Style as Box},
    text_layout::{FontFamily, FontStyle},
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Style {
    Plain,
    Bold,
    Code,
    Link,
}

/// The paragraph, as its runs of styled text: "The plan costs $5 and $10 a
/// seat, or $3 with the discount; the bound is `$O(n \log n)$` for part 3,
/// and **the rest** follows from it. A [link](https://example.com/3) and
/// `inline code` too." `times` over, as one paragraph.
pub fn paragraph(times: usize) -> (String, Vec<Style>) {
    let parts = [
        (
            "The plan costs $5 and $10 a seat, or $3 with the discount; the bound is ",
            Style::Plain,
        ),
        ("$O(n \\log n)$", Style::Code),
        (" for part 3, and ", Style::Plain),
        ("the rest", Style::Bold),
        (" follows from it. A ", Style::Plain),
        ("link", Style::Link),
        (" and ", Style::Plain),
        ("inline code", Style::Code),
        (" too. ", Style::Plain),
    ];
    let (mut text, mut styles) = (String::new(), Vec::new());
    for _ in 0..times {
        for (part, style) in parts {
            text.push_str(part);
            styles.extend(std::iter::repeat_n(style, part.len()));
        }
    }
    (text, styles)
}

/// Bytes appended at once.
pub const PIECE: usize = 24;
/// Frames a piece fades in over.
pub const FADE: u32 = 6;

/// A reply streaming in: what of its paragraph shows, the pieces it came
/// in (where each starts, and the frame it came), and the frame now.
pub struct Reply {
    pub text: String,
    styles: Vec<Style>,
    pub shown: usize,
    pieces: Vec<(usize, u32)>,
    frame: u32,
}
impl Reply {
    /// The paragraph `times` over: its runs as many as its length.
    pub fn new(times: usize) -> Self {
        let (text, styles) = paragraph(times);
        Self::of(text, styles)
    }
    /// The paragraph once, then prose of its style to `bytes` and more: its
    /// runs as many whatever its length (but the pieces fading).
    pub fn prose(bytes: usize) -> Self {
        let (mut text, mut styles) = paragraph(1);
        let more = "Words keep arriving as the reply streams on, a piece after another. ";
        while text.len() < bytes {
            text.push_str(more);
            styles.extend(std::iter::repeat_n(Style::Plain, more.len()));
        }
        Self::of(text, styles)
    }
    fn of(text: String, styles: Vec<Style>) -> Self {
        Self {
            text,
            styles,
            shown: 0,
            pieces: Vec::new(),
            frame: 0,
        }
    }
    /// Shown to `shown` bytes, all of it faded in long ago.
    pub fn shown_to(&mut self, shown: usize) {
        self.shown = self.boundary(shown);
        self.pieces = vec![(0, 0)];
        self.frame = 1000;
    }
    fn boundary(&self, mut at: usize) -> usize {
        at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at += 1;
        }
        at
    }
    /// Whether all of it shows.
    pub fn done(&self) -> bool {
        self.shown == self.text.len()
    }
    /// The next piece comes, a frame on.
    pub fn append(&mut self) {
        self.frame += 1;
        self.pieces.push((self.shown, self.frame));
        self.shown = self.boundary(self.shown + PIECE);
    }
    /// A frame on: its pieces fade in.
    pub fn tick(&mut self) {
        self.frame += 1;
    }
    /// What shows, as rich text: runs of a style, each piece still fading
    /// a run of its own in its colour as it is this frame.
    pub fn rich(&self) -> RichText {
        let shown = &self.text[..self.shown];
        // Each byte's piece, if it still fades.
        let mut fading = vec![None; shown.len()];
        for (i, (start, born)) in self.pieces.iter().enumerate() {
            let end = self.pieces.get(i + 1).map_or(shown.len(), |next| next.0);
            if self.frame - born < FADE {
                for at in &mut fading[*start..end] {
                    *at = Some(i);
                }
            }
        }
        let mut runs: Vec<TextRun> = Vec::new();
        let mut start = 0;
        for at in 1..=shown.len() {
            let ends = at == shown.len()
                || self.styles[at] != self.styles[start]
                || fading[at] != fading[start];
            if !ends || !shown.is_char_boundary(at) {
                continue;
            }
            let alpha = fading[start].map_or(1., |piece| {
                let t = (self.frame - self.pieces[piece].1 + 1) as f32 / FADE as f32;
                1. - (1. - t).powi(2)
            });
            runs.push(run(self.styles[start], start..at, alpha));
            start = at;
        }
        RichText::new(shown, runs).unwrap()
    }
}

fn run(style: Style, range: std::ops::Range<usize>, alpha: f32) -> TextRun {
    let color = |r, g, b| Color(r, g, b, (255. * alpha).round() as u8);
    let mut run = TextRun {
        range,
        font_size: 15.,
        color: color(225, 225, 230),
        ..Default::default()
    };
    match style {
        Style::Plain => {}
        Style::Bold => {
            run.font = FontStyle {
                weight: 700,
                ..FontStyle::default()
            }
        }
        Style::Code => {
            run.font = FontStyle {
                family: FontFamily::Monospace,
                ..FontStyle::default()
            };
            run.font_size = 14.;
            run.color = color(240, 200, 150);
            run.background = Some(Color(55, 55, 65, 255));
            run.padding = [3., 1.];
            run.radius = 4.;
        }
        Style::Link => {
            run.color = color(120, 170, 255);
            run.underline = Some(Decoration::new(1.));
        }
    }
    run
}

/// A window `width` by `height` of a heading and the paragraph, in a
/// column, measured and drawn by `renderer`.
pub fn window(
    renderer: &zgx_gpu::GpuRenderer,
    (width, height): (f32, f32),
    paragraph: RichText,
) -> (Scene, NodeId) {
    let mut scene = Scene::new(width, height);
    renderer.install_text(&mut scene);
    let column = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Column),
        Box {
            gap: 6.,
            padding: 8.,
            align: Align::Stretch,
            width: Some(width),
            ..Box::default()
        },
    );
    let heading = RichText::new(
        "A reply, streaming in",
        vec![TextRun {
            range: 0..21,
            font_size: 18.,
            ..Default::default()
        }],
    )
    .unwrap();
    for rich in [heading, paragraph] {
        scene.append(
            column,
            NodeKind::RichText {
                text: Arc::new(rich),
            },
            Box {
                text_wrap: true,
                ..Box::default()
            },
        );
    }
    let text = *scene.children(column).last().unwrap();
    (scene, text)
}
