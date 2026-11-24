//! Sound in space, as numbers: how loud an emitter is at a distance, in
//! its cone, through the air; where it sits between the speakers; how its
//! pitch moves as it passes. Pure functions, on the caller's thread: an
//! [`Emitter`] turns them into its voice's gain, pan and pitch, sent as
//! ramps over the time to its next update, so a fast pass doesn't step.
//!
//! Coordinates are right-handed, y up, the listener looking down -z by
//! default (glTF's and OpenAL's).
use crate::effects::LowPass;
use crate::{EffectHandle, Error, Ramp, Time, Voice};
use std::sync::Arc;

/// A point or a direction: x, y, z.
pub type Vec3 = [f32; 3];

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}
/// `a` made one long, or `None` for a zero (or not finite) vector.
fn unit(a: Vec3) -> Option<Vec3> {
    let l = length(a);
    (l > 1e-9 && l.is_finite()).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

/// How an emitter's gain falls with distance: from 1 at its reference
/// distance (and nearer) to what the curve gives at its maximum (and
/// further, where it holds).
#[derive(Clone)]
pub enum Falloff {
    /// `reference / (reference + rolloff (d - reference))`: the inverse
    /// distance law, -6 dB a doubling at a rolloff of 1.
    Inverse {
        reference: f32,
        max: f32,
        rolloff: f32,
    },
    /// Straight down from 1 at the reference to `1 - rolloff` at the
    /// maximum.
    Linear {
        reference: f32,
        max: f32,
        rolloff: f32,
    },
    /// `(d / reference)^-rolloff`.
    Exponential {
        reference: f32,
        max: f32,
        rolloff: f32,
    },
    /// The app's own: its gain at a distance, kept from 0 to 1.
    Custom(Arc<dyn Fn(f32) -> f32 + Send + Sync>),
}

impl std::fmt::Debug for Falloff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Falloff::Inverse {
                reference,
                max,
                rolloff,
            } => {
                write!(f, "Inverse({reference}, {max}, {rolloff})")
            }
            Falloff::Linear {
                reference,
                max,
                rolloff,
            } => {
                write!(f, "Linear({reference}, {max}, {rolloff})")
            }
            Falloff::Exponential {
                reference,
                max,
                rolloff,
            } => {
                write!(f, "Exponential({reference}, {max}, {rolloff})")
            }
            Falloff::Custom(_) => f.write_str("Custom"),
        }
    }
}

impl Falloff {
    /// The inverse distance law from `reference` to `max`, rolloff 1.
    pub fn inverse(reference: f32, max: f32) -> Falloff {
        Falloff::Inverse {
            reference,
            max,
            rolloff: 1.,
        }
    }

    /// Straight down from 1 at `reference` to 0 at `max`.
    pub fn linear(reference: f32, max: f32) -> Falloff {
        Falloff::Linear {
            reference,
            max,
            rolloff: 1.,
        }
    }

    /// `(d / reference)^-rolloff` up to `max`.
    pub fn exponential(reference: f32, max: f32, rolloff: f32) -> Falloff {
        Falloff::Exponential {
            reference,
            max,
            rolloff,
        }
    }

    /// The app's own curve.
    pub fn custom(curve: impl Fn(f32) -> f32 + Send + Sync + 'static) -> Falloff {
        Falloff::Custom(Arc::new(curve))
    }

    /// The gain at `distance`, from 0 to 1. A reference that isn't above
    /// zero is taken as a millimetre, a maximum below it as it.
    pub fn gain(&self, distance: f32) -> f32 {
        let distance = if distance.is_nan() {
            0.
        } else {
            distance.max(0.)
        };
        let clamp = |reference: f32, max: f32| {
            let reference = if reference > 1e-3 { reference } else { 1e-3 };
            let max = if max >= reference { max } else { reference };
            (reference, max, distance.clamp(reference, max))
        };
        let gain = match *self {
            Falloff::Inverse {
                reference,
                max,
                rolloff,
            } => {
                let (r, _, d) = clamp(reference, max);
                r / (r + rolloff.max(0.) * (d - r))
            }
            Falloff::Linear {
                reference,
                max,
                rolloff,
            } => {
                let (r, m, d) = clamp(reference, max);
                if m > r {
                    1. - rolloff.clamp(0., 1.) * (d - r) / (m - r)
                } else {
                    1.
                }
            }
            Falloff::Exponential {
                reference,
                max,
                rolloff,
            } => {
                let (r, _, d) = clamp(reference, max);
                crate::math::powf(d / r, -rolloff.max(0.))
            }
            Falloff::Custom(ref curve) => curve(distance),
        };
        if gain.is_nan() {
            0.
        } else {
            gain.clamp(0., 1.)
        }
    }
}

