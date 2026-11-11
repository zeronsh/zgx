//! The settings panel's values: each a signal, each bound to the world by
//! an effect, so a control only writes its signal and the world follows.
//! The bench sets the same signals; nothing writes the world beside them.
use crate::scene::{self, Harbor};
use zgx::reactive::{Effect, Runtime, Signal};
use zgx_3d::{AutoExposure, Bloom, Post, ToneMapping, World};

/// What the panel controls.
#[derive(Clone)]
pub struct Settings {
    /// The camera follows the fly-through; off, it orbits under the
    /// pointer.
    pub fly: Signal<bool>,
    /// The sun's elevation, degrees.
    pub sun_elevation: Signal<f32>,
    /// Exposure, stops: over the harbor's own (EV 11), or with
    /// `auto_exposure` on, over where it has adapted to.
    pub exposure: Signal<f32>,
    /// Exposure follows the scene's light, as an eye adapts.
    pub auto_exposure: Signal<bool>,
    /// Light spreads round what's bright, as a lens's glare.
    pub bloom: Signal<bool>,
    /// How light reaches the display's range.
    pub tone_mapping: Signal<ToneMapping>,
    /// The sun casts shadows.
    pub shadows: Signal<bool>,
    /// Its shadow is cut into cascades (off: one map over everything).
    pub cascades: Signal<bool>,
    /// 4x multisampling; off, one sample.
    pub msaa: Signal<bool>,
    /// The yard's lamps are lit.
    pub lamps: Signal<bool>,
    /// It rains.
    pub rain: Signal<bool>,
    /// The sea's waves move.
    pub sea: Signal<bool>,
    /// How much of the yard's containers show, 0 to 1.
    pub yard: Signal<f32>,
    /// The HUD shows.
    pub hud: Signal<bool>,
}

impl Settings {
    /// The defaults, on `runtime`, from `harbor`'s config.
    pub fn new(runtime: &Runtime, harbor: &Harbor) -> Self {
        Self {
            fly: runtime.signal(true),
            sun_elevation: runtime.signal(harbor.config.sun.0),
            exposure: runtime.signal(0.),
            auto_exposure: runtime.signal(true),
            bloom: runtime.signal(true),
            tone_mapping: runtime.signal(ToneMapping::Aces),
            shadows: runtime.signal(true),
            cascades: runtime.signal(true),
            msaa: runtime.signal(true),
            lamps: runtime.signal(true),
            rain: runtime.signal(harbor.rain.is_some()),
            sea: runtime.signal(true),
            yard: runtime.signal(1.),
            hud: runtime.signal(true),
        }
    }

    /// Bind each value to `world`: an effect each, run now and whenever
    /// its signal changes. Keep the effects for as long as the binding.
    pub fn bind(&self, runtime: &Runtime, world: &World, harbor: &Harbor) -> Vec<Effect> {
        let mut effects = Vec::new();
        let azimuth = harbor.config.sun.1;
        {
            let (s, w, sun) = (self.sun_elevation.clone(), world.clone(), harbor.sun);
            effects
                .push(runtime.effect(move || w.set_light(sun, scene::sun_light(s.get(), azimuth))));
        }
        {
            let (s, auto, w) = (
                self.exposure.clone(),
                self.auto_exposure.clone(),
                world.clone(),
            );
            effects.push(runtime.effect(move || {
                let stops = s.get();
                // Adapting, the stops are compensation over where it is.
                w.set_exposure(if auto.get() { stops } else { stops - scene::EV });
            }));
        }
        {
            let (auto, bloom, w) = (
                self.auto_exposure.clone(),
                self.bloom.clone(),
                world.clone(),
            );
            effects.push(runtime.effect(move || w.set_post(post(auto.get(), bloom.get()))));
        }
        {
            let (s, w) = (self.tone_mapping.clone(), world.clone());
            effects.push(runtime.effect(move || w.set_tone_mapping(s.get())));
        }
        {
            let (s, w) = (self.shadows.clone(), world.clone());
            effects.push(runtime.effect(move || w.set_shadows(s.get())));
        }
        {
            let (s, w) = (self.cascades.clone(), world.clone());
            effects.push(runtime.effect(move || {
                w.set_cascades(s.get().then(scene::cascades));
            }));
        }
        {
            let (s, w) = (self.msaa.clone(), world.clone());
            effects.push(runtime.effect(move || w.set_samples(if s.get() { 4 } else { 1 })));
        }
        {
            let (s, w, lamps) = (self.lamps.clone(), world.clone(), harbor.lamps.clone());
            effects.push(runtime.effect(move || {
                let on = s.get();
                for &(id, light) in &lamps {
                    w.set_point_light(id, scene::lamp(light, on));
                }
            }));
        }
        if let Some(sea) = harbor.sea.clone() {
            let (s, w) = (self.sea.clone(), world.clone());
            effects.push(runtime.effect(move || sea.set_moving(&w, s.get())));
        }
        if let Some(rain) = harbor.rain {
            let (s, w, sea) = (self.rain.clone(), world.clone(), harbor.sea.clone());
            effects.push(runtime.effect(move || {
                let on = s.get();
                rain.set(&w, on);
                if let Some(sea) = &sea {
                    sea.set_rain(&w, if on { 1. } else { 0. });
                }
            }));
        }
        {
            let (s, w, yard) = (self.yard.clone(), world.clone(), harbor.yard.clone());
            effects.push(runtime.effect(move || {
                let shown = (s.get().clamp(0., 1.) * yard.len() as f32).round() as usize;
                for (i, &id) in yard.iter().enumerate() {
                    let visible = i < shown;
                    // `update` draws a frame only if `visible` changed.
                    if w.object(id).is_some_and(|o| o.visible != visible) {
                        w.update(id, |o| o.visible = visible);
                    }
                }
            }));
        }
        effects
    }
}

/// The harbor's post chain: bloom and adapting exposure as asked.
pub fn post(auto_exposure: bool, bloom: bool) -> Post {
    Post {
        bloom: bloom.then_some(Bloom { intensity: 0.05 }),
        auto_exposure: auto_exposure.then(|| AutoExposure {
            // In the harbor's units (lux and candela): a lamp-lit night to
            // a bright morning.
            min: -4.,
            max: 16.,
            ..AutoExposure::default()
        }),
        ..Post::default()
    }
}
