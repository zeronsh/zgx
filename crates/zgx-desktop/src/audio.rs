//! The application's sound engine (the `audio` feature): one zgx-audio
//! engine for every window, made the first time something asks for it
//! ([`WindowFactory::audio`](crate::WindowFactory::audio)). Making it opens
//! nothing: the device starts with the first sound and stops a second after
//! the last (zgx-audio's `Config::hold`), so an app that plays nothing is as
//! idle as one without the feature.
//!
//! The engine wakes the event loop when voices end (never polled), and the
//! `ui` bus follows the user's interface-sound setting, read as the engine is
//! made and each time the app comes forward (macOS: "Play user interface
//! sound effects"; elsewhere it counts as on).
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use zgx_audio::{Audio, Config};

/// What the engine tells the app, on the main thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioEvent<'a> {
    /// Voices ended, by `Voice::id`.
    VoicesEnded(&'a [u64]),
    /// The user's interface-sound setting changed: whether interface sounds
    /// (the `ui` bus) play.
    UiSounds(bool),
}

type Listener = dyn Fn(&AudioEvent<'_>);

/// Hears the engine's events while it's held
/// ([`WindowFactory::on_audio`](crate::WindowFactory::on_audio)).
#[must_use = "the listener stops when this is dropped"]
pub struct AudioListening(#[allow(dead_code)] Rc<Listener>);

#[derive(Default)]
struct AppAudio {
    engine: Option<Audio>,
    config: Option<Config>,
    listeners: Vec<Weak<Listener>>,
}

thread_local! {
    /// The app's: the event loop's thread is the main thread.
    static APP: RefCell<AppAudio> = RefCell::default();
}

/// `config` for the engine when it's made (`Application::audio_config`).
pub(crate) fn configure(config: Config) {
    APP.with_borrow_mut(|app| app.config = Some(config));
}

/// The engine, made now if it isn't: `wake` is called, from the engine's
/// own thread, when voices end.
pub(crate) fn engine(wake: impl Fn() + Send + 'static) -> Audio {
    if let Some(audio) = APP.with_borrow(|app| app.engine.clone()) {
        return audio;
    }
    let config = APP
        .with_borrow_mut(|app| app.config.take())
        .unwrap_or_default();
    // An app's config zgx-audio refuses (a rate it can't run at) is said,
    // and the defaults play instead: sound isn't worth failing the app for.
    let audio = Audio::new(config).unwrap_or_else(|error| {
        eprintln!("zgx: audio_config: {error}; zgx-audio's defaults instead");
        Audio::new(Config::default()).expect("zgx-audio's defaults are a config it runs")
    });
    audio.on_voices_ended(wake);
    // No device is touched: it's a command applied before any sound.
    let _ = audio.set_ui_sounds(ui_sounds_enabled());
    APP.with_borrow_mut(|app| app.engine = Some(audio.clone()));
    audio
}

pub(crate) fn listen(listener: impl Fn(&AudioEvent<'_>) + 'static) -> AudioListening {
    let listener: Rc<Listener> = Rc::new(listener);
    APP.with_borrow_mut(|app| {
        app.listeners.retain(|l| l.strong_count() > 0);
        app.listeners.push(Rc::downgrade(&listener));
    });
    AudioListening(listener)
}

fn tell(event: &AudioEvent<'_>) {
    let listeners: Vec<Rc<Listener>> =
        APP.with_borrow(|app| app.listeners.iter().filter_map(Weak::upgrade).collect());
    for listener in listeners {
        listener(event);
    }
}

/// The engine woke the event loop: voices ended.
pub(crate) fn voices_ended() {
    let Some(audio) = APP.with_borrow(|app| app.engine.clone()) else {
        return;
    };
    let ended = audio.take_ended();
    if !ended.is_empty() {
        tell(&AudioEvent::VoicesEnded(&ended));
    }
}

/// The app came forward: the user may have changed the interface-sound
/// setting meanwhile (System Settings is another app), so it's read again.
/// Nothing is read while no engine is made.
pub(crate) fn came_forward() {
    let Some(audio) = APP.with_borrow(|app| app.engine.clone()) else {
        return;
    };
    let enabled = ui_sounds_enabled();
    if enabled != audio.ui_sounds_enabled() {
        let _ = audio.set_ui_sounds(enabled);
        tell(&AudioEvent::UiSounds(enabled));
    }
}

/// The user's interface-sound setting: macOS's global
/// `com.apple.sound.uiaudio.enabled` ("Play user interface sound effects"),
/// absent meaning on. Read only, never written.
#[cfg(target_os = "macos")]
pub(crate) fn ui_sounds_enabled() -> bool {
    use std::ffi::c_void;
    type CFTypeRef = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFPreferencesAnyApplication: CFTypeRef;
        fn CFStringCreateWithBytes(
            alloc: CFTypeRef,
            bytes: *const u8,
            length: isize,
            encoding: u32,
            external: u8,
        ) -> CFTypeRef;
        fn CFPreferencesAppSynchronize(app: CFTypeRef) -> u8;
        fn CFPreferencesCopyAppValue(key: CFTypeRef, app: CFTypeRef) -> CFTypeRef;
        fn CFGetTypeID(value: CFTypeRef) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(value: CFTypeRef) -> u8;
        fn CFNumberGetTypeID() -> usize;
        fn CFNumberGetValue(value: CFTypeRef, kind: isize, out: *mut c_void) -> u8;
        fn CFRelease(value: CFTypeRef);
    }
    /// `kCFStringEncodingUTF8`, `kCFNumberSInt64Type`.
    const UTF8: u32 = 0x0800_0100;
    const SINT64: isize = 4;
    const KEY: &str = "com.apple.sound.uiaudio.enabled";
    // SAFETY: CoreFoundation's documented calls, each object made here
    // released here; the key outlives the read.
    unsafe {
        let key =
            CFStringCreateWithBytes(std::ptr::null(), KEY.as_ptr(), KEY.len() as isize, UTF8, 0);
        if key.is_null() {
            return true;
        }
        // What another process (System Settings) wrote since this one read.
        CFPreferencesAppSynchronize(kCFPreferencesAnyApplication);
        let value = CFPreferencesCopyAppValue(key, kCFPreferencesAnyApplication);
        CFRelease(key);
        if value.is_null() {
            return true;
        }
        let kind = CFGetTypeID(value);
        let enabled = if kind == CFBooleanGetTypeID() {
            CFBooleanGetValue(value) != 0
        } else if kind == CFNumberGetTypeID() {
            let mut number = 1i64;
            CFNumberGetValue(value, SINT64, (&raw mut number).cast());
            number != 0
        } else {
            true
        };
        CFRelease(value);
        enabled
    }
}

/// Elsewhere the setting isn't read yet: interface sounds are on.
#[cfg(not(target_os = "macos"))]
pub(crate) fn ui_sounds_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The setting as `defaults` reads it, absent meaning on (it's read,
    /// never written).
    #[cfg(target_os = "macos")]
    #[test]
    fn the_interface_sound_setting_reads_as_defaults_does() {
        let out = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "-g", "com.apple.sound.uiaudio.enabled"])
            .output()
            .expect("defaults runs");
        let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let expected = !(out.status.success() && said == "0");
        assert_eq!(ui_sounds_enabled(), expected, "defaults says {said:?}");
    }

    #[test]
    fn listeners_hear_until_dropped() {
        let heard = Rc::new(RefCell::new(Vec::new()));
        let into = heard.clone();
        let listening = listen(move |event| {
            if let AudioEvent::UiSounds(on) = event {
                into.borrow_mut().push(*on);
            }
        });
        tell(&AudioEvent::UiSounds(false));
        drop(listening);
        tell(&AudioEvent::UiSounds(true));
        assert_eq!(*heard.borrow(), vec![false]);
    }
}
