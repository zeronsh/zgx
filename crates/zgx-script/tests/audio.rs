//! The `audio` library (zgx-script's `audio` feature) against an offline
//! engine: the test renders the mixer block by block, as a device would,
//! and tells the host which voices ended, as a desktop host does when the
//! engine wakes it.
use std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration};
use zgx::input::{InputEvent, Key};
use zgx_script::{
    ScriptHost,
    testing::TestApp,
    zgx_audio::{
        Audio, Config, Offline,
        wav::{self, Encoding},
    },
};

/// Frames rendered at a time: a device's block.
const BLOCK: usize = 256;

/// A WAV file of `seconds` of a steady level (a click needs no shape to be
/// heard), in a directory of its own.
fn wav(name: &str, seconds: f64) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("zgx-audio-script-{}-{}", std::process::id(), name));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.wav"));
    let samples = vec![0.5_f32; (48_000. * seconds) as usize];
    std::fs::write(
        &path,
        wav::write(&samples, 1, 48_000, Encoding::Float32).unwrap(),
    )
    .unwrap();
    path
}

/// An app running `code` (`{click}` replaced by a sound's path) with an
/// offline engine, and the engine's mixer.
struct Sounding {
    app: TestApp,
    audio: Audio,
    mixer: Offline,
}

impl Sounding {
    fn new(code: &str) -> Sounding {
        Sounding::with(|runtime| ScriptHost::inline(runtime, "app", code))
    }
    fn with(make: impl FnOnce(&zgx::reactive::Runtime) -> ScriptHost) -> Sounding {
        Sounding::configured(Config::default(), make)
    }
    fn configured(
        config: Config,
        make: impl FnOnce(&zgx::reactive::Runtime) -> ScriptHost,
    ) -> Sounding {
        let (audio, mixer) = Audio::offline(config).unwrap();
        let engine = audio.clone();
        let app = TestApp::new(move |runtime| {
            let host = make(runtime);
            host.set_audio(move || engine.clone());
            host
        });
        Sounding { app, audio, mixer }
    }
    /// One block mixed, as a device asks, and the voices that ended told to
    /// the host.
    fn block(&mut self) -> Vec<f32> {
        let out = self.mixer.frames(BLOCK);
        self.app.host.voices_ended(&self.audio.take_ended());
        self.app.settle();
        out
    }
    /// Blocks mixed for `seconds`, the loudest sample of each.
    fn render(&mut self, seconds: f64) -> Vec<f32> {
        let blocks = (seconds * 48_000. / BLOCK as f64).ceil() as usize;
        (0..blocks).map(|_| loudest(&self.block())).collect()
    }
    /// Waits for the sounds the script loads.
    fn ready(&mut self, sounds: usize) {
        assert!(
            self.app
                .wait_until(|app| app.text("ready") == sounds.to_string()),
            "the sounds load"
        );
    }
}

fn loudest(block: &[f32]) -> f32 {
    block.iter().fold(0., |m, s| m.max(s.abs()))
}

fn script(code: &str, sounds: &[(&str, &PathBuf)]) -> String {
    let mut code = code.to_string();
    for (name, path) in sounds {
        code = code.replace(&format!("{{{name}}}"), &path.display().to_string());
    }
    code
}

/// How many of `sounds` are ready, as text `ready` shows.
const READY: &str = r#"
local function count_ready(...)
	local sounds = { ... }
	return function()
		local n = 0
		for _, s in sounds do
			if s.ready:get() then
				n += 1
			end
		end
		return tostring(n)
	end
end
"#;

/// A way a person activates a control.
type Activation = Box<dyn Fn(&mut TestApp)>;

#[test]
fn sound_plays_on_activation_on_the_ui_bus_before_on_click() {
    let click = wav("activation", 0.05);
    let code = script(
        &format!(
            "{READY}
            local click = audio.sound(\"{{click}}\")
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(click) }},
                ui.button {{
                    id = \"go\",
                    sound = click,
                    on_click = function() ui.invoke(\"seen\") end,
                    \"Go\",
                }},
            }}"
        ),
        &[("click", &click)],
    );
    let seen = Rc::new(Cell::new(None));
    let (audio, mixer) = Audio::offline(Config::default()).unwrap();
    let (engine, at_click) = (audio.clone(), seen.clone());
    let app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", code);
        let at = engine.clone();
        host.set_audio(move || engine.clone());
        // What the engine has as the handler runs: the click is sent.
        host.action("seen", move |_| at_click.set(Some(at.voices_playing())));
        host
    });
    let mut s = Sounding { app, audio, mixer };
    s.ready(1);
    // Nothing plays before the activation.
    assert!(s.render(0.02).iter().all(|&l| l == 0.));
    let mut activations: Vec<(&str, Activation)> = vec![
        ("click", Box::new(|app: &mut TestApp| app.click("go"))),
        (
            "Enter",
            Box::new(|app: &mut TestApp| {
                let node = app.target("go").unwrap();
                app.ui.input.focus(&app.ui.scene, Some(node));
                app.key(Key::Enter);
            }),
        ),
        (
            "assistive technology",
            Box::new(|app: &mut TestApp| {
                let node = app.target("go").unwrap();
                app.ui
                    .input
                    .dispatch_to(&app.ui.scene, node, InputEvent::Activate);
                app.settle();
            }),
        ),
    ];
    for (how, activate) in activations.drain(..) {
        seen.set(None);
        activate(&mut s.app);
        assert_eq!(seen.get(), Some(1), "{how}: sent before on_click ran");
        // Heard in the very block after: nothing waits for a frame.
        let first = s.block();
        assert!(loudest(&first) > 0.4, "{how}: {}", loudest(&first));
        // It ends, and the next one starts from silence.
        s.render(0.1);
        assert_eq!(s.audio.voices_playing(), 0, "{how}");
    }
    // On the `ui` bus: the user's setting silences it.
    s.audio.set_ui_sounds(false).unwrap();
    // (It fades out over 5 ms.)
    s.render(0.01);
    s.app.click("go");
    assert!(s.render(0.05).iter().all(|&l| l == 0.), "the ui bus is off");
}

