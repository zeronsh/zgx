//! Colour grading in the post chain's last pass (design-3d-aaa.md, 6.1):
//! white balance before the tones are mapped, then lift, gamma, gain,
//! saturation and a look-up table on what the display shows.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// How a frame's colours are graded (`Post::grade`). Each part is as
/// nothing by default; set what it needs with `..Default::default()`.
#[derive(Clone, Debug, PartialEq)]
pub struct Grade {
    /// Warmer (to 1) or cooler (to −1) light: the white point moved along
    /// the daylight locus, adapted as the eye adapts (in LMS, von Kries).
    /// Before the tones are mapped.
    pub temperature: f32,
    /// Toward magenta (to 1) or green (to −1), across the locus.
    pub tint: f32,
    /// Raises the darks toward this, a channel each: 0 none.
    pub lift: [f32; 3],
    /// Bends the middle tones: above 1 brighter, below darker; 1 none.
    pub gamma: [f32; 3],
    /// Scales the lights: 1 none.
    pub gain: [f32; 3],
    /// 0 grey, 1 as it is, above 1 more saturated.
    pub saturation: f32,
    /// A look-up table: applied last, to what the display shows (sRGB
    /// encoded in, sRGB encoded out, as `.cube` files for the display are),
    /// or, made `Lut::in_acescct`, to the scene's light in ACEScct in place
    /// of the tone curve (as an ACES look and output transform's table).
    pub lut: Option<Lut>,
}
impl Default for Grade {
    fn default() -> Self {
        Self {
            temperature: 0.,
            tint: 0.,
            lift: [0.; 3],
            gamma: [1.; 3],
            gain: [1.; 3],
            saturation: 1.,
            lut: None,
        }
    }
}
impl Grade {
    /// The same, every number finite and in its range.
    pub(crate) fn sanitized(&self) -> Self {
        let finite = |v: f32, default: f32| if v.is_finite() { v } else { default };
        Self {
            temperature: finite(self.temperature, 0.).clamp(-1., 1.),
            tint: finite(self.tint, 0.).clamp(-1., 1.),
            lift: self.lift.map(|v| finite(v, 0.).clamp(-1., 1.)),
            gamma: self.gamma.map(|v| finite(v, 1.).clamp(0.01, 100.)),
            gain: self.gain.map(|v| finite(v, 1.).clamp(0., 100.)),
            saturation: finite(self.saturation, 1.).clamp(0., 10.),
            lut: self.lut.clone(),
        }
    }
    /// What white balance multiplies a colour's L, M and S by: the
    /// standard illuminant at `temperature` and `tint` adapted to D65
    /// (as Unity's colour balance computes it, its ±100 as our ±1).
    pub(crate) fn white_balance(&self) -> [f32; 3] {
        let (t1, t2) = (self.temperature * 100. / 65., self.tint * 100. / 65.);
        let x = 0.31271 - t1 * if t1 < 0. { 0.1 } else { 0.05 };
        let standard_y = 2.87 * x - 3. * x * x - 0.275_095_07;
        let y = standard_y + t2 * 0.05;
        // D65 in LMS, and the illuminant at (x, y) with Y = 1.
        let d65 = [0.949_237, 1.035_42, 1.087_28];
        let (cx, cz) = (x / y, (1. - x - y) / y);
        let lms = [
            0.7328 * cx + 0.4296 - 0.1624 * cz,
            -0.7036 * cx + 1.6975 + 0.0061 * cz,
            0.0030 * cx + 0.0136 + 0.9834 * cz,
        ];
        [d65[0] / lms[0], d65[1] / lms[1], d65[2] / lms[2]]
    }
}

/// A 3D colour look-up table, N by N by N entries of RGB, read from a
/// `.cube` file (`from_cube`) or a strip image (`from_strip`). Cheap to
/// clone; uploaded once to each device that draws it.
#[derive(Clone)]
pub struct Lut {
    id: u64,
    size: u32,
    /// The table as a strip: N tables of N by N side by side, blue across
    /// them, red across each, green down; RGBA, 8 bits a channel.
    strip: Arc<[u8]>,
    /// Read in ACEScct, in place of the tone curve (`in_acescct`).
    cct: bool,
}
impl std::fmt::Debug for Lut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lut({0}x{0}x{0})", self.size)
    }
}
impl PartialEq for Lut {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.cct == other.cct
    }
}

