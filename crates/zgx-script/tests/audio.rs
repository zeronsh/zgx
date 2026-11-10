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
        let (audio, mixer) = Audio::offline(Config::default()).unwrap();
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
                ui.text {{ id = \"reach\", tostring(audio.bus == nil and audio.prewarm == nil and audio.ui_sounds_enabled == nil) }},
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
