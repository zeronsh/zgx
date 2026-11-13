//! The perf gate's output check: each flush of a gate bench's scene as two
//! hashes, what it paints (each paint item in document order: what it is,
//! its box, clip, effects and extras) and what it damages (the flush's
//! repaint areas). The gate compares a change's against its base's, scene by
//! scene: a change that claims to keep what is drawn reads the same, one that
//! doesn't shows from which flush, and "same paint, other damage" is a
//! repaint change of its own.
//!
//! A bench calls [`frame`] after each flush. With `ZGX_FINGERPRINT` naming a
//! file, the flush's two hashes are appended to it as a line; unset (the
//! gate's counted runs), the call reads one byte and returns. Apart from zgx
//! itself, so zgx's own code is the same with or without it.
//!
//! What is drawn, not how it is stored: node ids and images' and SVGs'
//! process-wide ids are left out, so the same paint made in another order
//! reads the same. Boxes, clips and damage are rounded to 1/64 of a point,
//! so arithmetic reordered below a pixel doesn't read as a change; what a
//! style gives (colours, radii, text, sizes) is hashed as it is. Text by its
//! content and style: a change of shaping that shows moves a run's box.
use std::fmt::Write as _;
use std::hash::Hasher as _;
use std::io::Write as _;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
use zgx::scene::{FrameReport, NodeKind, PaintItem, Rect, Scene};

/// Whether flushes are fingerprinted: 0 not yet asked, 1 no, 2 yes.
static STATE: AtomicU8 = AtomicU8::new(0);

/// Whether `ZGX_FINGERPRINT` is set: a bench asks before making
/// [`frame`]'s arguments (a scene's `RefCell` borrowed), so a run without it
/// pays a byte read a flush and nothing more.
#[inline]
pub fn on() -> bool {
    match STATE.load(Relaxed) {
        1 => false,
        2 => true,
        _ => arm(),
    }
}

/// `scene` just flushed as `report` says: with `ZGX_FINGERPRINT` set, its
/// paint and damage hashes appended to that file; otherwise a byte read.
#[inline]
pub fn frame(scene: &Scene, report: &FrameReport) {
    let on = match STATE.load(Relaxed) {
        1 => false,
        2 => true,
        _ => arm(),
    };
    if on {
        note(scene, report);
    }
}
/// `frame` for a bench whose flush is out of its hands (a harness's): the
/// paint alone, its damage written as `-`, which the gate leaves out.
#[inline]
pub fn paint_only(scene: &Scene) {
    let on = match STATE.load(Relaxed) {
        1 => false,
        2 => true,
        _ => arm(),
    };
    if on {
        note_paint(scene);
    }
}
#[cold]
#[inline(never)]
fn note_paint(scene: &Scene) {
    if let Some(path) = std::env::var_os("ZGX_FINGERPRINT") {
        let line = format!("{:016x} -\n", paint(scene));
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
}
#[cold]
#[inline(never)]
fn arm() -> bool {
    let on = std::env::var_os("ZGX_FINGERPRINT").is_some();
    STATE.store(if on { 2 } else { 1 }, Relaxed);
    on
}
#[cold]
#[inline(never)]
fn note(scene: &Scene, report: &FrameReport) {
    if let Some(path) = std::env::var_os("ZGX_FINGERPRINT") {
        append(std::path::Path::new(&path), scene, report);
    }
}
/// `note`'s line, appended to `path`.
fn append(path: &std::path::Path, scene: &Scene, report: &FrameReport) {
    let line = format!("{:016x} {:016x}\n", paint(scene), damage(&report.damage));
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// FNV-1a, 64 bits: the same on every machine and run, unlike std's
/// seeded hashers.
struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}
impl std::hash::Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}
/// What a style's `Debug` writes, hashed as it is written: no string made.
impl std::fmt::Write for Fnv {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        std::hash::Hasher::write(self, text.as_bytes());
        Ok(())
    }
}

/// A coordinate to 1/64 of a point.
fn point(hash: &mut Fnv, value: f32) {
    let quantized = match value.is_finite() {
        true => (f64::from(value) * 64.).round() as i64,
        false => i64::MIN,
    };
    hash.write_i64(quantized);
}
fn rect(hash: &mut Fnv, rect: Rect) {
    for value in [rect.x, rect.y, rect.width, rect.height] {
        point(hash, value);
    }
}

