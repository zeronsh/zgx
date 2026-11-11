//! The mixer's stock effects in Luau: `audio.low_pass { … }` and its kin
//! describe one, `bus:add_effect` puts it on a bus, and `bus:duck` ducks a
//! bus under another. Each parameter takes a number, a state, a function or
//! a binding, followed as a voice's gain is: one command a change, one a
//! frame while a binding eases, no script per frame. Custom DSP is Rust's
//! (`zgx_audio::Effect`): a script can't run on the audio thread.
use super::{AudioHost, BusUd, Follow, GLIDE, Param, curve, fail, finite};
use crate::value::Value;
use mlua::{Lua, Table, UserData, UserDataMethods};
use std::rc::Rc;
use zgx_audio::effects::{Compressor, Delay, Duck, HighPass, LowPass, Peaking, Reverb, Slope};
use zgx_audio::{EffectHandle, Ramp, Time};

/// Effects a plugin keeps on its bus at once.
pub(super) const PLUGIN_EFFECTS: usize = 4;

/// A stock effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    LowPass,
    HighPass,
    Peaking,
    Delay,
    Reverb,
    Compressor,
    Duck,
}

impl Kind {
    /// Its parameters' names, by the index the engine sets them by, with
    /// the value each starts at when a script doesn't say.
    fn parameters(self) -> &'static [(&'static str, f32)] {
        match self {
            Kind::LowPass => &[("cutoff", 20_000.), ("resonance", zgx_audio::effects::FLAT)],
            Kind::HighPass => &[("cutoff", 20.), ("resonance", zgx_audio::effects::FLAT)],
            Kind::Peaking => &[("frequency", 1_000.), ("q", 1.), ("gain", 0.)],
            Kind::Delay => &[("time", 0.25), ("feedback", 0.5), ("wet", 0.5), ("dry", 1.)],
            Kind::Reverb => &[
                ("size", 1.),
                ("decay", 1.5),
                ("damping", 0.5),
                ("pre_delay", 0.01),
                ("wet", 0.3),
                ("dry", 1.),
            ],
            Kind::Compressor => &[
                ("threshold", -18.),
                ("ratio", 4.),
                ("attack", 0.01),
                ("release", 0.1),
                ("knee", 6.),
                ("makeup", 0.),
            ],
            Kind::Duck => &[
                ("depth", -9.),
                ("threshold", -40.),
                ("attack", 0.05),
                ("release", 0.4),
            ],
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::LowPass => "low_pass",
            Kind::HighPass => "high_pass",
            Kind::Peaking => "peaking",
            Kind::Delay => "delay",
            Kind::Reverb => "reverb",
            Kind::Compressor => "compressor",
            Kind::Duck => "duck",
        }
    }

    fn index(self, name: &str) -> mlua::Result<usize> {
        let parameters = self.parameters();
        parameters
            .iter()
            .position(|(n, _)| *n == name)
            .ok_or_else(|| {
                let names: Vec<&str> = parameters.iter().map(|(n, _)| *n).collect();
                mlua::Error::runtime(format!(
                    "{} has no parameter {name:?}: it has {}",
                    self.name(),
                    names.join(", ")
                ))
            })
    }
}

/// `audio.low_pass { … }` and its kin: an effect described, to put on a
/// bus with `bus:add_effect`.
#[derive(Clone)]
pub(crate) struct SpecUd {
    kind: Kind,
    /// Each parameter's value, by index: the default, a number, or what's
    /// followed.
    params: Vec<Param>,
    /// A filter's: 24 dB an octave rather than 12.
    steep: bool,
    /// A delay's longest time, in seconds: what its line holds.
    longest: f32,
}
impl UserData for SpecUd {}

