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
    /// Whether opening the device has been tried, opened or not.
    pub fn was_tried(&self) -> bool {
        self.0.borrow().is_some()
    }
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.field(key)?.as_f64()
}

/// Gives `host`'s scripts `require("@audio")`, `base` being the folder their
/// sounds' paths count from. Returns `host` and the engine they share (not
/// open: it opens at the first sound a script plays).
pub fn install(host: ScriptHost, base: &Path) -> (ScriptHost, Audio) {
    let host = match package() {
        Ok(dir) => host.package("audio", dir),
        Err(error) => {
            eprintln!("zgx-audio: writing its script: {error}");
            host
        }
    };
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
    (host, audio)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `code` as a script with `@audio` installed; whether it tried to
    /// open the device.
    fn tried(name: &str, code: &str) -> bool {
        let dir =
            std::env::temp_dir().join(format!("zgx-audio-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("app.luau");
        std::fs::write(&entry, code).unwrap();
        let ui = zgx::widgets::Ui::new(1., 1.);
        let (host, audio) = install(ScriptHost::new(&ui.runtime, &entry), &dir);
        host.reload().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        audio.was_tried()
    }

    #[test]
    fn a_script_that_plays_nothing_opens_no_device() {
        assert!(!tried("none", "return ui.div {}"));
        assert!(
            !tried(
                "required",
                "local audio = require(\"@audio\")\nlocal _ = audio\nreturn ui.div {}"
            ),
            "requiring it costs a table, not a device"
        );
    }

    #[test]
    fn its_first_sound_opens_it() {
        // Whether a device exists (a CI runner may have none), it is tried.
        assert!(tried(
            "plays",
            "require(\"@audio\").play(\"missing.wav\")\nreturn ui.div {}"
        ));
    }
}