fn paint(scene: &Scene) -> u64 {
    let mut hash = Fnv::new();
    for item in scene.paint_items() {
        paint_item(&mut hash, &item);
    }
    hash.finish()
}

fn paint_item(hash: &mut Fnv, item: &PaintItem<'_>) {
    hash.write_u8(u8::from(item.isolated) | u8::from(item.border_pass) << 1);
    rect(hash, item.bounds);
    match item.clip {
        Some(clip) => rect(hash, clip),
        None => hash.write_u8(0xff),
    }
    let _ = write!(hash, "{:?}{:?}{:?}", item.effects, item.extras, item.font);
    let _ = write!(hash, "{:?}", item.text_options);
    kind(hash, item.kind);
}

fn kind(hash: &mut Fnv, kind: &NodeKind) {
    match kind {
        NodeKind::Image(image) => {
            hash.write_u8(1);
            hash.write_u32(image.width());
            hash.write_u32(image.height());
            let _ = write!(hash, "{:?}{:?}", image.effect(), image.transform());
            hash.write(image.pixels());
        }
        NodeKind::Svg(svg) => {
            hash.write_u8(2);
            hash.write(svg.bytes());
            let _ = write!(hash, "{:?}{:?}", svg.tint(), svg.transform());
        }
        NodeKind::Text {
            text,
            color,
            font_size,
        } => {
            hash.write_u8(3);
            hash.write(text.as_bytes());
            let _ = write!(hash, "{color:?}");
            hash.write_u32(font_size.to_bits());
        }
        #[cfg(target_os = "macos")]
        NodeKind::NativeSurface(_) => hash.write_u8(4),
        // Containers, panels, boxes, canvases and rich text: what their
        // style holds, as written.
        other => {
            let _ = write!(hash, "{other:?}");
        }
    }
}

fn damage(rects: &[Rect]) -> u64 {
    let mut hash = Fnv::new();
    for area in rects {
        rect(&mut hash, *area);
    }
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zgx::scene::{Color, Style, Transform};

    fn scene_with(x: f32) -> (Scene, zgx::scene::NodeId) {
        let mut scene = Scene::new(200., 100.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(9, 9, 9, 255)),
            Style {
                width: Some(40.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.flush();
        scene.set_transform(node, Transform { x, y: 0. });
        (scene, node)
    }

    /// The same paint made again reads the same; a box moved a point, or
    /// a colour changed, reads differently; a move below 1/64 of a point
    /// doesn't.
    #[test]
    fn the_same_paint_reads_the_same_and_a_change_does_not() {
        let hashes = |x: f32, color: Color| {
            let (mut scene, node) = scene_with(x);
            scene.set_kind(node, NodeKind::Rect(color));
            let report = scene.flush();
            (paint(&scene), damage(&report.damage))
        };
        let grey = Color(9, 9, 9, 255);
        assert_eq!(hashes(3., grey), hashes(3., grey));
        assert_ne!(hashes(3., grey).0, hashes(4., grey).0);
        assert_ne!(hashes(3., grey).0, hashes(3., Color(9, 9, 10, 255)).0);
        assert_eq!(hashes(3., grey).0, hashes(3.001, grey).0);
    }

    /// Damage is its own hash: the same paint with other damage differs
    /// there alone.
    #[test]
    fn damage_is_hashed_apart_from_paint() {
        let (mut scene, _) = scene_with(5.);
        let report = scene.flush();
        let painted = paint(&scene);
        assert_eq!(painted, paint(&scene));
        assert_ne!(damage(&report.damage), damage(&[]));
        assert_eq!(damage(&[]), Fnv::new().finish());
    }

    /// Each flush's line holds its two hashes, and the same frames give
    /// the same lines. (Through `append`: setting `ZGX_FINGERPRINT` here
    /// would fingerprint every test flushing at the same time.)
    #[test]
    fn each_flush_appends_its_hashes() {
        let out = std::env::temp_dir().join(format!("zgx-fingerprint-{}", std::process::id()));
        let lines = || {
            let _ = std::fs::remove_file(&out);
            let (mut scene, _) = scene_with(5.);
            for _ in 0..2 {
                let report = scene.flush();
                append(&out, &scene, &report);
            }
            std::fs::read_to_string(&out).unwrap()
        };
        let first = lines();
        assert_eq!(first.lines().count(), 2, "{first}");
        assert!(first.lines().all(|line| line.split(' ').count() == 2));
        assert_eq!(first, lines());
        let _ = std::fs::remove_file(&out);
    }
}