#[test]
fn a_bus_gain_following_state_sends_no_command() {
    let tone = wav("bus-state", 1.);
    let code = script(
        &format!(
            "{READY}
            local tone = audio.sound(\"{{tone}}\")
            local volume = ui.state(\"volume\", 1)
            audio.bus(\"music\"):gain(volume)
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(tone) }},
                ui.button {{
                    id = \"play\",
                    on_click = function() audio.play(tone, {{ bus = \"music\", loop = true }}) end,
                    \"Play\",
                }},
                ui.button {{ id = \"quiet\", on_click = function() volume:set(0.25) end, \"Quiet\" }},
            }}"
        ),
        &[("tone", &tone)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    s.app.click("play");
    let loud = s.render(0.1);
    assert!((loud.last().unwrap() - 0.5).abs() < 0.01, "{loud:?}");
    let sent = s.audio.stats().commands;
    let calls = s.app.calls();
    s.render(100. * BLOCK as f64 / 48_000.);
    assert_eq!(s.audio.stats().commands, sent, "no command per block");
    s.app.click("quiet");
    assert_eq!(s.audio.stats().commands, sent, "nor for a change");
    let quiet = s.render(0.05);
    assert!(
        (quiet.last().unwrap() - 0.125).abs() < 0.01,
        "it glides to the state's gain: {quiet:?}"
    );
    assert_eq!(
        s.app.calls(),
        calls + 1,
        "the click's handler alone ran script"
    );
}

#[test]
fn a_bus_gain_bound_to_a_tween_glides_with_no_script_per_frame() {
    let tone = wav("bus-tween", 2.);
    let code = script(
        &format!(
            "{READY}
            local tone = audio.sound(\"{{tone}}\")
            local volume = ui.state(\"volume\", 1)
            audio.bus(\"music\"):gain(ui.tween(volume, {{ duration = 500 }}))
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(tone) }},
                ui.button {{
                    id = \"play\",
                    on_click = function() audio.play(tone, {{ bus = \"music\", loop = true }}) end,
                    \"Play\",
                }},
                ui.button {{ id = \"quiet\", on_click = function() volume:set(0) end, \"Quiet\" }},
            }}"
        ),
        &[("tone", &tone)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    s.app.click("play");
    s.render(0.05);
    let sent = s.audio.stats().commands;
    s.app.click("quiet");
    let calls = s.app.calls();
    // A frame, then a frame's worth of sound, for half a second.
    let mut levels = Vec::new();
    for _ in 0..30 {
        s.app.frame();
        levels.push(*s.render(1. / 60.).last().unwrap());
    }
    assert_eq!(s.app.calls(), calls, "no script per frame");
    assert_eq!(s.audio.stats().commands, sent, "no command per frame");
    // It eases down, through values between.
    assert!(levels.windows(2).all(|w| w[1] <= w[0] + 1e-4), "{levels:?}");
    assert!(
        levels.iter().any(|&l| l > 0.05 && l < 0.45),
        "a glide: {levels:?}"
    );
    s.app.frames(10);
    let end = s.render(0.05);
    assert!(*end.last().unwrap() < 1e-3, "{end:?}");
}

#[test]
fn a_voice_gain_bound_to_a_tween_is_one_command_a_frame_while_it_eases() {
    let tone = wav("voice-tween", 2.);
    let code = script(
        &format!(
            "{READY}
            local tone = audio.sound(\"{{tone}}\")
            local level = ui.state(\"level\", 1)
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(tone) }},
                ui.button {{
                    id = \"play\",
                    on_click = function()
                        audio.play(tone, {{ loop = true, gain = ui.tween(level, {{ duration = 250 }}) }})
                    end,
                    \"Play\",
                }},
                ui.button {{ id = \"quiet\", on_click = function() level:set(0.2) end, \"Quiet\" }},
            }}"
        ),
        &[("tone", &tone)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    s.app.click("play");
    s.render(0.05);
    let calls = s.app.calls();
    let at_rest = s.audio.stats().commands;
    s.app.frames(20);
    assert_eq!(s.audio.stats().commands, at_rest, "none at rest");
    s.app.click("quiet");
    let sent = s.audio.stats().commands;
    s.app.frames(30);
    let eased = s.audio.stats().commands - sent;
    assert!(
        (10..=20).contains(&eased),
        "one a frame while it eases: {eased}"
    );
    assert_eq!(s.app.calls(), calls + 1, "the click's handler alone");
    let end = s.render(0.05);
    assert!((end.last().unwrap() - 0.1).abs() < 0.01, "{end:?}");
}

/// A script holding the voice its button plays (a plain local: states
/// hold values, not handles), with text `playing` reading its state.
const HOLDS_A_VOICE: &str = r#"
local played = ui.state("played", 0)
local held
local function play(sound)
	return function()
		held = audio.play(sound)
		played:set(played:peek() + 1)
	end
end
local function playing()
	played:get()
	return if held then tostring(held.playing:get()) else "none"
end
"#;

#[test]
fn playing_turns_false_when_the_voice_ends() {
    let click = wav("ends", 0.05);
    let code = script(
        &format!(
            "{READY}{HOLDS_A_VOICE}
            local click = audio.sound(\"{{click}}\")
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(click) }},
                ui.text {{ id = \"playing\", playing }},
                ui.button {{ id = \"play\", on_click = play(click), \"Play\" }},
            }}"
        ),
        &[("click", &click)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    assert_eq!(s.app.text("playing"), "none");
    s.app.click("play");
    assert_eq!(s.app.text("playing"), "true");
    s.render(0.02);
    assert_eq!(s.app.text("playing"), "true", "still playing");
    s.render(0.1);
    assert_eq!(s.app.text("playing"), "false", "ended");
}

#[test]
fn a_play_before_its_sound_is_ready_plays_when_ready_within_250_ms() {
    let click = wav("waits", 0.2);
    let code = script(
        &format!(
            "{READY}{HOLDS_A_VOICE}
            local click = audio.sound(\"{{click}}\", {{ preload = false }})
            local late = ui.state(\"late\", \"\")
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(click) }},
                ui.text {{ id = \"playing\", playing }},
                ui.button {{ id = \"play\", on_click = play(click), \"Play\" }},
                ui.text {{ id = \"late\", late }},
                ui.button {{ id = \"stats\", on_click = function() late:set(tostring(audio.stats().late)) end, \"Stats\" }},
            }}"
        ),
        &[("click", &click)],
    );
    // In time: the play waits for the decode, then starts.
    let mut s = Sounding::new(&code);
    assert_eq!(s.app.text("ready"), "0", "not loaded until played");
    s.app.click("play");
    s.ready(1);
    assert_eq!(s.audio.voices_playing(), 1);
    assert!(s.render(0.05).iter().any(|&l| l > 0.4));
    drop(s);
    // Late: the decode took longer than a click may wait.
    let mut s = Sounding::new(&code);
    let clock = zgx::clock::VirtualClock::new();
    s.app.click("play");
    assert_eq!(s.app.text("playing"), "true", "waiting");
    clock.advance(Duration::from_millis(300), || {});
    s.ready(1);
    assert_eq!(s.audio.voices_playing(), 0, "dropped");
    assert_eq!(s.app.text("playing"), "false");
    s.app.click("stats");
    assert_eq!(s.app.text("late"), "1", "counted");
    assert!(s.render(0.05).iter().all(|&l| l == 0.));
    s.app.click("play");
    assert_eq!(s.audio.voices_playing(), 1, "ready now, it plays at once");
    drop(clock);
}

#[test]
fn a_reload_stops_the_voices_and_streams_the_old_script_started() {
    let tone = wav("reload", 2.);
    let dir = tone.parent().unwrap().to_path_buf();
    let app = dir.join("app.luau");
    let code = |gain: &str| {
        script(
            &format!(
                "{READY}
                local tone = audio.sound(\"{{tone}}\")
                audio.play(tone, {{ loop = true, gain = {gain} }})
                audio.music(\"{{tone}}\", {{ loop = true, gain = {gain} }})
                return ui.text {{ id = \"ready\", count_ready(tone) }}"
            ),
            &[("tone", &tone)],
        )
    };
    std::fs::write(&app, code("0.1")).unwrap();
    let path = app.clone();
    let mut s = Sounding::with(move |runtime| ScriptHost::new(runtime, path));
    s.ready(1);
    let first = s.render(0.1);
    assert!(
        (first.last().unwrap() - 0.1).abs() < 0.01,
        "a sound and a stream, each at 0.1: {first:?}"
    );
    assert_eq!(s.audio.voices_playing(), 2);
    std::fs::write(&app, code("0.2")).unwrap();
    s.app.host.reload().unwrap();
    s.ready(1);
    let after = s.render(0.1);
    // The old voices fade out over 5 ms; the new ones play alone.
    assert_eq!(s.audio.voices_playing(), 2, "only the new script's");
    assert!(
        (after.last().unwrap() - 0.2).abs() < 0.01,
        "not both scripts' at once: {after:?}"
    );
}

#[test]
fn the_engine_is_made_at_the_first_audio_call() {
    let made = Rc::new(Cell::new(0));
    let (audio, _mixer) = Audio::offline(Config::default()).unwrap();
    let host_made = made.clone();
    let mut app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            "local now = ui.state(\"now\", \"\")
            return ui.column {
                ui.text { id = \"now\", now },
                ui.button { id = \"ask\", on_click = function() now:set(tostring(audio.now())) end, \"Now\" },
            }",
        );
        host.set_audio(move || {
            host_made.set(host_made.get() + 1);
            audio.clone()
        });
        host
    });
    app.frames(3);
    assert_eq!(made.get(), 0, "a script that hasn't called audio has none");
    assert!(app.host.audio_engine().is_none());
    app.click("ask");
    assert_eq!(app.text("now"), "0");
    app.click("ask");
    assert_eq!(made.get(), 1, "made once");
}

#[test]
fn audio_without_an_engine_says_how_to_give_one() {
    let mut app = TestApp::inline(
        "local said = ui.state(\"said\", \"\")
        return ui.column {
            ui.text { id = \"said\", said },
            ui.button { id = \"ask\", on_click = function()
                local ok, why = pcall(audio.now)
                said:set(tostring(why))
            end, \"Now\" },
        }",
    );
    app.click("ask");
    assert!(
        app.text("said").contains("ScriptHost::set_audio"),
        "{}",
        app.text("said")
    );
}

#[test]
fn a_plugin_plays_into_its_own_bus_under_ui_at_most_eight_voices() {
    let tone = wav("plugin", 1.);
    let dir = tone.parent().unwrap().to_path_buf();
    let entry = dir.join("init.luau");
    std::fs::write(
        &entry,
        format!(
            "{READY}
            local tone = audio.sound(\"plugin.wav\")
            ui.contribute(\"bar\", ui.column {{
                ui.text {{ id = \"ready\", count_ready(tone) }},
                ui.text {{ id = \"reach\", tostring(audio.bus(\"music\") == nil and audio.prewarm == nil and audio.ui_sounds_enabled == nil) }},
                ui.button {{
                    id = \"nine\",
                    on_click = function()
                        for _ = 1, 9 do
                            audio.play(tone, {{ loop = true, gain = 0.1 }})
                        end
                    end,
                    \"Nine\",
                }},
            }})"
        ),
    )
    .unwrap();
    let mut s = Sounding::with(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", "return ui.slot(\"bar\", { id = \"bar\" })");
        host.set_plugins(vec![zgx_script::Plugin {
            name: "sfx".into(),
            entry,
            grants: vec![],
        }]);
        host
    });
    s.ready(1);
    assert_eq!(
        s.app.text("reach"),
        "true",
        "no other bus, prewarm or setting"
    );
    s.app.click("nine");
    assert_eq!(s.audio.voices_playing(), 8, "the ninth is refused");
    let level = s.render(0.05);
    assert!((level.last().unwrap() - 0.4).abs() < 0.01, "{level:?}");
    assert!(s.audio.bus("plugin:sfx").is_some());
    // Under `ui`: the user's interface-sound setting silences it.
    s.audio.set_ui_sounds(false).unwrap();
    let level = s.render(0.05);
    assert_eq!(*level.last().unwrap(), 0.);
}

// The mixer's effects: zgx-script's `audio-effects`.
#[cfg(feature = "audio-effects")]
#[test]
fn the_sounds_example_runs_and_plays() {
    let app = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sounds/app.luau");
    let mut s = Sounding::with(move |runtime| ScriptHost::new(runtime, app));
    s.app.frames(30);
    assert_eq!(s.app.error(), None);
    // The clicks load in their own time.
    assert!(s.app.wait_until(|app| {
        app.click("soft");
        app.host
            .audio_engine()
            .is_some_and(|a| a.voices_playing() > 0)
    }));
    assert!(s.render(0.05).iter().any(|&l| l > 0.05), "a click");
    s.render(0.1);
    // The pad streams on the music bus (its button clicks too), the bar
    // schedules four ticks.
    s.app.click("pad");
    s.app.click("bar");
    assert_eq!(s.audio.voices_playing(), 6);
    s.app.frames(10);
    s.render(0.2);
    assert_eq!(s.app.error(), None);
    // The checkbox sweeps the pad's low-pass down, eased in Rust.
    s.app.click("muffle");
    s.app.frames(40);
    s.render(0.2);
    assert_eq!(s.app.error(), None);
    s.app.click("pad");
    s.render(2.);
    s.app.frames(10);
    assert_eq!(
        s.audio.voices_playing(),
        0,
        "the pad faded out, the bar ended"
    );
}

#[test]
fn sound_plays_from_a_menu_item_and_a_checkbox() {
    let click = wav("menu", 0.05);
    let code = script(
        &format!(
            "{READY}
            local click = audio.sound(\"{{click}}\")
            local log = ui.state(\"log\", \"\")
            local checked = ui.state(\"checked\", false)
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(click) }},
                ui.text {{ id = \"log\", log }},
                ui.checkbox {{ id = \"check\", label = \"Check\", value = checked, sound = {{ sound = click, gain = 0.5 }} }},
                ui.menu {{
                    open = ui.state(\"open\", true),
                    anchor = ui.button {{ id = \"anchor\", w = 40, h = 20 }},
                    ui.menu_item {{ label = \"Rename\", id = \"rename\", sound = click, on_click = function() log:set(\"renamed\") end }},
                }},
            }}"
        ),
        &[("click", &click)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    s.app.click("rename");
    assert_eq!(s.app.text("log"), "renamed");
    assert!(loudest(&s.block()) > 0.4, "the menu item's");
    s.render(0.1);
    s.app.click("check");
    let level = loudest(&s.block());
    assert!(
        (level - 0.25).abs() < 0.01,
        "the checkbox's, at its gain: {level}"
    );
}

/// A plugin `sfx` in a directory of its own (with `init.luau` from
/// `code`), loaded beside an app with a `bar` slot.
fn with_plugin(dir: &std::path::Path, code: &str) -> Sounding {
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    Sounding::with(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", "return ui.slot(\"bar\", { id = \"bar\" })");
        host.set_plugins(vec![zgx_script::Plugin {
            name: "sfx".into(),
            entry,
            grants: vec![],
        }]);
        host
    })
}

#[test]
fn a_plugin_loads_sounds_from_its_own_directory_alone() {
    // A sound outside the plugin's directory, and a link to it inside.
    let outside = wav("secret", 0.05);
    let dir = std::env::temp_dir().join(format!("zgx-audio-confined-{}", std::process::id()));
    let plugin = dir.join("plugin");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::copy(&outside, dir.join("beside.wav")).unwrap();
    std::fs::copy(&outside, plugin.join("own.wav")).unwrap();
    let link = plugin.join("link.wav");
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let mut s = with_plugin(
        &plugin,
        &format!(
            "local said = {{}}
            local function try(call, src)
                local ok, why = pcall(call, src)
                table.insert(said, if ok then \"loaded\" else tostring(why))
            end
            try(audio.sound, \"{outside}\")
            try(audio.sound, \"../beside.wav\")
            try(audio.sound, \"link.wav\")
            try(audio.sound, \"{missing}\")
            try(audio.music, \"../beside.wav\")
            try(audio.sound, \"own.wav\")
            ui.contribute(\"bar\", ui.text {{ id = \"said\", table.concat(said, \"|\") }})",
            outside = outside.display(),
            missing = dir.join("nothing-here.wav").display(),
        ),
    );
    let said = s.app.text("said");
    let said: Vec<&str> = said.split('|').collect();
    assert_eq!(said.len(), 6, "{said:?}");
    assert_eq!(said[5], "loaded", "its own sound");
    for refused in &said[..5] {
        assert!(
            refused.contains("a plugin's sounds come from its directory or below"),
            "{said:?}"
        );
    }
    // A refusal says nothing of what's there: a file and no file read alike.
    let tail = |s: &str| s.rsplit(':').next().unwrap_or_default().to_string();
    assert_eq!(tail(said[0]), tail(said[3]), "{said:?}");
    s.render(0.01);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_binding_given_nan_lets_the_window_go_idle() {
    let tone = wav("nan", 1.);
    let code = script(
        &format!(
            "{READY}
            local tone = audio.sound(\"{{tone}}\")
            local level = ui.state(\"level\", 1)
            audio.bus(\"music\"):gain(ui.tween(level, {{ duration = 100 }}))
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(tone) }},
                ui.button {{
                    id = \"play\",
                    on_click = function()
                        audio.play(tone, {{ bus = \"music\", loop = true, gain = ui.spring(level) }})
                        audio.play(tone, {{ loop = true, gain = ui.tween(level, {{ duration = 100 }}) }})
                        audio.play(tone, {{ loop = true, gain = level }})
                    end,
                    \"Play\",
                }},
                ui.button {{ id = \"nan\", on_click = function() level:set(0 / 0) end, \"NaN\" }},
            }}"
        ),
        &[("tone", &tone)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    s.app.click("play");
    s.app.frames(5);
    s.render(0.05);
    let before = s.audio.stats().commands;
    s.app.click("nan");
    s.app.frames(30);
    assert!(!s.app.clock.wants_frame(), "the window goes idle");
    let sent = s.audio.stats().commands;
    s.app.frames(30);
    assert_eq!(s.audio.stats().commands, sent, "and sends nothing more");
    assert!(sent - before <= 3, "{} commands for a NaN", sent - before);
    let level = s.render(0.05);
    assert!(level.iter().all(|l| l.is_finite() && *l > 0.), "{level:?}");
}

#[test]
fn a_plugins_streams_count_among_its_voices() {
    let dir = std::env::temp_dir().join(format!("zgx-audio-streams-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(wav("stream", 1.), dir.join("loop.wav")).unwrap();
    let mut s = with_plugin(
        &dir,
        "local played = {}
        ui.contribute(\"bar\", ui.button {
            id = \"many\",
            on_click = function()
                for _ = 1, 12 do
                    table.insert(played, audio.music(\"loop.wav\", { loop = true, gain = 0.01 }))
                end
            end,
            \"Many\",
        })",
    );
    s.app.click("many");
    assert_eq!(s.audio.voices_playing(), 8, "streams count against the 8");
    s.render(0.01);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_plugins_sounds_count_against_its_memory() {
    let dir = std::env::temp_dir().join(format!("zgx-audio-memory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // 16 s of mono at 48 kHz decodes to 3 MB of floats, past the 2 MiB.
    std::fs::copy(wav("big", 16.), dir.join("big.wav")).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(
        &entry,
        "local big = audio.sound(\"big.wav\")
        ui.contribute(\"bar\", ui.text { id = \"error\", function() return tostring(big.error:get()) end })",
    )
    .unwrap();
    let mut s = Sounding::with(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", "return ui.slot(\"bar\", { id = \"bar\" })")
            .limits(zgx_script::Limits {
                plugin_memory: 2 << 20,
                ..Default::default()
            });
        host.set_plugins(vec![zgx_script::Plugin {
            name: "sfx".into(),
            entry,
            grants: vec![],
        }]);
        host
    });
    assert!(
        s.app.wait_until(|app| app.text("error") != "nil"),
        "the decode is refused"
    );
    assert!(
        s.app.text("error").contains("plugin's memory"),
        "{}",
        s.app.text("error")
    );
    s.render(0.01);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn sounds_no_script_holds_are_let_go_after_a_reload() {
    let one = wav("held-one", 0.05);
    let two = wav("held-two", 0.05);
    let dir = one.parent().unwrap().join("held-app");
    std::fs::create_dir_all(&dir).unwrap();
    let app = dir.join("app.luau");
    let code = |path: &PathBuf| {
        format!(
            "{READY}
            local s = audio.sound(\"{}\")
            return ui.text {{ id = \"ready\", count_ready(s) }}",
            path.display()
        )
    };
    std::fs::write(&app, code(&one)).unwrap();
    let path = app.clone();
    let mut s = Sounding::with(move |runtime| ScriptHost::new(runtime, path));
    s.ready(1);
    assert_eq!(s.app.host.sounds_held(), 1);
    std::fs::write(&app, code(&two)).unwrap();
    s.app.host.reload().unwrap();
    s.ready(1);
    s.app.host.collect_garbage();
    s.app.frames(2);
    assert_eq!(
        s.app.host.sounds_held(),
        1,
        "the first, no longer held, is let go"
    );
}

/// A WAV file of `seconds` of a sine at `hz`, at half scale.
#[cfg(feature = "audio-effects")]
fn sine(name: &str, hz: f64, seconds: f64) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("zgx-audio-script-{}-{}", std::process::id(), name));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.wav"));
    let samples: Vec<f32> = (0..(48_000. * seconds) as usize)
        .map(|i| (0.5 * (std::f64::consts::TAU * hz * i as f64 / 48_000.).sin()) as f32)
        .collect();
    std::fs::write(
        &path,
        wav::write(&samples, 1, 48_000, Encoding::Float32).unwrap(),
    )
    .unwrap();
    path
}

// The mixer's effects: zgx-script's `audio-effects`.
#[cfg(feature = "audio-effects")]
#[test]
fn a_low_pass_bound_to_a_tween_moves_with_no_script_per_frame() {
    let tone = sine("lowpass", 5_000., 2.);
    let code = script(
        &format!(
            "{READY}
            local tone = audio.sound(\"{{tone}}\")
            local cutoff = ui.state(\"cutoff\", 20000)
            local music = audio.bus(\"music\")
            if music then
                music:add_effect(audio.low_pass {{ cutoff = ui.tween(cutoff, {{ duration = 200 }}), slope = 24 }})
            end
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(tone) }},
                ui.button {{ id = \"play\", on_click = function() audio.play(tone, {{ bus = \"music\", loop = true }}) end, \"Play\" }},
                ui.button {{ id = \"muffle\", on_click = function() cutoff:set(100) end, \"Muffle\" }},
            }}"
        ),
        &[("tone", &tone)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    s.app.click("play");
    let open = s.render(0.1);
    assert!(*open.last().unwrap() > 0.45, "open: {open:?}");
    s.app.frames(5);
    let at_rest = s.audio.stats().commands;
    s.app.frames(10);
    assert_eq!(s.audio.stats().commands, at_rest, "none at rest");
    s.app.click("muffle");
    let (calls, sent) = (s.app.calls(), s.audio.stats().commands);
    let mut levels = Vec::new();
    for _ in 0..20 {
        s.app.frame();
        levels.push(*s.render(1. / 60.).last().unwrap());
    }
    let eased = s.audio.stats().commands - sent;
    assert!(
        (8..=16).contains(&eased),
        "a command a frame while it eases: {eased}"
    );
    assert_eq!(s.app.calls(), calls, "no script per frame");
    let muffled = *s.render(0.1).last().unwrap();
    assert!(
        muffled < 0.01,
        "5 kHz under a 100 Hz low-pass: {muffled} ({levels:?})"
    );
}

// The mixer's effects: zgx-script's `audio-effects`.
#[cfg(feature = "audio-effects")]
#[test]
fn an_effect_parameter_following_state_is_a_command_a_change() {
    let tone = sine("peaking", 1_000., 2.);
    let code = script(
        &format!(
            "{READY}
            local tone = audio.sound(\"{{tone}}\")
            local boost = ui.state(\"boost\", 0)
            local eq
            local world = audio.bus(\"world\")
            if world then
                eq = world:add_effect(audio.peaking {{ frequency = 1000, q = 1, gain = boost }})
            end
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(tone) }},
                ui.button {{ id = \"play\", on_click = function() audio.play(tone, {{ loop = true, gain = 0.25 }}) end, \"Play\" }},
                ui.button {{ id = \"boost\", on_click = function() boost:set(12) end, \"Boost\" }},
                ui.button {{ id = \"flat\", on_click = function() if eq then eq:fade(\"gain\", 0, 0.05) end end, \"Flat\" }},
                ui.button {{ id = \"off\", on_click = function() if eq then eq:remove() end end, \"Off\" }},
            }}"
        ),
        &[("tone", &tone)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    s.app.click("play");
    let flat = *s.render(0.1).last().unwrap();
    assert!((flat - 0.125).abs() < 0.01, "{flat}");
    let sent = s.audio.stats().commands;
    s.app.click("boost");
    assert_eq!(
        s.audio.stats().commands,
        sent + 1,
        "one command for the change"
    );
    let boosted = *s.render(0.1).last().unwrap();
    // +12 dB is four times.
    assert!(
        (boosted / flat - 3.98).abs() < 0.2,
        "{boosted} against {flat}"
    );
    s.app.click("flat");
    let back = *s.render(0.15).last().unwrap();
    assert!((back - flat).abs() < 0.01, "faded back: {back}");
    s.app.click("boost");
    s.render(0.05);
    s.app.click("off");
    let off = *s.render(0.1).last().unwrap();
    assert!((off - flat).abs() < 0.01, "taken off: {off}");
}

// The mixer's effects: zgx-script's `audio-effects`.
#[cfg(feature = "audio-effects")]
#[test]
fn a_reload_takes_off_the_effects_the_old_script_added() {
    let tone = sine("reload-effects", 5_000., 2.);
    let dir = tone.parent().unwrap().join("effects-app");
    std::fs::create_dir_all(&dir).unwrap();
    let app = dir.join("app.luau");
    let code = |effect: &str| {
        script(
            &format!(
                "{READY}
                local tone = audio.sound(\"{{tone}}\")
                local world = audio.bus(\"world\")
                {effect}
                audio.play(tone, {{ loop = true }})
                return ui.text {{ id = \"ready\", count_ready(tone) }}"
            ),
            &[("tone", &tone)],
        )
    };
    std::fs::write(
        &app,
        code("if world then world:add_effect(audio.low_pass { cutoff = 20 }) end"),
    )
    .unwrap();
    let path = app.clone();
    let mut s = Sounding::with(move |runtime| ScriptHost::new(runtime, path));
    s.ready(1);
    let muffled = *s.render(0.2).last().unwrap();
    assert!(muffled < 0.01, "{muffled}");
    std::fs::write(&app, code("")).unwrap();
    s.app.host.reload().unwrap();
    s.ready(1);
    let open = *s.render(0.2).last().unwrap();
    assert!(open > 0.45, "the old script's filter is off: {open}");
}

// The mixer's effects: zgx-script's `audio-effects`.
#[cfg(feature = "audio-effects")]
#[test]
fn a_bus_ducks_under_another() {
    let tone = wav("duck", 2.);
    let code = script(
        &format!(
            "{READY}
            local tone = audio.sound(\"{{tone}}\")
            local music, ui_bus = audio.bus(\"music\"), audio.bus(\"ui\")
            if music and ui_bus then
                music:duck(ui_bus, {{ depth = -12, attack = 0.01, release = 0.1 }})
            end
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(tone) }},
                ui.button {{ id = \"music\", on_click = function() audio.play(tone, {{ bus = \"music\", loop = true }}) end, \"Music\" }},
                ui.button {{ id = \"speak\", on_click = function() audio.play(tone, {{ bus = \"ui\", loop = true, gain = 0.05 }}) end, \"Speak\" }},
            }}"
        ),
        &[("tone", &tone)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    s.app.click("music");
    let alone = *s.render(0.2).last().unwrap();
    assert!((alone - 0.5).abs() < 0.01, "{alone}");
    s.app.click("speak");
    let ducked = *s.render(0.3).last().unwrap();
    // Music down 12 dB (0.126) under the voice's 0.025.
    assert!(
        (ducked - (0.5 * 0.251 + 0.025)).abs() < 0.02,
        "ducked to {ducked}"
    );
}

#[test]
fn a_voice_crossfades_to_another() {
    let tone = wav("crossfade", 2.);
    let code = script(
        &format!(
            "{READY}{HOLDS_A_VOICE}
            local tone = audio.sound(\"{{tone}}\")
            local a, b
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(tone) }},
                ui.button {{ id = \"start\", on_click = function()
                    a = audio.play(tone, {{ loop = true }})
                    b = audio.play(tone, {{ loop = true, gain = 0 }})
                end, \"Start\" }},
                ui.button {{ id = \"cross\", on_click = function() if a and b then a:crossfade(b, 0.1, 0.4) end end, \"Cross\" }},
            }}"
        ),
        &[("tone", &tone)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    s.app.click("start");
    let before = *s.render(0.05).last().unwrap();
    assert!((before - 0.5).abs() < 0.01, "{before}");
    s.app.click("cross");
    let mut levels = s.render(0.3);
    let after = levels.pop().unwrap();
    assert!((after - 0.2).abs() < 0.01, "the second at 0.4: {after}");
    assert_eq!(s.audio.voices_playing(), 1, "the first stopped");
    // Equal power: halfway (50 ms), cos and sin of 45°, 0.35 + 0.14,
    // where a straight fade would be at 0.25 + 0.1.
    let middle = levels[(0.05 * 48_000. / BLOCK as f64) as usize];
    assert!(middle > 0.45, "{middle}: {levels:?}");
}

// The mixer's effects: zgx-script's `audio-effects`.
#[cfg(feature = "audio-effects")]
#[test]
fn a_plugin_puts_effects_on_its_own_bus_within_its_limits() {
    let dir = std::env::temp_dir().join(format!("zgx-audio-plugin-fx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = with_plugin(
        &dir,
        "local said = {}
        local own = audio.bus()
        table.insert(said, tostring(audio.bus(\"music\") == nil))
        local function try(f)
            local ok, why = pcall(f)
            table.insert(said, if ok then \"ok\" else tostring(why))
        end
        for _ = 1, 5 do
            try(function() own:add_effect(audio.low_pass { cutoff = 8000 }) end)
        end
        try(function() own:duck(own, {}) end)
        ui.contribute(\"bar\", ui.text { id = \"said\", table.concat(said, \"|\") })",
    );
    let said = s.app.text("said");
    let said: Vec<&str> = said.split('|').collect();
    assert_eq!(said[0], "true", "no other bus");
    assert_eq!(&said[1..5], ["ok"; 4], "{said:?}");
    assert!(said[5].contains("at most 4 effects"), "{said:?}");
    assert!(said[6].contains("ducks under no other"), "{said:?}");
    s.render(0.01);
    std::fs::remove_dir_all(&dir).unwrap();
}

// The mixer's effects: zgx-script's `audio-effects`.
#[cfg(feature = "audio-effects")]
#[test]
fn a_plugins_effects_count_against_its_memory() {
    let dir = std::env::temp_dir().join(format!("zgx-audio-plugin-fx-mem-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(
        &entry,
        "local ok, why = pcall(function()
            audio.bus():add_effect(audio.delay { time = 0.5, longest = 10 })
        end)
        ui.contribute(\"bar\", ui.text { id = \"said\", if ok then \"ok\" else tostring(why) })",
    )
    .unwrap();
    let mut s = Sounding::with(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", "return ui.slot(\"bar\", { id = \"bar\" })")
            .limits(zgx_script::Limits {
                plugin_memory: 2 << 20,
                ..Default::default()
            });
        host.set_plugins(vec![zgx_script::Plugin {
            name: "sfx".into(),
            entry,
            grants: vec![],
        }]);
        host
    });
    // 10 s of stereo line is 3.8 MB.
    assert!(
        s.app.text("said").contains("plugin's memory"),
        "{}",
        s.app.text("said")
    );
    s.render(0.01);
    std::fs::remove_dir_all(&dir).unwrap();
}

// The mixer's effects: zgx-script's `audio-effects`.
#[cfg(feature = "audio-effects")]
#[test]
fn an_effect_says_which_parameters_it_has() {
    let mut app = TestApp::inline(
        "local said = ui.state(\"said\", \"\")
        return ui.column {
            ui.text { id = \"said\", said },
            ui.button { id = \"ask\", on_click = function()
                local ok, why = pcall(audio.low_pass, { cutof = 800 })
                local ok2, why2 = pcall(audio.low_pass, { slope = 18 })
                said:set(tostring(why) .. \"|\" .. tostring(why2))
            end, \"Ask\" },
        }",
    );
    app.click("ask");
    let said = app.text("said");
    assert!(
        said.contains("no parameter \"cutof\": it has cutoff, resonance"),
        "{said}"
    );
    assert!(said.contains("slope is 12 or 24"), "{said}");
}

/// Fills `audio`'s command queue until it refuses a command.
fn fill_queue(audio: &Audio) {
    let master = audio.master();
    while master
        .set_gain(1., zgx_script::zgx_audio::Ramp::NONE)
        .is_ok()
    {}
}

#[test]
fn a_reload_stops_the_old_voices_even_past_a_full_queue() {
    let tone = wav("full-stop", 2.);
    let dir = tone.parent().unwrap().join("full-stop-app");
    std::fs::create_dir_all(&dir).unwrap();
    let app = dir.join("app.luau");
    std::fs::write(
        &app,
        script(
            &format!(
                "{READY}
                local tone = audio.sound(\"{{tone}}\")
                local played = false
                return ui.column {{
                    ui.text {{ id = \"ready\", count_ready(tone) }},
                    ui.button {{ id = \"play\", on_click = function() audio.play(tone, {{ loop = true }}) end, \"Play\" }},
                }}"
            ),
            &[("tone", &tone)],
        ),
    )
    .unwrap();
    let path = app.clone();
    let config = Config {
        commands: 16,
        ..Config::default()
    };
    let mut s = Sounding::configured(config, move |runtime| ScriptHost::new(runtime, path));
    s.ready(1);
    s.app.click("play");
    assert!(*s.render(0.05).last().unwrap() > 0.4);
    fill_queue(&s.audio);
    std::fs::write(&app, "return ui.text { id = \"ready\", \"1\" }").unwrap();
    s.app.host.reload().unwrap();
    // The queue drains as the mixer runs, and the stop goes then.
    let after = s.render(0.1);
    assert_eq!(
        *after.last().unwrap(),
        0.,
        "the old voice stopped: {after:?}"
    );
    assert_eq!(s.audio.voices_playing(), 0);
}

// The mixer's effects: zgx-script's `audio-effects`.
#[cfg(feature = "audio-effects")]
#[test]
fn a_reload_takes_an_effect_off_even_past_a_full_queue() {
    let tone = sine("full-remove", 5_000., 2.);
    let dir = tone.parent().unwrap().join("full-remove-app");
    std::fs::create_dir_all(&dir).unwrap();
    let app = dir.join("app.luau");
    std::fs::write(
        &app,
        "local world = audio.bus(\"world\")
        if world then
            world:add_effect(audio.low_pass { cutoff = 20 })
        end
        return ui.text { \"filtered\" }",
    )
    .unwrap();
    let path = app.clone();
    let config = Config {
        commands: 16,
        ..Config::default()
    };
    let mut s = Sounding::configured(config, move |runtime| ScriptHost::new(runtime, path));
    // A voice of the host's own, which no reload stops.
    let samples: Vec<f32> = (0..96_000)
        .map(|i| (0.5 * (std::f64::consts::TAU * 5_000. * i as f64 / 48_000.).sin()) as f32)
        .collect();
    let sound = zgx_script::zgx_audio::Sound::from_samples(&samples, 1, 48_000).unwrap();
    let world = s.audio.bus("world").unwrap();
    s.audio
        .play(
            &sound,
            zgx_script::zgx_audio::Play::new().bus(&world).looping(true),
        )
        .unwrap();
    let muffled = *s.render(0.1).last().unwrap();
    assert!(muffled < 0.01, "{muffled}");
    fill_queue(&s.audio);
    std::fs::write(&app, "return ui.text { \"plain\" }").unwrap();
    s.app.host.reload().unwrap();
    let open = *s.render(0.2).last().unwrap();
    assert!(
        open > 0.45,
        "the filter came off once the queue had room: {open}"
    );
}

/// A scene with a looping sound at `x` metres to the camera's right, and
/// (with `zone`) a hall around the camera, its sounds' readiness as text
/// `ready`.
#[cfg(feature = "audio-spatial")]
fn scene_with_sound(tone: &std::path::Path, x: f32, zone: bool) -> String {
    let zone = if zone {
        "ui.reverb_zone { position = { 0, 0, 0 }, size = { 10, 4, 10 }, preset = \"hall\" },"
    } else {
        ""
    };
    format!(
        "{READY}
        local hum = audio.sound(\"{}\")
        return ui.column {{
            ui.text {{ id = \"ready\", count_ready(hum) }},
            ui.scene3d {{
                id = \"scene\",
                w = 200,
                h = 100,
                camera = {{ position = {{ 0, 0, 0 }}, target = {{ 0, 0, -1 }} }},
                ui.group {{
                    ui.sound3d {{ sound = hum, loop = true, position = {{ {x}, 0, 0 }}, falloff = {{ ref = 1, max = 50 }} }},
                }},
                {zone}
            }},
        }}",
        tone.display()
    )
}

// Sound in 3D scenes: zgx-script's `audio-spatial`.
#[cfg(feature = "audio-spatial")]
#[test]
fn a_sound3d_plays_from_its_object_once_its_sound_is_ready() {
    let tone = wav("sound3d", 2.);
    let mut s = Sounding::new(&scene_with_sound(&tone, 4., false));
    s.ready(1);
    s.app.frames(3);
    assert_eq!(s.app.error(), None);
    assert_eq!(s.audio.voices_playing(), 1, "its emitter's voice");
    let out = s.mixer.frames(4800);
    let (left, right) = out.chunks(2).fold((0f32, 0f32), |(l, r), f| {
        (l.max(f[0].abs()), r.max(f[1].abs()))
    });
    // Inverse falloff from 1 m: a quarter at 4 m, all of it on the right.
    assert!((right - 0.125).abs() < 0.005, "right {right}");
    assert!(left < 0.001, "left {left}");
}

#[cfg(feature = "audio-spatial")]
#[test]
fn a_reverb_zone_around_the_camera_is_heard() {
    let tone = wav("sound3d-zone", 2.);
    let mut s = Sounding::new(&scene_with_sound(&tone, 4., true));
    s.ready(1);
    s.app.frames(3);
    assert_eq!(s.app.error(), None);
    let out = s.mixer.frames(9600);
    let left = out.chunks(2).fold(0f32, |l, f| l.max(f[0].abs()));
    // The sound is all on the right; the hall around the camera rings on
    // both sides.
    assert!(left > 0.001, "the hall's reverb on the left: {left}");
}

#[cfg(feature = "audio-spatial")]
#[test]
fn a_reload_stops_a_scenes_sounds() {
    let tone = wav("sound3d-reload", 2.);
    let dir = tone.parent().unwrap().join("sound3d-app");
    std::fs::create_dir_all(&dir).unwrap();
    let app = dir.join("app.luau");
    std::fs::write(&app, scene_with_sound(&tone, 2., true)).unwrap();
    let path = app.clone();
    let mut s = Sounding::with(move |runtime| ScriptHost::new(runtime, path));
    s.ready(1);
    s.app.frames(3);
    assert_eq!(s.audio.voices_playing(), 1);
    std::fs::write(&app, "return ui.text { id = \"ready\", \"1\" }").unwrap();
    s.app.host.reload().unwrap();
    s.app.frames(3);
    s.render(0.1);
    assert_eq!(
        s.audio.voices_playing(),
        0,
        "the scene's world went, and its sound"
    );
}

#[cfg(feature = "audio-spatial")]
#[test]
fn sound3d_and_reverb_zone_say_what_they_need() {
    let mut app = TestApp::inline(
        "local said = ui.state(\"said\", \"\")
        return ui.column {
            ui.text { id = \"said\", said },
            ui.button { id = \"ask\", on_click = function()
                local _, a = pcall(ui.sound3d, { loop = true })
                local _, b = pcall(ui.reverb_zone, { size = { 1, 2, 3 }, preset = \"cave\" })
                local _, c = pcall(ui.reverb_zone, {})
                said:set(tostring(a) .. \"|\" .. tostring(b) .. \"|\" .. tostring(c))
            end, \"Ask\" },
        }",
    );
    app.click("ask");
    let said = app.text("said");
    assert!(
        said.contains("ui.sound3d needs `sound = audio.sound(…)`"),
        "{said}"
    );
    assert!(said.contains("\"room\", \"hall\", \"warehouse\""), "{said}");
    assert!(said.contains("ui.reverb_zone needs a `size`"), "{said}");
}

#[test]
fn a_wrong_argument_names_the_call_it_was_given_to() {
    let tone = wav("bad-arguments", 0.5);
    let code = script(
        &format!(
            "{READY}
            local tone = audio.sound(\"{{tone}}\")
            local said = ui.state(\"said\", \"\")
            return ui.column {{
                ui.text {{ id = \"ready\", count_ready(tone) }},
                ui.text {{ id = \"said\", said }},
                ui.button {{ id = \"ask\", on_click = function()
                    local voice = audio.play(tone, {{ gain = 0 }})
                    local bus = audio.bus(\"music\")
                    local errors = {{}}
                    for _, call in {{
                        function() voice:fade(nil, 1) end,
                        function() if bus then bus:fade(\"loud\", 1) end end,
                        function() audio.play(tone, 5) end,
                    }} do
                        local _, why = pcall(call)
                        table.insert(errors, tostring(why))
                    end
                    said:set(table.concat(errors, \"|\"))
                end, \"Ask\" }},
            }}"
        ),
        &[("tone", &tone)],
    );
    let mut s = Sounding::new(&code);
    s.ready(1);
    s.app.click("ask");
    let said = s.app.text("said");
    let said: Vec<&str> = said.split('|').collect();
    assert!(said[0].contains("voice:fade: bad arguments ("), "{said:?}");
    assert!(said[1].contains("bus:fade: bad arguments ("), "{said:?}");
    assert!(said[2].contains("audio.play: bad arguments ("), "{said:?}");
}

// The mixer's effects: zgx-script's `audio-effects`.
#[cfg(feature = "audio-effects")]
#[test]
fn a_wrong_argument_to_an_effect_names_its_call() {
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            "local said = ui.state(\"said\", \"\")
            return ui.column {
                ui.text { id = \"said\", said },
                ui.button { id = \"ask\", on_click = function()
                    local world = audio.bus(\"world\")
                    if not world then return end
                    local filter = world:add_effect(audio.low_pass { cutoff = 800 })
                    local _, a = pcall(function() filter:fade(\"cutoff\", nil, 1) end)
                    local _, b = pcall(function() world:add_effect(5) end)
                    said:set(tostring(a) .. \"|\" .. tostring(b))
                end, \"Ask\" },
            }",
        );
        let (audio, _mixer) = Audio::offline(Config::default()).unwrap();
        host.set_audio(move || audio.clone());
        host
    });
    app.click("ask");
    let said = app.text("said");
    assert!(said.contains("effect:fade: bad arguments ("), "{said}");
    assert!(said.contains("bus:add_effect: bad arguments ("), "{said}");
}
