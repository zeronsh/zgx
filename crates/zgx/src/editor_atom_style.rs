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
/// text, centred on its first line.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct AtomIcon {
    /// The icon.
    pub svg: Arc<SvgData>,
    /// Its width and height, in points.
    pub size: f32,
    /// The room between it and the text, in points.
    pub gap: f32,
}

impl PartialEq for AtomIcon {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.svg, &other.svg) && self.size == other.size && self.gap == other.gap
    }
}

impl AtomIcon {
    /// The room it takes before the text: itself and its gap.
    pub(crate) fn room(&self) -> f32 {
        (self.size.max(0.) + self.gap.max(0.)).max(0.)
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
        }
    }
    /// Whether it shows anything.
    pub(crate) fn is_empty(&self) -> bool {
        self.color.is_none()
            && self.weight.is_none()
            && self.underline.is_none()
            && self.icon.is_none()
            && self.svg.is_none()
    }
    /// The spans that show an atom over `range` of `text` so: its style over
    /// all of it, and the icon's room before its first character.
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
        if let Some(icon) = &self.icon {
            let first = text[range.start..range.end]
                .chars()
                .next()
                .map_or(range.end, |c| range.start + c.len_utf8());
            let mut lead = style;
            lead.look = Some(name(SpanLook {
                lead: Some(icon.room()),
                ..look
            }));
            out.push((range.start..first, lead));
        }
        if style != SpanStyle::default() {
            out.push((range, style));
        }
    }
}
