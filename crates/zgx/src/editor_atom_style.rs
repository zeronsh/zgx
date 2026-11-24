//! How a text area's atoms show beyond a wash (`AtomStyle`): a mention's
//! colour, weight, underline and leading icon. Shown through the text area's
//! styled lines (`editor_lines`), so its atoms shape, wrap, hit and select
//! as the rest of the text does; a text area with no styled atom draws as it
//! did.
use crate::{
    editor_lines::{SpanLook, SpanStyle},
    scene::Color,
    svg::SvgData,
};
use std::{ops::Range, sync::Arc};

/// How an atom shows (`EditorHandle::set_atom_style` for a text area's,
/// `Atom::style` for one's own, each of whose fields wins): its text's
/// colour and weight, a line under it, and an icon before it that stays with
/// its first word as the atom wraps.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AtomStyle {
    /// Its text's colour.
    pub color: Option<Color>,
    /// Its text's font weight (`500`).
    pub weight: Option<u16>,
    /// A line under its text.
    pub underline: Option<AtomUnderline>,
    /// An icon before its text.
    pub icon: Option<AtomIcon>,
    /// Its text shown as a picture instead (an equation): one caret unit, its
    /// text kept for copy, search and assistive technology.
    pub svg: Option<AtomSvg>,
    /// Chip padding `[left, right, y]` in points: `left` and `right` add
    /// horizontal room inside the atom wash, and `y` hugs the wash around the
    /// text vertically (matching `ui.text` inline chips).
    pub padding: Option<[f32; 3]>,
    /// Corner radius of the atom wash, in points.
    pub radius: Option<f32>,
}

/// An atom shown as a picture: `width` and `height` in EM of the text area's
/// font size, `descent` of it below the baseline.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct AtomSvg {
    /// The picture.
    pub svg: Arc<SvgData>,
    /// Its width, in EM.
    pub width: f32,
    /// Its height, in EM.
    pub height: f32,
    /// How far it reaches below the baseline, in EM.
    pub descent: f32,
}

impl PartialEq for AtomSvg {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.svg, &other.svg)
            && self.width == other.width
            && self.height == other.height
            && self.descent == other.descent
    }
}

impl AtomSvg {
    /// Its box `[width, height, descent]` in EM, sanitized.
    pub(crate) fn extents(&self) -> [f32; 3] {
        let clean = |v: f32| if v.is_finite() { v.max(0.) } else { 0. };
        let height = clean(self.height);
        [clean(self.width), height, clean(self.descent).min(height)]
    }
}

/// An atom's underline: the font's thickness, in `color` (the text's if
/// none), `offset` points below the baseline (the font's place if none).
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AtomUnderline {
    /// Its colour.
    pub color: Option<Color>,
    /// How far below the baseline it sits, in points.
    pub offset: Option<f32>,
}

/// An atom's leading icon: `size` points square, `gap` points before its
/// text, centred on its first line, optionally seated inside a rounded well
/// (`bg`, `radius`, `pad`).
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct AtomIcon {
    /// The icon.
    pub svg: Arc<SvgData>,
    /// Its width and height, in points.
    pub size: f32,
    /// The room between it (or its well) and the text, in points.
    pub gap: f32,
    /// Optional rounded well background behind the icon.
    pub bg: Option<Color>,
    /// Well corner radius, in points.
    pub radius: f32,
    /// Padding inside the well on each side of the icon, in points.
    pub pad: f32,
}

impl PartialEq for AtomIcon {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.svg, &other.svg)
            && self.size == other.size
            && self.gap == other.gap
            && self.bg == other.bg
            && self.radius == other.radius
            && self.pad == other.pad
    }
}

impl AtomIcon {
    /// Padding around the icon when seated in a well (`0` without `bg`).
    pub(crate) fn well_pad(&self) -> f32 {
        if self.bg.is_some() {
            self.pad.max(0.)
        } else {
            0.
        }
    }
    /// The room it takes before the text: itself, its well, and its gap.
    pub(crate) fn room(&self) -> f32 {
        (self.size.max(0.) + 2. * self.well_pad() + self.gap.max(0.)).max(0.)
    }
}

