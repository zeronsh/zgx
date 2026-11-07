//! The mix, sample by sample.
use std::path::PathBuf;
use std::rc::Rc;
use zgx_video::AudioClip;
use zgx_video::audio::{SAMPLE_RATE, Sound, decode, mix, write_wav};

fn clip(from: u64) -> AudioClip {
    AudioClip {
        src: PathBuf::from("a"),
        from,
        duration: None,
        trim: 0,
        volume: 1.,
        fade_in: 0,
        fade_out: 0,
        looped: false,
        optional: false,
        volumes: None,
    }
}
fn constant(seconds: f64, value: f32) -> Rc<Sound> {
    Rc::new(Sound(vec![
        value;
        (seconds * f64::from(SAMPLE_RATE)) as usize * 2
    ]))
}

#[test]
fn a_clip_starts_at_its_frame_and_ends_with_itself() {
    let mixed = mix(&[clip(30)], 30., 90, |_| Some(constant(1., 0.5)));
    let rate = SAMPLE_RATE as usize;
    assert_eq!(mixed.len(), rate * 3 * 2);
    assert_eq!(mixed[rate * 2 - 2], 0., "before its frame");
    assert_eq!(mixed[rate * 2], 0.5, "from it");
    assert_eq!(mixed[rate * 4 - 2], 0.5, "a second of it");
    assert_eq!(mixed[rate * 4], 0., "and no more");
}

#[test]
fn volume_fades_loops_and_trims() {
    let rate = SAMPLE_RATE as usize;
    let mut quiet = clip(0);
    quiet.volume = 0.5;
    assert_eq!(
        mix(&[quiet], 30., 30, |_| Some(constant(1., 0.5)))[100],
        0.25
    );
    let mut faded = clip(0);
    faded.fade_in = 30;
    let mixed = mix(&[faded], 30., 60, |_| Some(constant(2., 0.8)));
    assert!(
        (mixed[rate] - 0.4).abs() < 0.001,
        "half way in: {}",
        mixed[rate]
    );
    let mut looped = clip(0);
    looped.looped = true;
    let mixed = mix(&[looped], 30., 90, |_| Some(constant(1., 0.3)));
    assert_eq!(mixed[rate * 5], 0.3, "still going in its third second");
    let mut trimmed = clip(0);
    trimmed.trim = 15;
    let mixed = mix(&[trimmed], 30., 30, |_| Some(constant(1., 0.3)));
    assert_eq!(mixed[rate - 2], 0.3);
    assert_eq!(mixed[rate + 2], 0., "half a second skipped, half left");
}

#[test]
fn a_mix_too_loud_comes_down_whole() {
    let two = [clip(0), clip(0)];
    let mixed = mix(&two, 30., 30, |_| Some(constant(1., 0.8)));
    assert!((mixed[0] - 1.).abs() < 1e-6);
}

#[test]
fn a_wav_reads_back_as_it_was_written_and_comes_to_the_mix_rate() {
    let path = std::env::temp_dir().join(format!("zgx-video-wav-{}.wav", std::process::id()));
    let tone: Vec<f32> = (0..1000)
        .flat_map(|i| [i as f32 / 2000., -(i as f32) / 2000.])
        .collect();
    write_wav(&path, &tone).unwrap();
    let sound = decode(&path).unwrap();
    assert_eq!(sound.0.len(), tone.len());
    assert!((sound.0[998] - tone[998]).abs() < 1e-4);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_volume_of_the_frame_shapes_the_clip_between_frames() {
    let mut ramp = clip(0);
    ramp.volumes = Some(vec![0., 1., 1., 0.5]);
    let rate = SAMPLE_RATE as usize;
    let mixed = mix(&[ramp], 30., 30, |_| Some(constant(1., 0.8)));
    let at = |frame: f64| mixed[((frame / 30. * rate as f64) as usize) * 2];
    assert!(at(0.) < 0.01, "silent at its start");
    assert!((at(0.5) - 0.4).abs() < 0.01, "half way up: {}", at(0.5));
    assert!((at(2.) - 0.8).abs() < 0.01);
    assert!((at(10.) - 0.4).abs() < 0.01, "its last held: {}", at(10.));
}
