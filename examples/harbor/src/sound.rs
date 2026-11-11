//! The harbor's sound, on zgx-3d's spatial audio: each crane's drive on its
//! trolley (moving with the animator), the ship's generator at its funnel,
//! a bell on the buoy out in the water, rain over everything, and reverb
//! under each gantry's portal and in the container canyons. Every sound is
//! synthesised (`sounds.rs`).
//!
//! Two engines: offline (`HarborSound::offline`, a mixer the caller runs,
//! no device opened: the bench, the tests and every automated run, master's
//! gain 1 and the mix measured), and the system's output
//! (`HarborSound::device`), for a person running the window, which plays
//! at its normal level. An unattended run never opens the device: the
//! window's `--smoke-test` and `HARBOR_MUTE=1` take the offline engine.
use crate::{
    scene::Harbor,
    sounds::{self, RATE},
};
use zgx_3d::{Emitter3d, ReverbPreset, ReverbZone, World};
use zgx_audio::{Audio, Config, Error, Offline, Play, Sound, Voice, spatial::Falloff};

/// Steel and stacks: a large, hard space, not long.
const GANTRY: ReverbPreset = ReverbPreset {
    size: 1.4,
    decay: 1.8,
    damping: 0.2,
    wet: 0.3,
};
/// Between the stacks: close walls of steel.
const CANYON: ReverbPreset = ReverbPreset {
    size: 0.9,
    decay: 1.1,
    damping: 0.35,
    wet: 0.22,
};

/// The harbor's sound: its engine and the mixer the caller runs.
pub struct HarborSound {
    /// The engine.
    pub audio: Audio,
    /// Its mixer, offline: run it with `mix`. `None` for the device,
    /// whose own thread runs it.
    pub mixer: Option<Offline>,
    /// The rain's bed, playing while it rains.
    pub rain: Option<Voice>,
    /// Emitters placed on objects.
    pub emitters: usize,
}

fn sound(samples: &[f32]) -> Result<Sound, Error> {
    Sound::from_samples(samples, 1, RATE)
}

impl HarborSound {
    /// Sound for `harbor` in `world`, mixed offline: no device is opened.
    pub fn offline(world: &World, harbor: &Harbor) -> Result<Self, Error> {
        let (audio, mixer) = Audio::offline(Config::default())?;
        Self::on(world, harbor, audio, Some(mixer))
    }

    /// Sound for `harbor` in `world` through the system's output, at its
    /// normal level: for a person running the window, never an unattended
    /// run.
    pub fn device(world: &World, harbor: &Harbor) -> Result<Self, Error> {
        let audio = Audio::new(Config::default())?;
        Self::on(world, harbor, audio, None)
    }

    fn on(
        world: &World,
        harbor: &Harbor,
        audio: Audio,
        mixer: Option<Offline>,
    ) -> Result<Self, Error> {
        let bus = audio
            .bus("world")
            .ok_or_else(|| Error::Effect("no world bus".into()))?;
        world.set_audio(&audio, &bus)?;
        // The sea lies under every way from a sound to the listener: it
        // blocks none of them (and its grid's triangles needn't be tried).
        world.set_sound_transmission(harbor.water, 1.);
        if let Some(sea) = &harbor.sea {
            world.set_sound_transmission(sea.near, 1.);
        }
        let seed = harbor.config.seed;
        let mut emitters = 0;
        let motor = sound(&sounds::crane_motor(seed))?;
        for g in &harbor.gantries {
            world.add_emitter3d(
                g.trolley,
                Emitter3d::new(&motor)
                    .looping(true)
                    .gain(0.6)
                    .falloff(Falloff::Inverse {
                        reference: 8.,
                        max: 400.,
                        rolloff: 1.,
                    }),
            )?;
            emitters += 1;
        }
        world.add_emitter3d(
            harbor.funnel,
            Emitter3d::new(&sound(&sounds::generator(seed ^ 1))?)
                .looping(true)
                .gain(0.8)
                .falloff(Falloff::Inverse {
                    reference: 12.,
                    max: 600.,
                    rolloff: 1.,
                }),
        )?;
        world.add_emitter3d(
            harbor.buoy,
            Emitter3d::new(&sound(&sounds::bell())?)
                .looping(true)
                .gain(0.7)
                .falloff(Falloff::Inverse {
                    reference: 20.,
                    max: 1500.,
                    rolloff: 0.8,
                }),
        )?;
        emitters += 2;
        for g in &harbor.gantries {
            let x = world.world_matrix(g.root).map_or(0., |m| m.columns[3][0]);
            world.add_reverb_zone(
                ReverbZone::box_([x - 9., 0., 3.], [x + 9., 46., 33.])
                    .preset(GANTRY)
                    .fade(6.),
            )?;
        }
        world.add_reverb_zone(
            ReverbZone::box_([-320., 0., 70.], [320., 13., 380.])
                .preset(CANYON)
                .fade(4.),
        )?;
        // Rain everywhere at once: a bed on the world's bus, not placed.
        let rain = match harbor.rain {
            Some(_) => Some(audio.play(
                &sound(&sounds::rain(seed ^ 2))?,
                Play::new().bus(&bus).looping(true).gain(0.5),
            )?),
            None => None,
        };
        Ok(Self {
            audio,
            mixer,
            rain,
            emitters,
        })
    }

    /// Mix the next `seconds` of sound (offline), what a device would ask
    /// for over a frame: the master's samples, interleaved.
    /// Nothing for the device, which mixes on its own.
    pub fn mix(&mut self, seconds: f32) -> Vec<f32> {
        let frames = (seconds * RATE as f32).round() as usize;
        self.mixer
            .as_mut()
            .map_or_else(Vec::new, |mixer| mixer.frames(frames))
    }
}
