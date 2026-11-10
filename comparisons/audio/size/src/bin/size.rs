//! A binary whose size is measured three ways: std only; with zgx-audio's
//! offline mixer playing a sound; with its device as well.
fn main() {
    let samples: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin()).collect();
    #[cfg(not(feature = "zgx-audio"))]
    println!("{}", samples.iter().sum::<f32>());
    #[cfg(feature = "vorbis")]
    {
        let (audio, mut mixer) = zgx_audio::Audio::offline(zgx_audio::Config::default());
        let tone = include_bytes!("../../../../../crates/zgx-audio/tests/assets/tone.ogg");
        let sound = audio.load(zgx_audio::Source::bytes(&tone[..])).unwrap();
        audio.play(&sound, zgx_audio::Play::new()).unwrap();
        audio.stream(zgx_audio::Source::bytes(&tone[..]), zgx_audio::Play::new()).unwrap();
        println!("{}", mixer.frames(480).iter().sum::<f32>() + samples[0]);
    }
    #[cfg(all(feature = "zgx-audio", not(feature = "device"), not(feature = "vorbis")))]
    {
        let (audio, mut mixer) = zgx_audio::Audio::offline(zgx_audio::Config::default());
        let sound = zgx_audio::Sound::from_samples(&samples, 1, 48_000).unwrap();
        audio.play(&sound, zgx_audio::Play::new()).unwrap();
        // A stream too, so the vorbis arm differs by the decoder alone.
        let wav = zgx_audio::wav::write(&samples, 1, 48_000, zgx_audio::wav::Encoding::Float32);
        audio.stream(zgx_audio::Source::bytes(wav), zgx_audio::Play::new()).unwrap();
        println!("{}", mixer.frames(480).iter().sum::<f32>());
    }
    #[cfg(feature = "device")]
    {
        let audio = zgx_audio::Audio::new(zgx_audio::Config { gain: 0., ..Default::default() });
        let sound = zgx_audio::Sound::from_samples(&samples, 1, 48_000).unwrap();
        if std::env::args().count() > 5 {
            audio.play(&sound, zgx_audio::Play::new()).unwrap();
        }
        println!("{}", audio.stats().frames);
    }
}