/// Why a table couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LutError {
    /// A `.cube` file had no `LUT_3D_SIZE`, one outside 2 to 64, or a line
    /// that isn't a keyword or three numbers.
    Malformed(String),
    /// It held other than size³ entries.
    Entries {
        /// Entries its size says: size³.
        expected: usize,
        /// Entries it held.
        found: usize,
    },
    /// A strip's width isn't its height squared, or its bytes aren't width
    /// times height times 4.
    Strip,
}
impl std::fmt::Display for LutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "not a 3D .cube table: {why}"),
            Self::Entries { expected, found } => {
                write!(f, "a table of {expected} entries holds {found}")
            }
            Self::Strip => f.write_str("a strip's width must be its height squared, RGBA"),
        }
    }
}
impl std::error::Error for LutError {}

static IDS: AtomicU64 = AtomicU64::new(1);

impl Lut {
    /// The table in `text`, a `.cube` file (Adobe's and Resolve's format):
    /// `LUT_3D_SIZE N`, optionally `DOMAIN_MIN` and `DOMAIN_MAX` (applied:
    /// entries are scaled into 0 to 1), then N³ lines of `r g b` with red
    /// changing fastest. `TITLE`, comments (`#`) and blank lines are
    /// skipped.
    pub fn from_cube(text: &str) -> Result<Self, LutError> {
        let malformed = |why: &str| LutError::Malformed(why.into());
        let mut size = None;
        let (mut low, mut high) = ([0f32; 3], [1f32; 3]);
        let mut entries: Vec<[f32; 3]> = Vec::new();
        let three = |words: &[&str]| -> Option<[f32; 3]> {
            match words {
                [r, g, b] => Some([r.parse().ok()?, g.parse().ok()?, b.parse().ok()?]),
                _ => None,
            }
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let words: Vec<&str> = line.split_whitespace().collect();
            match words[0] {
                "TITLE" => {}
                "LUT_1D_SIZE" => return Err(malformed("a 1D table")),
                "LUT_3D_SIZE" => {
                    let n: u32 = words
                        .get(1)
                        .and_then(|n| n.parse().ok())
                        .ok_or_else(|| malformed("LUT_3D_SIZE without a number"))?;
                    if !(2..=64).contains(&n) {
                        return Err(malformed("a size outside 2 to 64"));
                    }
                    size = Some(n);
                }
                "DOMAIN_MIN" => low = three(&words[1..]).ok_or_else(|| malformed("DOMAIN_MIN"))?,
                "DOMAIN_MAX" => high = three(&words[1..]).ok_or_else(|| malformed("DOMAIN_MAX"))?,
                _ => entries.push(three(&words).ok_or_else(|| malformed(line))?),
            }
        }
        let size = size.ok_or_else(|| malformed("no LUT_3D_SIZE"))?;
        let n = size as usize;
        if entries.len() != n * n * n {
            return Err(LutError::Entries {
                expected: n * n * n,
                found: entries.len(),
            });
        }
        let mut strip = vec![0u8; n * n * n * 4];
        for (i, rgb) in entries.iter().enumerate() {
            let (r, g, b) = (i % n, (i / n) % n, i / (n * n));
            let at = (g * n * n + b * n + r) * 4;
            for c in 0..3 {
                let span = (high[c] - low[c]).max(1e-6);
                let v = ((rgb[c] - low[c]) / span).clamp(0., 1.);
                strip[at + c] = (v * 255.).round() as u8;
            }
            strip[at + 3] = 255;
        }
        Ok(Self::made(size, strip))
    }

