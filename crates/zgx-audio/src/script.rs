//! `require("@audio")` for a zgx-script host's scripts. The device opens at
//! the first sound a script plays, not before: a script that requires it
//! and plays nothing costs a table.
//!
//! The prototype's bridge is the host's actions; the design moves it into
//! zgx-script as `ui.sound` behind an `audio` feature (see
//! `docs/audio-design.md`).
use crate::{Engine, Play};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};
use zgx_script::{ScriptHost, Value};

/// The source of the Luau side.
const AUDIO_LUAU: &str = include_str!("../luau/audio.luau");

/// The package's folder, written once a process where the host can read it.
fn package() -> Result<PathBuf, String> {
    static WRITTEN: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    WRITTEN
        .get_or_init(|| {
            use std::hash::{DefaultHasher, Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            AUDIO_LUAU.hash(&mut hasher);
            let dir = std::env::temp_dir()
                .join(format!("zgx-audio-{:016x}", hasher.finish()))
                .join("audio");
            let file = dir.join("init.luau");
            if std::fs::read_to_string(&file).is_ok_and(|on_disk| on_disk == AUDIO_LUAU) {
                return Ok(dir);
            }
            std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
            // Whole or not at all: another process may be reading it.
            let partial = dir.join(format!("init.{}.tmp", std::process::id()));
            std::fs::write(&partial, AUDIO_LUAU).map_err(|error| error.to_string())?;
            std::fs::rename(&partial, &file).map_err(|error| error.to_string())?;
            Ok(dir)
        })
        .clone()
}

/// The engine a host's scripts share, opened at its first sound.
#[derive(Clone, Default)]
pub struct Audio(Rc<RefCell<Option<Result<Engine, String>>>>);

impl Audio {
    /// The engine, opening it the first time; `None` if it couldn't open.
    pub fn with<R>(&self, run: impl FnOnce(&mut Engine) -> R) -> Option<R> {
        let mut engine = self.0.borrow_mut();
        let engine = engine.get_or_insert_with(Engine::open);
        match engine {
            Ok(engine) => Some(run(engine)),
            Err(_) => None,
        }
    }
    /// Whether a script has played anything (and so opened the device).
    pub fn is_open(&self) -> bool {
        matches!(&*self.0.borrow(), Some(Ok(_)))
    }
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.field(key)?.as_f64()
}

/// Gives `host`'s scripts `require("@audio")`, `base` being the folder their
/// sounds' paths count from. Returns `host` and the engine they share.
pub fn install(host: ScriptHost, base: &Path) -> Result<(ScriptHost, Audio), String> {
    let host = host.package("audio", package()?);
    let audio = Audio::default();
    let playing = audio.clone();
    let base = base.to_owned();
    host.action("zgx.audio.play", move |arg| {
        let Some(Value::Str(src)) = arg.field("src") else {
            return;
        };
        let path = base.join(&**src);
        let play = Play {
            volume: number(&arg, "volume").unwrap_or(1.) as f32,
            pan: number(&arg, "pan").unwrap_or(0.) as f32,
            looped: false,
            delay: number(&arg, "delay").unwrap_or(0.),
        };
        let played = playing.with(|engine| {
            let sound = engine.load(&path)?;
            engine.play(&sound, play);
            Ok::<_, String>(())
        });
        if let Some(Err(error)) = played {
            eprintln!("zgx-audio: {error}");
        }
    });
    Ok((host, audio))
}
