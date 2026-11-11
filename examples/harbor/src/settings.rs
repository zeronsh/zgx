//! The settings panel's values: each a signal, each bound to the world by
//! an effect, so a control only writes its signal and the world follows.
//! The bench sets the same signals; nothing writes the world beside them.
use crate::scene::{self, Harbor};
use zgx::reactive::{Effect, Runtime, Signal};
use zgx_3d::{ToneMapping, World};

/// What the panel controls.
#[derive(Clone)]
pub struct Settings {
    /// The camera follows the fly-through; off, it orbits under the
    /// pointer.
    pub fly: Signal<bool>,
    /// The sun's elevation, degrees.
    pub sun_elevation: Signal<f32>,
    /// Exposure, stops.
    pub exposure: Signal<f32>,
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
            tone_mapping: runtime.signal(ToneMapping::Aces),
            shadows: runtime.signal(true),
            cascades: runtime.signal(true),
            msaa: runtime.signal(true),
            lamps: runtime.signal(true),
            rain: runtime.signal(harbor.rain.is_some()),
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
            let (s, w) = (self.exposure.clone(), world.clone());
            effects.push(runtime.effect(move || w.set_exposure(s.get() - scene::EV)));
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
        if let Some(rain) = harbor.rain {
            let (s, w) = (self.rain.clone(), world.clone());
            effects.push(runtime.effect(move || rain.set(&w, s.get())));
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