/// Where an emitter sends its sound: whole within `inner` degrees of its
/// direction (the cone's full angle), `outer_gain` past `outer`, in a
/// straight line between.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cone {
    pub inner: f32,
    pub outer: f32,
    pub outer_gain: f32,
}

impl Cone {
    pub fn new(inner: f32, outer: f32, outer_gain: f32) -> Cone {
        Cone {
            inner,
            outer,
            outer_gain,
        }
    }

    /// The gain toward a listener `to_listener` from an emitter facing
    /// `facing`.
    pub fn gain(&self, facing: Vec3, to_listener: Vec3) -> f32 {
        let (Some(facing), Some(to)) = (unit(facing), unit(to_listener)) else {
            return 1.;
        };
        let angle = crate::math::acosf(dot(facing, to).clamp(-1., 1.)).to_degrees();
        let (inner, outer) = (
            self.inner.clamp(0., 360.) / 2.,
            self.outer.clamp(0., 360.) / 2.,
        );
        let outer_gain = self.outer_gain.clamp(0., 1.);
        if angle <= inner {
            1.
        } else if angle >= outer || outer <= inner {
            outer_gain
        } else {
            1. + (outer_gain - 1.) * (angle - inner) / (outer - inner)
        }
    }
}

/// The cutoff, in hertz, of a [`LowPass`] (12 dB, flat) that stands for
/// the air between an emitter and its listener `distance` metres away: one
/// that takes the 0.1 dB a metre air takes at 10 kHz (20 °C, half humid),
/// kept from 500 Hz to 20 kHz.
pub fn air_cutoff(distance: f32) -> f32 {
    let lost = 0.1 * distance.max(0.);
    // A 12 dB Butterworth falls 10 log10(1 + (f / cutoff)^4) dB at f.
    let ratio = crate::math::exp10f(lost / 10.) - 1.;
    if ratio <= 0. {
        return 20_000.;
    }
    (10_000. / ratio.sqrt().sqrt()).clamp(500., 20_000.)
}

/// The speed of sound in air at 20 °C, metres a second.
pub const SPEED_OF_SOUND: f32 = 343.3;

/// The pitch a listener hears an emitter at, for their positions and
/// velocities (metres and metres a second): from their speeds along the
/// line between them, `factor` times as strong (1: as the air would),
/// each kept under the speed of sound, the result kept from 1/4 to 4.
pub fn doppler(
    emitter: Vec3,
    emitter_velocity: Vec3,
    listener: Vec3,
    listener_velocity: Vec3,
    factor: f32,
) -> f32 {
    let Some(toward) = unit(sub(listener, emitter)) else {
        return 1.;
    };
    let limit = SPEED_OF_SOUND * 0.99;
    let factor = if factor.is_finite() {
        factor.max(0.)
    } else {
        0.
    };
    // Each one's speed toward the other.
    let source = (factor * dot(emitter_velocity, toward)).clamp(-limit, limit);
    let hearer = (factor * dot(listener_velocity, toward)).clamp(-limit, limit);
    let pitch = (SPEED_OF_SOUND - hearer) / (SPEED_OF_SOUND - source);
    if pitch.is_finite() {
        pitch.clamp(0.25, 4.)
    } else {
        1.
    }
}

/// Left and right gains for a pan from -1 to 1 by the equal-power law: a
/// quarter turn of cosine and sine, so the power is the same anywhere and
/// the middle is -3 dB a side. (A voice's own pan is a balance, whole in
/// the middle; [`PanLaw`](crate::PanLaw) chooses.)
pub fn equal_power(pan: f32) -> [f32; 2] {
    let pan = if pan.is_nan() { 0. } else { pan.clamp(-1., 1.) };
    let angle = (pan + 1.) * std::f32::consts::FRAC_PI_4;
    let (sin, cos) = crate::math::sin_cosf(angle);
    [cos, sin]
}

