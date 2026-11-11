//! What every Rust arm reports, the same way: per-frame timings reduced to
//! a median and a 95th percentile, the proof that it drew, and one JSON line
//! on stdout for `run.py`. Included by `#[path]`.
// Each binary including this uses part of it.
#![allow(dead_code)]
use std::fmt::Write as _;

/// The scene everyone draws, read from `run.py`'s files.
pub struct Setup {
    pub model: String,
    pub environment: String,
    /// One camera a line: position, then target.
    pub path: Vec<([f32; 3], [f32; 3])>,
    pub width: u32,
    pub height: u32,
    /// Frames drawn at the path's first pose before counting starts.
    pub warmup: usize,
    /// Where the sun's light travels.
    pub sun: [f32; 3],
    /// The last frame, as a PNG.
    pub out: String,
    /// Vertical field of view, degrees.
    pub fov: f32,
    /// Multisampling: 4 (every arm's default) or 1.
    pub msaa: u32,
}

pub fn setup() -> Setup {
    let mut args = std::collections::HashMap::new();
    for arg in std::env::args().skip(1) {
        if let Some((key, value)) = arg.trim_start_matches("--").split_once('=') {
            args.insert(key.to_owned(), value.to_owned());
        }
    }
    let get = |key: &str| {
        args.get(key)
            .cloned()
            .unwrap_or_else(|| panic!("--{key}= is needed"))
    };
    let floats = |text: &str| -> Vec<f32> {
        text.split([',', ' '])
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().expect("a number"))
            .collect()
    };
    let path = std::fs::read_to_string(get("path"))
        .expect("the camera path")
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|line| {
            let v = floats(line);
            ([v[0], v[1], v[2]], [v[3], v[4], v[5]])
        })
        .collect();
    let sun = floats(&get("sun"));
    Setup {
        model: get("model"),
        environment: get("environment"),
        path,
        width: get("width").parse().unwrap(),
        height: get("height").parse().unwrap(),
        warmup: get("warmup").parse().unwrap(),
        sun: [sun[0], sun[1], sun[2]],
        out: get("out"),
        fov: args.get("fov").map_or(45., |f| f.parse().unwrap()),
        msaa: args.get("msaa").map_or(4, |m| m.parse().unwrap()),
    }
}

/// Median and 95th percentile of `values`.
pub fn spread(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    (at(0.5), at(0.95))
}

/// That a frame holds a picture: a hash of its pixels, and the fraction of
/// them that differ from the colour it was cleared to.
pub fn proof(rgba: &[u8], clear: [u8; 3]) -> (u64, f64) {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut differ = 0usize;
    for px in rgba.chunks_exact(4) {
        for &b in px {
            hash = (hash ^ b as u64).wrapping_mul(0x100000001b3);
        }
        if px[0].abs_diff(clear[0]) > 2
            || px[1].abs_diff(clear[1]) > 2
            || px[2].abs_diff(clear[2]) > 2
        {
            differ += 1;
        }
    }
    (hash, differ as f64 / (rgba.len() / 4).max(1) as f64)
}

/// One JSON object, built by hand (the arms share no serde).
#[derive(Default)]
pub struct Json(String);
impl Json {
    pub fn num(&mut self, key: &str, value: f64) -> &mut Self {
        let value = if value.is_finite() {
            format!("{value:.6}")
        } else {
            "null".into()
        };
        self.raw(key, &value)
    }
    pub fn int(&mut self, key: &str, value: u64) -> &mut Self {
        self.raw(key, &value.to_string())
    }
    pub fn text(&mut self, key: &str, value: &str) -> &mut Self {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        self.raw(key, &format!("\"{escaped}\""))
    }
    pub fn spread(&mut self, key: &str, values: &[f64]) -> &mut Self {
        let (median, p95) = spread(values);
        self.num(&format!("{key}_median"), median)
            .num(&format!("{key}_p95"), p95)
    }
    fn raw(&mut self, key: &str, value: &str) -> &mut Self {
        if !self.0.is_empty() {
            self.0.push_str(", ");
        }
        let _ = write!(self.0, "\"{key}\": {value}");
        self
    }
    pub fn line(&self) -> String {
        format!("RESULT {{{}}}", self.0)
    }
}

/// Writes RGBA8 rows as a PNG.
pub fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).expect("the PNG's file"));
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .expect("the PNG");
}

/// The generated instance scene's name, `boxes:N:M`: N cubes, the first M
/// moving. `None` for a file.
pub fn boxes(model: &str) -> Option<(usize, usize)> {
    let rest = model.strip_prefix("boxes:")?;
    let (n, m) = rest.split_once(':')?;
    Some((n.parse().ok()?, m.parse().ok()?))
}

/// Where cube `i` of `count` is at `frame`: a square grid 1.5 m apart about
/// the origin, resting on y = 0; a moving one bobs half a metre.
pub fn box_at(i: usize, count: usize, frame: usize) -> [f32; 3] {
    let side = (count as f32).sqrt().ceil() as usize;
    let half = side as f32 * 0.75;
    let bob = 0.5 * (0.05 * frame as f32 + 0.1 * i as f32).sin();
    [
        (i % side) as f32 * 1.5 - half,
        0.5 + if frame > 0 { bob } else { 0. },
        (i / side) as f32 * 1.5 - half,
    ]
}

/// Point light `i` of the light scene: somewhere over Bistro's quarter
/// (x −40..90, z −60..70, 1–6 m up), its colour a warm-to-cool mix; 150 lm,
/// reaching 8 m. The Filament arm computes the same.
pub fn light_at(i: usize) -> ([f32; 3], [f32; 3]) {
    let mut state = (i as u32).wrapping_mul(2654435761).wrapping_add(12345);
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state % 10_000) as f32 / 10_000.
    };
    let position = [-40. + 130. * next(), 1. + 5. * next(), -60. + 130. * next()];
    let warm = next();
    (position, [1., 0.6 + 0.4 * (1. - warm), 0.4 + 0.6 * warm])
}

/// The GPU's busy seconds in `intervals` (their union) and the span they
/// cover, first start to last end.
pub fn busy(mut intervals: Vec<(f64, f64)>) -> (f64, f64) {
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let (mut total, mut current): (f64, Option<(f64, f64)>) = (0., None);
    for (start, end) in &intervals {
        current = match current {
            Some((s, e)) if *start <= e => Some((s, e.max(*end))),
            Some((s, e)) => {
                total += e - s;
                Some((*start, *end))
            }
            None => Some((*start, *end)),
        };
    }
    if let Some((s, e)) = current {
        total += e - s;
    }
    let covered = match (
        intervals.first(),
        intervals.iter().map(|i| i.1).reduce(f64::max),
    ) {
        (Some(first), Some(last)) => last - first.0,
        _ => 0.,
    };
    (total, covered)
}

/// `materials:N`: N meshes, each with a material of its own (and every
/// fourth a texture of 64): many draws that don't batch alike.
pub fn materials(model: &str) -> Option<usize> {
    model.strip_prefix("materials:")?.parse().ok()
}

/// `hierarchy:D:F`: a tree D levels deep, F children a node, cubes at its
/// leaves; the F subtrees under the root turn every frame, so every leaf's
/// world placement changes through its parents.
pub fn hierarchy(model: &str) -> Option<(usize, usize)> {
    let (d, f) = model.strip_prefix("hierarchy:")?.split_once(':')?;
    Some((d.parse().ok()?, f.parse().ok()?))
}
