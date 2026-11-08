//! A script plays a click through zgx-audio: `cargo run -p zgx-audio
//! --features script --example click`. Headless: no window, only the
//! script's host. Then a synth's stream plays a short tone, filled on the
//! audio thread.
use std::time::Duration;
use zgx_audio::{Engine, Info};

fn main() -> Result<(), String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/click");
    // The click: 30 ms of a falling 2 kHz tone, made here so the repository
    // holds no sound file.
    let click = dir.join("click.wav");
    if !click.exists() {
        write_click(&click).map_err(|error| error.to_string())?;
    }
    let ui = zgx::widgets::Ui::new(1., 1.);
    let host = zgx_script::ScriptHost::new(&ui.runtime, dir.join("app.luau"));
    let (host, audio) = zgx_audio::script::install(host, &dir)?;
    host.reload()?;
    std::thread::sleep(Duration::from_millis(700));
    let reading = audio.with(Engine::reading).ok_or("no output")?;
    println!(
        "script: {} clicks sounded, peak {:.2}, {} frames at {} Hz",
        reading.started,
        reading.peak,
        reading.frames,
        audio.with(|engine| engine.rate()).unwrap_or(0)
    );

    // A synth: a sine filled on the audio thread, its envelope from the
    // frame count alone (no allocation, no lock, no script).
    let mut engine = Engine::open()?;
    let rate = engine.rate() as f32;
    let mut phase = 0f32;
    engine.stream(move |out: &mut [f32], info: Info| {
        for (i, frame) in out.chunks_exact_mut(2).enumerate() {
            let t = (info.frame + i as u64) as f32 / rate;
            let envelope = (t * 40.).min(1.) * (1. - t / 0.4).max(0.);
            phase = (phase + 440. / rate).fract();
            let sample = (phase * std::f32::consts::TAU).sin() * 0.2 * envelope;
            frame.fill(sample);
        }
    });
    std::thread::sleep(Duration::from_millis(500));
    let reading = engine.reading();
    println!(
        "stream: peak {:.2}, {} frames",
        reading.peak, reading.frames
    );
    Ok(())
}

fn write_click(path: &std::path::Path) -> std::io::Result<()> {
    let rate = 48_000u32;
    let samples: Vec<i16> = (0..rate * 3 / 100)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let s = (t * 2000. * std::f32::consts::TAU).sin() * (-t * 160.).exp();
            (s * 32767.) as i16
        })
        .collect();
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 2).to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    std::fs::write(path, bytes)
}