/// The gain each speaker of a horizontal layout plays a sound from
/// `azimuth` degrees with: vector-base amplitude panning (Pulkki) between
/// the two speakers either side of it, at the same power wherever it is.
/// `speakers` are azimuths in degrees, 0 ahead, positive to the right
/// (5.1's front and surrounds: `[-30., 30., 0., -110., 110.]`). Writes
/// one gain a speaker into `gains`.
pub fn vbap(speakers: &[f32], azimuth: f32, gains: &mut [f32]) {
    gains.iter_mut().for_each(|g| *g = 0.);
    let n = speakers.len().min(gains.len());
    if n == 0 {
        return;
    }
    let wrap = |a: f32| a.rem_euclid(360.);
    let source = wrap(if azimuth.is_nan() { 0. } else { azimuth });
    if n == 1 {
        gains[0] = 1.;
        return;
    }
    // The two speakers either side: the nearest clockwise and
    // anticlockwise, round the circle.
    let (mut before, mut after) = (0, 0);
    let (mut gap_before, mut gap_after) = (f32::MAX, f32::MAX);
    for (i, &s) in speakers[..n].iter().enumerate() {
        let back = wrap(source - s);
        let ahead = wrap(s - source);
        if back < gap_before {
            (gap_before, before) = (back, i);
        }
        if ahead < gap_after {
            (gap_after, after) = (ahead, i);
        }
    }
    if before == after || gap_before == 0. {
        gains[before] = 1.;
        return;
    }
    // Solve source = g1 l1 + g2 l2 for the two speakers' unit vectors.
    let direction = |degrees: f32| {
        let (sin, cos) = crate::math::sin_cosf(degrees.to_radians());
        [sin, cos]
    };
    let (p, l1, l2) = (
        direction(source),
        direction(speakers[before]),
        direction(speakers[after]),
    );
    let det = l1[0] * l2[1] - l1[1] * l2[0];
    if det.abs() < 1e-6 {
        // Opposite speakers: the nearer alone.
        let i = if gap_before <= gap_after {
            before
        } else {
            after
        };
        gains[i] = 1.;
        return;
    }
    let g1 = (p[0] * l2[1] - p[1] * l2[0]) / det;
    let g2 = (l1[0] * p[1] - l1[1] * p[0]) / det;
    let (g1, g2) = (g1.max(0.), g2.max(0.));
    let power = (g1 * g1 + g2 * g2).sqrt().max(1e-9);
    gains[before] = g1 / power;
    gains[after] = g2 / power;
}

/// Where the sound is heard from: a position, which way is ahead and
/// which up, and how fast it moves (for Doppler).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    pub velocity: Vec3,
}

impl Default for Listener {
    fn default() -> Listener {
        Listener {
            position: [0.; 3],
            forward: [0., 0., -1.],
            up: [0., 1., 0.],
            velocity: [0.; 3],
        }
    }
}

impl Listener {
    /// From -1 (left) to 1 (right): the sine of where `point` is round
    /// the listener's head (ahead and behind both 0).
    pub fn pan(&self, point: Vec3) -> f32 {
        let (Some(to), Some(right)) = (
            unit(sub(point, self.position)),
            unit(cross(self.forward, self.up)),
        ) else {
            return 0.;
        };
        dot(to, right).clamp(-1., 1.)
    }
}

/// What an emitter's voice is given for where it is: its gain (fall-off,
/// cone, its own), its pan, its pitch (Doppler) and the air's cutoff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub gain: f32,
    pub pan: f32,
    pub pitch: f32,
    pub cutoff: f32,
}

/// A sound at a place: its voice, how it falls off, its cone and its
/// Doppler factor. [`update`](Emitter::update) it as it or the listener
/// moves; it sends its voice the new gain, pan and pitch as ramps over the
/// time to the next update. Play its voice with
/// [`PanLaw::EqualPower`](crate::PanLaw::EqualPower), so it keeps its
/// power as it goes round.
#[derive(Clone, Debug)]
pub struct Emitter {
    pub voice: Voice,
    pub falloff: Falloff,
    pub cone: Option<Cone>,
    /// Which way it faces, for its cone.
    pub facing: Vec3,
    /// Its own gain, before the rest.
    pub gain: f32,
    pub doppler: f32,
    /// A [`LowPass`] on the bus its voice plays into, if it has one: the
    /// air's cutoff is set on it each update.
    pub air: Option<EffectHandle>,
}

impl Emitter {
    /// `voice` as an emitter falling off by `falloff`: no cone, its own
    /// gain 1, Doppler as the air would.
    pub fn new(voice: Voice, falloff: Falloff) -> Emitter {
        Emitter {
            voice,
            falloff,
            cone: None,
            facing: [0., 0., -1.],
            gain: 1.,
            doppler: 1.,
            air: None,
        }
    }