/// Reads `audio.<kind>`'s options.
fn spec(lua: &Lua, kind: Kind, opts: Option<Table>) -> mlua::Result<SpecUd> {
    let parameters = kind.parameters();
    let mut params: Vec<Param> = parameters.iter().map(|(_, d)| Param::Lit(*d)).collect();
    let (mut steep, mut longest) = (false, None);
    if let Some(opts) = opts {
        for pair in opts.pairs::<String, mlua::Value>() {
            let (key, value) = pair?;
            match (kind, key.as_str()) {
                (Kind::LowPass | Kind::HighPass, "slope") => {
                    steep = match lua.unpack::<f64>(value)? {
                        12. => false,
                        24. => true,
                        other => {
                            return Err(mlua::Error::runtime(format!(
                                "{}'s slope is 12 or 24 (dB an octave), not {other}",
                                kind.name()
                            )));
                        }
                    }
                }
                (Kind::Delay, "longest") => longest = Some(lua.unpack::<f32>(value)?),
                (_, name) => {
                    let at = kind.index(name)?;
                    params[at] = Param::read(
                        lua,
                        value,
                        parameters[at].1,
                        &format!("{}'s `{name}`", kind.name()),
                    )?;
                }
            }
        }
    }
    // A delay's line holds its longest time: a second, or what it starts at
    // if longer, unless the script says (at most 10 s).
    let longest = longest
        .unwrap_or_else(|| params[0].now().max(1.))
        .clamp(0.001, 10.);
    Ok(SpecUd {
        kind,
        params,
        steep,
        longest,
    })
}

/// The constructors in `audio`: `audio.low_pass { cutoff = 800 }`.
pub(super) fn install(lua: &Lua, audio: &Table) -> mlua::Result<()> {
    for kind in [
        Kind::LowPass,
        Kind::HighPass,
        Kind::Peaking,
        Kind::Delay,
        Kind::Reverb,
        Kind::Compressor,
    ] {
        audio.set(
            kind.name(),
            lua.create_function(move |lua, opts: Option<Table>| spec(lua, kind, opts))?,
        )?;
    }
    Ok(())
}

/// An effect on a bus, as the host keeps it: stopped following and taken
/// off when a reload replaces the script that added it.
pub(super) struct LiveEffect {
    pub(super) epoch: u64,
    pub(super) handle: EffectHandle,
    kind: Kind,
    /// What follows state into each parameter, by index.
    follows: Vec<Option<Follow>>,
    pub(super) plugin: Option<Rc<str>>,
    /// Alive while it's on its bus: what a plugin's meter counts its
    /// memory by.
    _held: Rc<()>,
}

/// What an effect holds in memory, made at `rate`: a delay's line, a
/// reverb's lines and pre-delay. Filters and dynamics hold a few numbers.
fn bytes(spec: &SpecUd, rate: u32) -> usize {
    let rate = f64::from(rate);
    match spec.kind {
        Kind::Delay => ((f64::from(spec.longest) * rate).ceil() as usize + 2 + 4096) * 2 * 4,
        // Eight lines of up to 147 ms (at its largest size), and 250 ms of
        // stereo pre-delay.
        Kind::Reverb => (0.147 * rate).ceil() as usize * 8 * 4 + (0.25 * rate).ceil() as usize * 8,
        _ => 256,
    }
}

