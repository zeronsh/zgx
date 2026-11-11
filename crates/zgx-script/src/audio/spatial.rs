//! `ui.sound3d` and `ui.reverb_zone` (zgx-script's `audio-spatial`): sound
//! placed in a 3D scene by zgx-3d's world audio. An emitter is an object of
//! the scene's (placed, and a child of a group, as any is) whose sound
//! plays from where it is, falling off with distance, panned, its pitch
//! moved by Doppler and dulled by the meshes between it and the camera. A
//! zone is a box whose reverb the camera hears inside it. A world that
//! nothing moves in sends nothing to the mixer.
use super::{Audio, SoundSlot, SoundUd};
use crate::value::Value;
use mlua::{Lua, Table};
use std::cell::Cell;
use std::rc::Rc;
use zgx_3d::{Emitter3d, ObjectId, ReverbPreset, ReverbZone, Vec3, World, vec3};
use zgx_audio::spatial::Falloff;

/// `ui.sound3d { sound, loop, gain, pitch, start, falloff, cone, doppler,
/// occlusion, air }`.
pub(crate) struct Sound3d {
    slot: Rc<SoundSlot>,
    looping: bool,
    gain: f32,
    /// Its own, times Doppler's.
    pitch: f32,
    /// Seconds into its sound it starts.
    start: f32,
    falloff: Falloff,
    /// Degrees within and past which it's at its full and its outer gain.
    cone: Option<(f32, f32, f32)>,
    doppler: f32,
    occlusion: bool,
    air: bool,
}

/// `ui.reverb_zone { position, size, preset, fade }`.
pub(crate) struct Zone {
    size: [f32; 3],
    preset: ReverbPreset,
    fade: f32,
}

fn refuse_plugins(lua: &Lua, what: &str) -> mlua::Result<()> {
    let env = crate::node::env(lua)?;
    if matches!(*env.grants, crate::Grants::Only(_)) {
        return Err(mlua::Error::runtime(format!(
            "{what}: a plugin's sound plays on its own bus, not in a 3D scene"
        )));
    }
    Ok(())
}

fn number(table: &Table, key: &str, default: f32, what: &str) -> mlua::Result<f32> {
    match table.get::<mlua::Value>(key)? {
        mlua::Value::Nil => Ok(default),
        mlua::Value::Integer(n) => Ok(n as f32),
        mlua::Value::Number(n) if n.is_finite() => Ok(n as f32),
        other => Err(mlua::Error::runtime(format!(
            "{what}'s `{key}` is a finite number, not {}",
            match other {
                mlua::Value::Number(n) => n.to_string(),
                other => other.type_name().to_string(),
            }
        ))),
    }
}

fn flag(table: &Table, key: &str, default: bool) -> mlua::Result<bool> {
    Ok(table.get::<Option<bool>>(key)?.unwrap_or(default))
}

/// Reads `ui.sound3d`'s table.
pub(crate) fn sound3d(lua: &Lua, table: &Table) -> mlua::Result<Sound3d> {
    refuse_plugins(lua, "ui.sound3d")?;
    let slot = match table.get::<mlua::Value>("sound")? {
        mlua::Value::UserData(ud) if ud.is::<SoundUd>() => ud.borrow::<SoundUd>()?.0.clone(),
        _ => {
            return Err(mlua::Error::runtime(
                "ui.sound3d needs `sound = audio.sound(…)`: a sound loaded once, at the top",
            ));
        }
    };
    let falloff = match table.get::<Option<Table>>("falloff")? {
        None => Falloff::inverse(1., 100.),
        Some(f) => {
            let what = "ui.sound3d's falloff";
            let reference = number(&f, "ref", 1., what)?.max(0.001);
            let max = number(&f, "max", 100., what)?.max(reference);
            let rolloff = number(&f, "rolloff", 1., what)?.max(0.);
            match f.get::<Option<String>>("kind")?.as_deref() {
                None | Some("inverse") => Falloff::Inverse {
                    reference,
                    max,
                    rolloff,
                },
                Some("linear") => Falloff::Linear {
                    reference,
                    max,
                    rolloff: rolloff.min(1.),
                },
                Some("exponential") => Falloff::Exponential {
                    reference,
                    max,
                    rolloff,
                },
                Some(other) => {
                    return Err(mlua::Error::runtime(format!(
                        "{what}'s `kind` is \"inverse\", \"linear\" or \"exponential\", not {other:?}"
                    )));
                }
            }
        }
    };
    let cone = match table.get::<Option<Table>>("cone")? {
        None => None,
        Some(c) => {
            let what = "ui.sound3d's cone";
            Some((
                number(&c, "inner", 360., what)?,
                number(&c, "outer", 360., what)?,
                number(&c, "outer_gain", 1., what)?,
            ))
        }
    };
    Ok(Sound3d {
        slot,
        looping: flag(table, "loop", false)?,
        gain: number(table, "gain", 1., "ui.sound3d")?.max(0.),
        pitch: number(table, "pitch", 1., "ui.sound3d")?,
        start: number(table, "start", 0., "ui.sound3d")?.max(0.),
        falloff,
        cone,
        doppler: number(table, "doppler", 1., "ui.sound3d")?.max(0.),
        occlusion: flag(table, "occlusion", true)?,
        air: flag(table, "air", true)?,
    })
}