impl AtomStyle {
    /// `own`'s fields where it sets them, else `self`'s.
    pub(crate) fn under(&self, own: Option<&AtomStyle>) -> AtomStyle {
        let Some(own) = own else {
            return self.clone();
        };
        AtomStyle {
            color: own.color.or(self.color),
            weight: own.weight.or(self.weight),
            underline: own.underline.or(self.underline),
            icon: own.icon.clone().or_else(|| self.icon.clone()),
            svg: own.svg.clone().or_else(|| self.svg.clone()),
            padding: own.padding.or(self.padding),
            radius: own.radius.or(self.radius),
        }
    }
    /// Whether it shows anything.
    pub(crate) fn is_empty(&self) -> bool {
        self.color.is_none()
            && self.weight.is_none()
            && self.underline.is_none()
            && self.icon.is_none()
            && self.svg.is_none()
            && self.padding.is_none()
            && self.radius.is_none()
    }
    /// The spans that show an atom over `range` of `text` so: its style over
    /// all of it, leading room before its first character, and trailing room
    /// after its last. Emitted strictly sorted and disjoint so `runs()` stays
    /// on its linear fast path.
    pub(crate) fn spans(
        &self,
        text: &str,
        range: Range<usize>,
        out: &mut Vec<(Range<usize>, SpanStyle)>,
        name: &mut dyn FnMut(SpanLook) -> std::num::NonZeroU32,
    ) {
        if range.is_empty() || range.end > text.len() {
            return;
        }
        // Its underline's colour and place and its picture, named once in
        // the editor's looks: its spans stay small.
        let look = SpanLook {
            underline_color: self.underline.and_then(|line| line.color),
            underline_offset: self.underline.and_then(|line| line.offset),
            picture: self
                .svg
                .as_ref()
                .map(|svg| (svg.svg.clone(), svg.extents())),
            ..SpanLook::default()
        };
        let style = SpanStyle {
            color: self.color,
            weight: self.weight,
            underline: self.underline.is_some(),
            look: (!look.is_empty()).then(|| name(look.clone())),
            ..SpanStyle::default()
        };
        let pad_left = self.padding.map_or(0., |p| p[0].max(0.));
        let pad_right = self.padding.map_or(0., |p| p[1].max(0.));
        let lead = pad_left + self.icon.as_ref().map_or(0., AtomIcon::room);
        let trail = pad_right;

        let slice = &text[range.start..range.end];
        let first = slice
            .chars()
            .next()
            .map_or(range.end, |c| range.start + c.len_utf8());
        let last_start = slice
            .chars()
            .next_back()
            .map_or(range.start, |c| range.end - c.len_utf8());

        if first >= range.end {
            // Single-character atom: combine lead and trail on the one span.
            if lead > 0. || trail > 0. {
                let mut edge = style;
                edge.look = Some(name(SpanLook {
                    lead: (lead > 0.).then_some(lead),
                    trail: (trail > 0.).then_some(trail),
                    ..look
                }));
                out.push((range, edge));
            } else if style != SpanStyle::default() {
                out.push((range, style));
            }
            return;
        }

        let mut mid_start = range.start;
        if lead > 0. {
            let mut lead_style = style;
            lead_style.look = Some(name(SpanLook {
                lead: Some(lead),
                ..look.clone()
            }));
            out.push((range.start..first, lead_style));
            mid_start = first;
        }
        let mid_end = if trail > 0. { last_start } else { range.end };
        if mid_start < mid_end && style != SpanStyle::default() {
            out.push((mid_start..mid_end, style));
        }
        if trail > 0. {
            let mut trail_style = style;
            trail_style.look = Some(name(SpanLook {
                trail: Some(trail),
                ..look
            }));
            out.push((last_start..range.end, trail_style));
        }
    }
}