impl AudioHost {
    /// Puts `spec` on `bus` (keyed to `side`, for a duck), following its
    /// parameters.
    fn add_effect(
        self: &Rc<Self>,
        lua: &Lua,
        bus: &BusUd,
        spec: &SpecUd,
        side: Option<&zgx_audio::Bus>,
        call: &str,
    ) -> mlua::Result<EffectUd> {
        let audio = self.engine()?;
        let held = Rc::new(());
        if let Some(plugin) = &bus.plugin {
            let count = self
                .effects
                .borrow()
                .values()
                .filter(|e| e.plugin.as_deref() == Some(plugin))
                .count();
            if count >= PLUGIN_EFFECTS {
                return Err(mlua::Error::runtime(format!(
                    "{call}: a plugin keeps at most {PLUGIN_EFFECTS} effects on its bus"
                )));
            }
            // Its buffers count against the plugin's memory while it's on.
            if let Some(meter) = crate::node::env(lua)?.meter.clone() {
                let bytes = bytes(spec, audio.rate());
                meter.room(bytes)?;
                let alive = Rc::downgrade(&held);
                meter.hold_native(move || alive.strong_count() > 0, bytes);
            }
        }
        let now: Vec<f32> = spec.params.iter().map(Param::now).collect();
        let slope = if spec.steep { Slope::Db24 } else { Slope::Db12 };
        let added = match spec.kind {
            Kind::LowPass => bus
                .bus
                .add_effect(LowPass::new(now[0]).resonance(now[1]).slope(slope)),
            Kind::HighPass => bus
                .bus
                .add_effect(HighPass::new(now[0]).resonance(now[1]).slope(slope)),
            Kind::Peaking => bus.bus.add_effect(Peaking::new(now[0], now[2], now[1])),
            Kind::Delay => bus.bus.add_effect(
                Delay::new(now[0], spec.longest)
                    .feedback(now[1])
                    .mix(now[2], now[3]),
            ),
            Kind::Reverb => bus.bus.add_effect(
                Reverb::new(now[0], now[1])
                    .damping(now[2])
                    .pre_delay(now[3])
                    .mix(now[4], now[5]),
            ),
            Kind::Compressor => bus.bus.add_effect(
                Compressor::new(now[0], now[1])
                    .times(now[2], now[3])
                    .knee(now[4], now[5]),
            ),
            Kind::Duck => {
                let side = side.expect("a duck has its side");
                bus.bus
                    .duck(side, Duck::new(now[0], now[2], now[3]).threshold(now[1]))
            }
        };
        let handle = added.map_err(|e| fail(call, e))?;
        let mut follows: Vec<Option<Follow>> = Vec::new();
        for (index, param) in spec.params.iter().enumerate() {
            follows.push(match param {
                Param::Followed(followed) => {
                    Some(self.follow_number(followed.clone(), now[index], setter(&handle, index)))
                }
                Param::Lit(_) => None,
            });
        }
        let id = self.next_effect.get() + 1;
        self.next_effect.set(id);
        self.effects.borrow_mut().insert(
            id,
            LiveEffect {
                epoch: self.epoch.get(),
                handle,
                kind: spec.kind,
                follows,
                plugin: bus.plugin.clone(),
                _held: held,
            },
        );
        Ok(EffectUd {
            host: self.clone(),
            id,
            at: None,
        })
    }

    /// Takes off the effects a replaced load added (`end_load`), or every
    /// one (the host going).
    pub(super) fn remove_effects(&self, replaced: impl Fn(u64) -> bool) {
        let gone: Vec<LiveEffect> = {
            let mut effects = self.effects.borrow_mut();
            let ids: Vec<u64> = effects
                .iter()
                .filter(|(_, e)| replaced(e.epoch))
                .map(|(id, _)| *id)
                .collect();
            ids.iter().filter_map(|id| effects.remove(id)).collect()
        };
        for mut effect in gone {
            // It stops following now; it stays on the books (and counted
            // against a plugin's memory) until a full queue takes its
            // removal.
            effect.follows.clear();
            self.send_until(move || effect.handle.remove() != Err(zgx_audio::Error::QueueFull));
        }
    }
}

/// Sets a parameter of `handle`'s: the value, and over how many seconds.
fn setter(handle: &EffectHandle, index: usize) -> super::Setter {
    let handle = handle.clone();
    Rc::new(move |value, seconds| {
        if let Err(error) = handle.set(index, value, Ramp::seconds(seconds)) {
            eprintln!("audio: an effect's parameter: {error}");
        }
    })
}

