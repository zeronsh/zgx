//! A canvas's drawing (`zgx::canvas::Canvas`) written into the frame's
//! buffer for the JS side to draw with Canvas 2D (`PAINT`): its commands as
//! words, numbers as their `f32` bits, text in the frame's strings. The
//! layout matches `paint` in `js/zgx-dom.js`.

use crate::ops::Ops;
use zgx::{
    canvas::{
        Brush, Draw, FillRule, LineCap, LineJoin, Path, Segment, Shape, TextAnchor, TextBaseline,
    },
    scene::Color,
};

const FILL: u32 = 1;
const STROKE: u32 = 2;
const CLIP: u32 = 3;
const RESTORE: u32 = 4;
const TEXT: u32 = 5;

/// `canvas`'s commands and text, after `PAINT canvas width height count`.
pub(crate) fn encode(ops: &mut Ops, canvas: &zgx::canvas::Canvas) {
    for draw in canvas.commands() {
        match draw {
            Draw::Fill { path, brush, rule } => {
                ops.words.extend_from_slice(&[FILL, rule_word(*rule)]);
                brush_words(ops, brush);
                path_words(ops, path);
            }
            Draw::Stroke { path, brush, style } => {
                ops.words.extend_from_slice(&[
                    STROKE,
                    style.width.to_bits(),
                    match style.cap {
                        LineCap::Butt => 0,
                        LineCap::Round => 1,
                        LineCap::Square => 2,
                    },
                    match style.join {
                        LineJoin::Miter => 0,
                        LineJoin::Round => 1,
                        LineJoin::Bevel => 2,
                    },
                    style.miter_limit.to_bits(),
                    style.dash.len() as u32,
                ]);
                ops.words.extend(style.dash.iter().map(|d| d.to_bits()));
                ops.words.push(style.dash_offset.to_bits());
                brush_words(ops, brush);
                path_words(ops, path);
            }
            Draw::Clip { path, rule } => {
                ops.words.extend_from_slice(&[CLIP, rule_word(*rule)]);
                path_words(ops, path);
            }
            Draw::Restore => ops.words.push(RESTORE),
        }
    }
    // Text goes over the shapes, whatever their order, unclipped.
    for text in canvas.texts() {
        ops.words.push(TEXT);
        ops.str(&text.text);
        ops.str(&zgx::compose::__dom::font_css(&text.font, text.size));
        ops.words.extend_from_slice(&[
            text.x.to_bits(),
            text.y.to_bits(),
            rgba(text.color),
            match text.anchor {
                TextAnchor::Start => 0,
                TextAnchor::Center => 1,
                TextAnchor::End => 2,
            },
            match text.baseline {
                TextBaseline::Top => 0,
                TextBaseline::Middle => 1,
                TextBaseline::Bottom => 2,
            },
        ]);
    }
}

/// How many commands `canvas` writes (`PAINT`'s count).
pub(crate) fn count(canvas: &zgx::canvas::Canvas) -> u32 {
    (canvas.commands().len() + canvas.texts().len()) as u32
}

fn rule_word(rule: FillRule) -> u32 {
    match rule {
        FillRule::Winding => 0,
        FillRule::EvenOdd => 1,
    }
}

fn rgba(Color(r, g, b, a): Color) -> u32 {
    (r as u32) << 24 | (g as u32) << 16 | (b as u32) << 8 | a as u32
}

fn brush_words(ops: &mut Ops, brush: &Brush) {
    match brush {
        Brush::Solid(color) => ops.words.extend_from_slice(&[0, rgba(*color)]),
        Brush::Linear { start, end, stops } => {
            ops.words.extend_from_slice(&[
                1,
                start.x.to_bits(),
                start.y.to_bits(),
                end.x.to_bits(),
                end.y.to_bits(),
                stops.len() as u32,
            ]);
            for stop in stops.iter() {
                ops.words
                    .extend_from_slice(&[stop.offset.to_bits(), rgba(stop.color)]);
            }
        }
        Brush::Slash {
            background,
            foreground,
            spacing,
            width,
        } => ops.words.extend_from_slice(&[
            2,
            rgba(*background),
            rgba(*foreground),
            spacing.to_bits(),
            width.to_bits(),
        ]),
    }
}

fn path_words(ops: &mut Ops, path: &Path) {
    match path.shape() {
        Some(Shape::Rect { rect, radius }) => ops.words.extend_from_slice(&[
            0,
            rect.x.to_bits(),
            rect.y.to_bits(),
            rect.width.to_bits(),
            rect.height.to_bits(),
            radius.to_bits(),
        ]),
        Some(Shape::Ellipse { rect }) => ops.words.extend_from_slice(&[
            1,
            rect.x.to_bits(),
            rect.y.to_bits(),
            rect.width.to_bits(),
            rect.height.to_bits(),
        ]),
        Some(Shape::Line { from, to }) => ops.words.extend_from_slice(&[
            2,
            from.x.to_bits(),
            from.y.to_bits(),
            to.x.to_bits(),
            to.y.to_bits(),
        ]),
        None => {
            let segments = path.segments();
            ops.words.extend_from_slice(&[3, segments.len() as u32]);
            for segment in segments {
                match segment {
                    Segment::Move(p) => {
                        ops.words
                            .extend_from_slice(&[0, p.x.to_bits(), p.y.to_bits()])
                    }
                    Segment::Line(p) => {
                        ops.words
                            .extend_from_slice(&[1, p.x.to_bits(), p.y.to_bits()])
                    }
                    Segment::Quadratic(c, p) => ops.words.extend_from_slice(&[
                        2,
                        c.x.to_bits(),
                        c.y.to_bits(),
                        p.x.to_bits(),
                        p.y.to_bits(),
                    ]),
                    Segment::Cubic(a, b, p) => ops.words.extend_from_slice(&[
                        3,
                        a.x.to_bits(),
                        a.y.to_bits(),
                        b.x.to_bits(),
                        b.y.to_bits(),
                        p.x.to_bits(),
                        p.y.to_bits(),
                    ]),
                    Segment::Close => ops.words.push(4),
                }
            }
        }
    }
}
