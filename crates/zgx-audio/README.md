# zgx-audio

Sound for [zgx](../zgx) apps, in a crate of its own: an app that doesn't depend on it pays nothing.

```rust
use zgx_audio::{Audio, Config, Curve, Play, Ramp, Source};

let (audio, mut mixer) = Audio::offline(Config::default());
let click = audio.load(Source::file("click.wav"))?;
let ui = audio.bus("ui").unwrap();
let voice = audio.play(&click, Play::new().bus(&ui).gain(0.8))?;
audio.play(&click, Play::new().at(audio.now() + audio.seconds(0.25)))?; // on that frame exactly
voice.stop_after(0.05)?;                                                  // a fade, never a cut
audio.bus("music").unwrap().fade_to(0., 2., Curve::EqualPower)?;
let samples = mixer.frames(48_000);                                       // a second, interleaved stereo
```

What there is so far:

- **A mixer that places every command on a frame:** plays, stops and parameter changes carry a time on the mixer's clock and land on that frame, whatever the device's block size. Tests render each scene in blocks of 1, 37, 128, 300 and 512 frames and require the same samples.
- **Voices** from a fixed pool (64 by default), with gain, pan and pitch, each moved by a ramp a sample at a time. Starts away from zero fade in over 1 ms and stops fade out, so nothing clicks. Loops can have an intro before their loop points. Past the limit, the quietest voice of the lowest priority is stolen with a 5 ms fade.
- **Buses** in a tree under `master` (`ui`, `music` and `world` to start), with gain, mute, solo and fades along linear, equal-power or exponential curves.
- **A look-ahead limiter** on master (1.5 ms): nothing leaves above its ceiling, and what's below passes unchanged.
- **WAV files** (PCM 8, 16, 24 and 32 bits, float 32 and 64), kept at their own rate and resampled per voice: a four-point cubic by default, or linear (`Config::interpolation`).
- **Streams** (`Audio::stream`) for music and anything long: decoded as they play, about half a second ahead, on one decode thread that sleeps when no stream is open. A stream sounds exactly as the same source loaded whole, loops without a seam, and when its decoding falls behind it fades out over 5 ms and back in rather than clicking (`Stats::dry` counts it).
- **Ogg Vorbis** with the `vorbis` feature (lewton, pure Rust, MIT/Apache), loaded whole or streamed.
- **An offline renderer** on a clock of its own: what the tests compare, at -90 dB, with references in `tests/references`.

The mixer never locks, allocates, frees or makes a system call; `tests/realtime.rs` checks it, with a counting allocator and the kernel's count of system calls, over every path a voice or a bus can take. Commands reach it through a fixed queue, and a full queue returns an error rather than waiting. Sounds it has finished with go back to the main thread to be dropped. With the `device` feature, `Audio::new` plays to the system's default output through cpal (CoreAudio, WASAPI shared mode, ALSA). Nothing is opened until the first sound. The stream then starts, and it stops after `Config::hold` (1 s) with nothing to hear: fades, the limiter's tail and sounds due later all count. While stopped, the host thread sleeps until the next command, so an engine with nothing playing does no work. `Audio::prewarm()` starts the stream early. A stream that's lost (a device gone, a format changed) starts again on the default output, keeping every voice. `Device` is public, so another backend goes in `Audio::with_device`.

What a voice costs: `cargo run --release -p zgx-audio --example mix_cost -- 64 44100 2` counts the instructions the mixer retires for each voice and frame.