/// `bus:add_effect`'s handle; `at` places what it's told next on a frame.
pub(crate) struct EffectUd {
    host: Rc<AudioHost>,
    id: u64,
    at: Option<Time>,
}
impl EffectUd {
    /// Its engine handle and kind, while it's on its bus.
    fn live(&self) -> Option<(EffectHandle, Kind)> {
        let effects = self.host.effects.borrow();
        let effect = effects.get(&self.id)?;
        let handle = match self.at {
            Some(at) => effect.handle.at(at),
            None => effect.handle.clone(),
        };
        Some((handle, effect.kind))
    }
    fn replace(&self, index: usize, follow: Option<Follow>) {
        if let Some(effect) = self.host.effects.borrow_mut().get_mut(&self.id) {
            effect.follows[index] = follow;
        }
    }
}
impl UserData for EffectUd {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("set", |lua, e, (name, value): (String, mlua::Value)| {
            let Some((handle, kind)) = e.live() else {
                return Ok(());
            };
            let index = kind.index(&name)?;
            let what = format!("{}'s `{name}`", kind.name());
            let set = setter(&handle, index);
            let follow = match Param::read(lua, value, 0., &what)? {
                Param::Lit(value) => {
                    if finite(&Value::Num(f64::from(value))).is_some() {
                        set(value, GLIDE);
                    }
                    None
                }
                Param::Followed(followed) => {
                    let now = followed.now();
                    set(now, GLIDE);
                    Some(e.host.follow_number(followed, now, set))
                }
            };
            e.replace(index, follow);
            Ok(())
        });
        methods.add_method(
            "fade",
            |_, e, (name, value, seconds, curve_name): (String, f32, f64, Option<String>)| {
                let Some((handle, kind)) = e.live() else {
                    return Ok(());
                };
                let index = kind.index(&name)?;
                let mut ramp = Ramp::seconds(seconds);
                if let Some(name) = curve_name {
                    ramp = ramp.curve(curve(&name)?);
                }
                // A fade takes the parameter over from what followed state.
                e.replace(index, None);
                handle
                    .set(index, value, ramp)
                    .map_err(|error| fail("effect:fade", error))
            },
        );
        methods.add_method("remove", |_, e, ()| {
            let Some((handle, _)) = e.live() else {
                return Ok(());
            };
            // Off the books once the engine has the removal: refused (a
            // full queue), it's still on, and the script may try again.
            handle
                .remove()
                .map_err(|error| fail("effect:remove", error))?;
            e.host.effects.borrow_mut().remove(&e.id);
            Ok(())
        });
        methods.add_method("at", |_, e, seconds: f64| {
            let audio = e.host.engine()?;
            Ok(EffectUd {
                host: e.host.clone(),
                id: e.id,
                at: Some(super::time(&audio, seconds)),
            })
        });
    }
}

/// `bus:add_effect(spec)` and `bus:duck(side, opts)`, on a bus's methods.
pub(super) fn bus_methods<M: UserDataMethods<BusUd>>(methods: &mut M) {
    methods.add_method("add_effect", |lua, b, spec: mlua::AnyUserData| {
        let spec = spec
            .borrow::<SpecUd>()
            .map_err(|_| {
                mlua::Error::runtime(
                    "bus:add_effect takes an effect: audio.low_pass { … }, audio.reverb { … }…",
                )
            })?
            .clone();
        b.host.add_effect(lua, b, &spec, None, "bus:add_effect")
    });
    methods.add_method(
        "duck",
        |lua, b, (side, opts): (mlua::AnyUserData, Option<Table>)| {
            if b.plugin.is_some() {
                return Err(mlua::Error::runtime(
                    "bus:duck: a plugin's bus ducks under no other",
                ));
            }
            let side = side
                .borrow::<BusUd>()
                .map_err(|_| mlua::Error::runtime("bus:duck takes the bus to duck under"))?
                .bus
                .clone();
            let spec = spec(lua, Kind::Duck, opts)?;
            b.host.add_effect(lua, b, &spec, Some(&side), "bus:duck")
        },
    );
}