    /// Where it is, heard by `listener`: what its voice is given.
    pub fn place(&self, position: Vec3, velocity: Vec3, listener: &Listener) -> Placement {
        let to_listener = sub(listener.position, position);
        let distance = length(to_listener);
        let cone = self
            .cone
            .map_or(1., |cone| cone.gain(self.facing, to_listener));
        Placement {
            gain: self.gain * self.falloff.gain(distance) * cone,
            pan: listener.pan(position),
            pitch: doppler(
                position,
                velocity,
                listener.position,
                listener.velocity,
                self.doppler,
            ),
            cutoff: air_cutoff(distance),
        }
    }

    /// Places it and sends its voice the gain, pan and pitch, each a ramp
    /// over `until_next` seconds (the time to the next update). Returns
    /// what it sent.
    pub fn update(
        &self,
        position: Vec3,
        velocity: Vec3,
        listener: &Listener,
        until_next: f64,
    ) -> Result<Placement, Error> {
        self.send(
            &self.voice,
            self.air.clone(),
            position,
            velocity,
            listener,
            until_next,
        )
    }

    /// As [`update`](Emitter::update), its ramps starting on frame `at` of
    /// the mixer's clock: what a render that must come out the same at any
    /// block size uses.
    pub fn update_at(
        &self,
        at: Time,
        position: Vec3,
        velocity: Vec3,
        listener: &Listener,
        until_next: f64,
    ) -> Result<Placement, Error> {
        let air = self.air.as_ref().map(|air| air.at(at));
        self.send(
            &self.voice.at(at),
            air,
            position,
            velocity,
            listener,
            until_next,
        )
    }

