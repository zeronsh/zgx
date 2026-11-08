//! Output on Windows: WASAPI in shared mode, event-driven. The mix runs on a
//! thread of ours in MMCSS's "Pro Audio" class, woken by the audio engine
//! each time its buffer has room. After the mixer's silence the client is
//! stopped and released (the device is closed) and the thread parks until a
//! command wakes it; it opens the default device again then, so a change of
//! device while closed is followed.
//!
//! Everything COM is made and used on that thread (in the multithreaded
//! apartment), never the app's, whose apartment is its own business.
use super::Gate;
use crate::mixer::{Mixer, Watch};
use std::{
    sync::{Arc, mpsc},
    thread::{JoinHandle, Thread},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        Media::Audio::{
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            IAudioClient, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
            WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0, eConsole, eRender,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
                CoUninitialize,
            },
            Threading::{
                AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
                WaitForSingleObject,
            },
        },
    },
    core::{GUID, w},
};

/// `WAVE_FORMAT_EXTENSIBLE`.
const EXTENSIBLE: u16 = 0xFFFE;
/// `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT`.
const IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
/// Front left and right.
const STEREO: u32 = 0x1 | 0x2;

/// A client playing, its event and its buffer's size in frames.
struct Open {
    client: IAudioClient,
    render: IAudioRenderClient,
    event: HANDLE,
    frames: u32,
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

/// The default output's rate (its mixer's), without opening a stream.
fn mix_rate() -> windows::core::Result<u32> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let mix = client.GetMixFormat()?;
        let rate = (*mix).nSamplesPerSec;
        CoTaskMemFree(Some(mix.cast()));
        Ok(rate)
    }
}

/// Opens the default output for float stereo at `rate` (the audio engine
/// converts, should its mix run at another), started.
fn open(rate: u32) -> windows::core::Result<Open> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let format = WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: EXTENSIBLE,
                nChannels: 2,
                nSamplesPerSec: rate,
                nAvgBytesPerSec: rate * 8,
                nBlockAlign: 8,
                wBitsPerSample: 32,
                cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
            },
            Samples: WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: 32,
            },
            dwChannelMask: STEREO,
            SubFormat: IEEE_FLOAT,
        };
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            // The engine's own period: event mode takes the smallest it may.
            0,
            0,
            (&raw const format).cast(),
            None,
        )?;
        let event = CreateEventW(None, false, false, None)?;
        let opened = (|| {
            client.SetEventHandle(event)?;
            let frames = client.GetBufferSize()?;
            let render: IAudioRenderClient = client.GetService()?;
            client.Start()?;
            Ok::<_, windows::core::Error>((frames, render))
        })();
        match opened {
            Ok((frames, render)) => Ok(Open {
                client,
                render,
                event,
                frames,
            }),
            Err(error) => {
                let _ = CloseHandle(event);
                Err(error)
            }
        }
    }
}

/// The device, open and silent until [`Device::start`].
pub struct Device {
    rate: u32,
    thread: JoinHandle<()>,
    gate: Arc<Gate>,
    start: mpsc::Sender<(Mixer, Watch)>,
}

/// Output running: closes and ends its thread when dropped.
pub struct Output {
    thread: Option<JoinHandle<()>>,
    handle: Thread,
    gate: Arc<Gate>,
}

impl Device {
    pub fn open() -> Result<Self, String> {
        let gate = Arc::new(Gate::default());
        let (opened, is_open) = mpsc::channel::<Result<u32, String>>();
        let (start, started) = mpsc::channel::<(Mixer, Watch)>();
        let running = gate.clone();
        let thread = std::thread::Builder::new()
            .name("zgx-audio".into())
            .spawn(move || unsafe {
                if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                    let _ = opened.send(Err("COM wouldn't start".into()));
                    return;
                }
                let first = mix_rate().and_then(|rate| open(rate).map(|open| (rate, open)));
                match first {
                    Ok((rate, first)) => {
                        let _ = opened.send(Ok(rate));
                        if let Ok((mixer, watch)) = started.recv() {
                            play(rate, first, mixer, &watch, &running);
                        }
                    }
                    Err(error) => {
                        let _ = opened.send(Err(format!("opening the output: {error}")));
                    }
                }
                CoUninitialize();
            })
            .map_err(|e| e.to_string())?;
        let rate = is_open
            .recv()
            .map_err(|_| "the audio thread ended".to_string())??;
        Ok(Self {
            rate,
            thread,
            gate,
            start,
        })
    }
    pub fn rate(&self) -> u32 {
        self.rate
    }
    pub fn start(self, mixer: Mixer, watch: Watch) -> Result<Output, String> {
        self.start
            .send((mixer, watch))
            .map_err(|_| "the audio thread ended".to_string())?;
        Ok(Output {
            handle: self.thread.thread().clone(),
            thread: Some(self.thread),
            gate: self.gate,
        })
    }
}

/// The audio thread: fills the buffer as the engine signals room, closes
/// after silence (or the device's loss), opens again when woken.
fn play(rate: u32, first: Open, mut mixer: Mixer, watch: &Watch, gate: &Gate) {
    let mut task = 0;
    let class = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task) };
    let mut current = Some(first);
    loop {
        let Some(device) = &current else {
            if !gate.wait(watch) {
                break;
            }
            match open(rate) {
                Ok(device) => current = Some(device),
                Err(error) => {
                    eprintln!("zgx-audio: opening the output: {error}");
                    // What waited is let go of, with no device to play it
                    // on; closed again until the next sound.
                    mixer.render(&mut []);
                    gate.begin_close();
                    gate.closed(watch);
                }
            }
            continue;
        };
        if gate.quitting() {
            break;
        }
        // A tenth of a second at most: a quit is seen even if the engine
        // stops signalling.
        let woke = unsafe { WaitForSingleObject(device.event, 100) } == WAIT_OBJECT_0;
        let filled = if !woke {
            Ok(())
        } else {
            unsafe {
                (|| {
                    let free = device.frames - device.client.GetCurrentPadding()?;
                    if free > 0 {
                        let data = device.render.GetBuffer(free)?;
                        let out =
                            std::slice::from_raw_parts_mut(data.cast::<f32>(), free as usize * 2);
                        mixer.render(out);
                        device.render.ReleaseBuffer(free, 0)?;
                    }
                    Ok::<_, windows::core::Error>(())
                })()
            }
        };
        // An error here is the device lost (unplugged, or the default
        // changed while playing): closed, to open the new default.
        if (filled.is_err() || watch.idle()) && gate.begin_close() {
            current = None;
            gate.closed(watch);
        }
    }
    drop(current);
    if let Ok(class) = class {
        unsafe {
            let _ = AvRevertMmThreadCharacteristics(class);
        }
    }
}

impl Output {
    /// Opens it again if it closed: a command is waiting.
    pub fn wake(&self) {
        if self.gate.reopen() {
            self.handle.unpark();
        }
    }
    pub fn is_running(&self) -> bool {
        self.gate.is_running()
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.gate.quit();
        self.handle.unpark();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