    /// The table in a strip image `width` by `height` of RGBA bytes: N
    /// tables of N by N side by side (width N², height N), blue across
    /// them, red across each, green down, as Unity's and Unreal's strips.
    pub fn from_strip(width: u32, height: u32, rgba: &[u8]) -> Result<Self, LutError> {
        if !(2..=64).contains(&height)
            || width != height * height
            || rgba.len() != (width * height * 4) as usize
        {
            return Err(LutError::Strip);
        }
        Ok(Self::made(height, rgba.to_vec()))
    }

    /// The table that leaves every colour as it is, `size` entries a side.
    pub fn identity(size: u32) -> Self {
        let n = size.clamp(2, 64) as usize;
        let mut strip = vec![0u8; n * n * n * 4];
        let level = |i: usize| ((i as f32 / (n - 1) as f32) * 255.).round() as u8;
        for g in 0..n {
            for b in 0..n {
                for r in 0..n {
                    let at = (g * n * n + b * n + r) * 4;
                    strip[at..at + 4].copy_from_slice(&[level(r), level(g), level(b), 255]);
                }
            }
        }
        Self::made(n as u32, strip)
    }

    fn made(size: u32, strip: Vec<u8>) -> Self {
        Self {
            id: IDS.fetch_add(1, Ordering::Relaxed),
            size,
            strip: strip.into(),
            cct: false,
        }
    }
    /// The table read in ACEScct (log-encoded ACEScg: the light exposed,
    /// before tone mapping, as ACES's tables take it), its entries what the
    /// display shows: it maps the tones itself, in place of
    /// `World::set_tone_mapping`'s curve.
    pub fn in_acescct(mut self) -> Self {
        self.cct = true;
        self
    }
    /// Whether it's read in ACEScct (`in_acescct`).
    pub fn is_acescct(&self) -> bool {
        self.cct
    }
    /// Entries a side.
    pub fn size(&self) -> u32 {
        self.size
    }
    pub(crate) fn id(&self) -> u64 {
        self.id
    }
    /// The strip's bytes, N² wide and N high.
    pub(crate) fn strip(&self) -> &[u8] {
        &self.strip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cube_file_reads_as_its_strip_and_a_bad_one_says_why() {
        // Size 2, red fastest: entry i is (i & 1, i >> 1 & 1, i >> 2).
        let mut cube = String::from("TITLE \"t\"\n# a comment\nLUT_3D_SIZE 2\n\n");
        for i in 0..8 {
            cube += &format!("{} {} {}\n", i & 1, (i >> 1) & 1, i >> 2);
        }
        let lut = Lut::from_cube(&cube).unwrap();
        assert_eq!(lut, lut.clone());
        assert_eq!(lut.strip(), Lut::identity(2).strip());
        // Scaled from its domain.
        let halved = cube.replace("LUT_3D_SIZE 2", "LUT_3D_SIZE 2\nDOMAIN_MAX 2 2 2");
        assert_eq!(
            Lut::from_cube(&halved).unwrap().strip()[4..8],
            [128, 0, 0, 255]
        );
        assert_eq!(
            Lut::from_cube("LUT_3D_SIZE 2\n0 0 0\n"),
            Err(LutError::Entries {
                expected: 8,
                found: 1
            })
        );
        assert!(matches!(
            Lut::from_cube("LUT_1D_SIZE 4"),
            Err(LutError::Malformed(_))
        ));
        assert!(matches!(
            Lut::from_cube("0 0 0"),
            Err(LutError::Malformed(_))
        ));
        assert_eq!(Lut::from_strip(4, 2, &[0; 31]), Err(LutError::Strip));
        assert!(Lut::from_strip(4, 2, &[0; 32]).is_ok());
    }

    #[test]
    fn white_balance_is_none_at_zero_and_warms_toward_red() {
        let none = Grade::default().white_balance();
        for k in none {
            assert!((k - 1.).abs() < 0.01, "{none:?}");
        }
        let warm = Grade {
            temperature: 0.5,
            ..Default::default()
        }
        .white_balance();
        assert!(warm[0] > warm[2], "long over short: {warm:?}");
    }
}