    fn send(
        &self,
        voice: &Voice,
        air: Option<EffectHandle>,
        position: Vec3,
        velocity: Vec3,
        listener: &Listener,
        until_next: f64,
    ) -> Result<Placement, Error> {
        let placed = self.place(position, velocity, listener);
        let ramp = Ramp::seconds(until_next);
        voice.set_gain(placed.gain, ramp)?;
        voice.set_pan(placed.pan, ramp)?;
        voice.set_pitch(placed.pitch, ramp)?;
        if let Some(air) = air {
            air.set(LowPass::CUTOFF, placed.cutoff, ramp)?;
        }
        Ok(placed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, e: f32) -> bool {
        (a - b).abs() <= e
    }

    #[test]
    fn each_falloff_is_whole_at_its_reference_and_holds_past_its_maximum() {
        let inverse = Falloff::inverse(2., 60.);
        assert_eq!(inverse.gain(0.), 1.);
        assert_eq!(inverse.gain(2.), 1.);
        // -6 dB a doubling.
        assert!(close(inverse.gain(4.), 0.5, 1e-6));
        assert!(close(inverse.gain(8.), 0.25, 1e-6));
        assert_eq!(inverse.gain(60.), inverse.gain(1_000.));
        // A rolloff of 2 falls twice as fast: 2 / (2 + 2 x 2) at 4 m.
        let steeper = Falloff::Inverse {
            reference: 2.,
            max: 60.,
            rolloff: 2.,
        };
        assert!(close(steeper.gain(4.), 1. / 3., 1e-6));
        let linear = Falloff::linear(1., 11.);
        assert!(close(linear.gain(6.), 0.5, 1e-6));
        assert_eq!(linear.gain(11.), 0.);
        assert_eq!(linear.gain(50.), 0.);
        let exponential = Falloff::exponential(1., 100., 2.);
        assert!(close(exponential.gain(10.), 0.01, 1e-6));
        assert!(close(exponential.gain(1_000.), 1e-4, 1e-9));
        let custom = Falloff::custom(|d| if d < 5. { 1. } else { 2. });
        assert_eq!((custom.gain(1.), custom.gain(9.)), (1., 1.), "kept to 1");
        // What can't be a distance or a curve: no panic, a gain.
        for f in [inverse, linear, exponential, Falloff::inverse(-1., -5.)] {
            for d in [f32::NAN, f32::INFINITY, -3.] {
                let g = f.gain(d);
                assert!((0. ..=1.).contains(&g), "{f:?} at {d}: {g}");
            }
        }
    }

    #[test]
    fn a_cone_is_whole_inside_quiet_outside_and_a_line_between() {
        let cone = Cone::new(60., 140., 0.3);
        let facing = [0., 0., -1.];
        let at = |degrees: f32| {
            let r = degrees.to_radians();
            cone.gain(facing, [r.sin(), 0., -r.cos()])
        };
        assert_eq!(at(0.), 1.);
        assert_eq!(at(29.), 1.);
        assert!(close(at(50.), 1. - 0.7 * 0.5, 1e-4), "{}", at(50.));
        assert_eq!(at(71.), 0.3);
        assert_eq!(at(180.), 0.3);
        assert_eq!(cone.gain([0.; 3], [1., 0., 0.]), 1., "no direction");
    }

    #[test]
    fn the_air_takes_more_of_the_highs_the_further_it_is() {
        assert_eq!(air_cutoff(0.), 20_000.);
        let (near, far) = (air_cutoff(20.), air_cutoff(200.));
        assert!(near > far && far >= 500.);
        // 100 m: 10 dB at 10 kHz, from a 12 dB low-pass at its cutoff.
        let cutoff = air_cutoff(100.);
        let lost = 10. * (1. + (10_000f32 / cutoff).powi(4)).log10();
        assert!(close(lost, 10., 1e-3), "{lost}");
        assert_eq!(air_cutoff(1e9), 500.);
    }

    #[test]
    fn doppler_raises_what_comes_and_lowers_what_goes() {
        let still = [0.; 3];
        // An emitter coming at 34.33 m/s from ahead: a tenth of the speed
        // of sound, 1 / 0.9 the pitch.
        let coming = doppler([0., 0., -100.], [0., 0., 34.33], still, still, 1.);
        assert!(close(coming, 1. / 0.9, 1e-4), "{coming}");
        let going = doppler([0., 0., -100.], [0., 0., -34.33], still, still, 1.);
        assert!(close(going, 1. / 1.1, 1e-4), "{going}");
        // The listener moving toward it: 1.1.
        let toward = doppler([0., 0., -100.], still, still, [0., 0., -34.33], 1.);
        assert!(close(toward, 1.1, 1e-4), "{toward}");
        // Across, not along: nothing. Off: nothing. Past sound: clamped.
        assert_eq!(
            doppler([0., 0., -100.], [50., 0., 0.], still, still, 1.),
            1.
        );
        assert_eq!(
            doppler([0., 0., -100.], [0., 0., 34.33], still, still, 0.),
            1.
        );
        let fast = doppler([0., 0., -100.], [0., 0., 1e6], still, still, 1.);
        assert_eq!(fast, 4.);
        assert_eq!(doppler(still, [1.; 3], still, still, 1.), 1., "together");
    }

    #[test]
    fn equal_power_keeps_the_power_and_is_3_db_down_a_side_in_the_middle() {
        for i in -10..=10 {
            let [l, r] = equal_power(i as f32 / 10.);
            assert!(close(l * l + r * r, 1., 1e-6));
        }
        let [l, r] = equal_power(0.);
        assert!(close(l, std::f32::consts::FRAC_1_SQRT_2, 1e-6) && close(l, r, 1e-6));
        assert!(equal_power(-1.)[1].abs() < 1e-7);
        assert_eq!(equal_power(f32::NAN), equal_power(0.));
    }

    #[test]
    fn vbap_plays_from_the_pair_either_side_at_one_power() {
        let surround = [-30., 30., 0., -110., 110.];
        let mut gains = [0f32; 5];
        // On a speaker: that one alone.
        vbap(&surround, 30., &mut gains);
        assert_eq!(gains, [0., 1., 0., 0., 0.]);
        // Between centre and right: those two, equally at 15 degrees.
        vbap(&surround, 15., &mut gains);
        assert!(close(gains[1], gains[2], 1e-5) && gains[1] > 0.);
        assert_eq!((gains[0], gains[3], gains[4]), (0., 0., 0.));
        // Behind: the two surrounds; and the power is 1 everywhere.
        vbap(&surround, 180., &mut gains);
        assert!(close(gains[3], gains[4], 1e-5) && gains[3] > 0.);
        for degrees in (0..360).step_by(7) {
            vbap(&surround, degrees as f32, &mut gains);
            let power: f32 = gains.iter().map(|g| g * g).sum();
            assert!(close(power, 1., 1e-5), "{degrees}: {gains:?}");
        }
        // Stereo, a source between: the nearer louder.
        let mut two = [0f32; 2];
        vbap(&[-30., 30.], 10., &mut two);
        assert!(two[1] > two[0]);
    }

    #[test]
    fn a_listener_hears_the_right_on_the_right() {
        let listener = Listener::default();
        assert!(close(listener.pan([5., 0., 0.]), 1., 1e-6));
        assert!(close(listener.pan([-5., 0., 0.]), -1., 1e-6));
        assert!(close(listener.pan([0., 0., -5.]), 0., 1e-6));
        assert!(close(listener.pan([0., 0., 5.]), 0., 1e-6), "behind");
        let turned = Listener {
            forward: [1., 0., 0.],
            ..Listener::default()
        };
        // Facing +x, +z is on the right.
        assert!(close(turned.pan([0., 0., 5.]), 1., 1e-6));
    }
}