/// Reads `ui.reverb_zone`'s table.
pub(crate) fn reverb_zone(lua: &Lua, table: &Table) -> mlua::Result<Zone> {
    refuse_plugins(lua, "ui.reverb_zone")?;
    let what = "ui.reverb_zone";
    let size = match table.get::<mlua::Value>("size")? {
        mlua::Value::Table(t) => [
            t.get::<f32>(1)?.abs(),
            t.get::<f32>(2)?.abs(),
            t.get::<f32>(3)?.abs(),
        ],
        mlua::Value::Integer(n) => [n.unsigned_abs() as f32; 3],
        mlua::Value::Number(n) if n.is_finite() => [n.abs() as f32; 3],
        _ => {
            return Err(mlua::Error::runtime(
                "ui.reverb_zone needs a `size`: { x, y, z } metres, or one number for a cube",
            ));
        }
    };
    let preset = match table.get::<mlua::Value>("preset")? {
        mlua::Value::Nil => ReverbPreset::ROOM,
        mlua::Value::String(name) => match &*name.to_str()? {
            "room" => ReverbPreset::ROOM,
            "hall" => ReverbPreset::HALL,
            "warehouse" => ReverbPreset::WAREHOUSE,
            other => {
                return Err(mlua::Error::runtime(format!(
                    "{what}'s `preset` is \"room\", \"hall\", \"warehouse\" or {{ size, decay, damping, wet }}, not {other:?}"
                )));
            }
        },
        mlua::Value::Table(p) => {
            let base = ReverbPreset::ROOM;
            ReverbPreset {
                size: number(&p, "size", base.size, what)?.clamp(0.25, 2.),
                decay: number(&p, "decay", base.decay, what)?.clamp(0.1, 20.),
                damping: number(&p, "damping", base.damping, what)?.clamp(0., 1.),
                wet: number(&p, "wet", base.wet, what)?.clamp(0., 1.),
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "{what}'s `preset` is a name or a table, not {}",
                other.type_name()
            )));
        }
    };
    Ok(Zone {
        size,
        preset,
        fade: number(table, "fade", 2., what)?.max(0.),
    })
}

/// Has `world` play through the engine `sounds`' came from, on its `world`
/// bus, and starts loading any not loaded yet.
pub(crate) fn hear(world: &World, sounds: &[Rc<Sound3d>]) {
    let Some(first) = sounds.first() else {
        return;
    };
    let Some(audio) = engine(first) else {
        return;
    };
    for sound in sounds {
        sound.slot.load(&audio);
    }
    let Some(bus) = audio.bus("world") else {
        return;
    };
    if let Err(error) = world.set_audio(&audio, &bus) {
        eprintln!("ui.scene3d: its sound: {error}");
    }
}

fn engine(sound: &Sound3d) -> Option<Audio> {
    sound.slot.host.upgrade()?.engine().ok()
}

/// What adds `sound`'s emitter to `object` once its sound is ready: the
/// body of an effect that reads whether it is.
pub(crate) fn attach(world: &World, object: ObjectId, sound: Rc<Sound3d>) -> Box<dyn FnMut()> {
    let (world, done) = (world.clone(), Cell::new(false));
    Box::new(move || {
        let ready = sound.slot.ready.with(Value::truthy);
        if done.get() || !ready {
            return;
        }
        let Some(decoded) = sound.slot.sound.borrow().clone() else {
            return;
        };
        done.set(true);
        let mut emitter = Emitter3d::new(&decoded)
            .looping(sound.looping)
            .gain(sound.gain)
            .pitch(sound.pitch)
            .start(sound.start)
            .falloff(sound.falloff.clone())
            .doppler(sound.doppler)
            .occlusion(sound.occlusion)
            .air(sound.air);
        if let Some((inner, outer, outer_gain)) = sound.cone {
            emitter = emitter.cone(inner, outer, outer_gain);
        }
        if let Err(error) = world.add_emitter3d(object, emitter) {
            eprintln!("ui.sound3d: {error}");
        }
    })
}

/// Adds `zone`'s box, centred on `centre` in the scene's coordinates.
pub(crate) fn add_zone(world: &World, centre: Vec3, zone: &Zone) {
    let half = vec3(zone.size[0] / 2., zone.size[1] / 2., zone.size[2] / 2.);
    let added = world.add_reverb_zone(
        ReverbZone::box_(centre - half, centre + half)
            .preset(zone.preset)
            .fade(zone.fade),
    );
    if let Err(error) = added {
        eprintln!("ui.reverb_zone: {error}");
    }
}
