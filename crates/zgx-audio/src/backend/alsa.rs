//! Output on Linux: ALSA's API, which plays on PipeWire (through its ALSA
//! plugin), PulseAudio and plain ALSA alike. libasound is loaded with
//! `dlopen` when a sound first plays: the binary doesn't link it, so an app
//! starts where it's missing, and one that plays nothing never loads it.
//!
//! The mix runs on a thread of ours that writes a period at a time, the
//! write blocking until the device has room. After the mixer's silence the
//! device is closed and the thread parks until a command wakes it.
use super::Gate;
use crate::mixer::{Mixer, Watch};
use std::{
    ffi::{CStr, c_char, c_int, c_long, c_uint, c_ulong, c_void},
    sync::{Arc, OnceLock, mpsc},
    thread::{JoinHandle, Thread},
};

type Pcm = *mut c_void;

#[cfg_attr(target_env = "gnu", link(name = "dl"))]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
const RTLD_NOW: c_int = 2;

type OpenFn = unsafe extern "C" fn(*mut Pcm, *const c_char, c_int, c_int) -> c_int;
type SetParamsFn = unsafe extern "C" fn(Pcm, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type GetParamsFn = unsafe extern "C" fn(Pcm, *mut c_ulong, *mut c_ulong) -> c_int;
type WriteiFn = unsafe extern "C" fn(Pcm, *const c_void, c_ulong) -> c_long;
type RecoverFn = unsafe extern "C" fn(Pcm, c_int, c_int) -> c_int;
type PcmFn = unsafe extern "C" fn(Pcm) -> c_int;
type StrerrorFn = unsafe extern "C" fn(c_int) -> *const c_char;

/// libasound's functions the output calls.
struct Alsa {
    open: OpenFn,
    set_params: SetParamsFn,
    get_params: GetParamsFn,
    writei: WriteiFn,
    recover: RecoverFn,
    drain: PcmFn,
    close: PcmFn,
    strerror: StrerrorFn,
}

fn alsa() -> Result<&'static Alsa, String> {
    static LOADED: OnceLock<Result<Alsa, String>> = OnceLock::new();
    LOADED
        .get_or_init(|| unsafe {
            let lib = dlopen(c"libasound.so.2".as_ptr(), RTLD_NOW);
            if lib.is_null() {
                return Err("libasound.so.2 isn't installed".into());
            }
            macro_rules! get {
                ($name:literal, $kind:ty) => {{
                    let symbol = dlsym(lib, $name.as_ptr());
                    if symbol.is_null() {
                        return Err(format!("libasound has no {:?}", $name));
                    }
                    std::mem::transmute::<*mut c_void, $kind>(symbol)
                }};
            }
            Ok(Alsa {
                open: get!(c"snd_pcm_open", OpenFn),
                set_params: get!(c"snd_pcm_set_params", SetParamsFn),
                get_params: get!(c"snd_pcm_get_params", GetParamsFn),
                writei: get!(c"snd_pcm_writei", WriteiFn),
                recover: get!(c"snd_pcm_recover", RecoverFn),
                drain: get!(c"snd_pcm_drain", PcmFn),
                close: get!(c"snd_pcm_close", PcmFn),
                strerror: get!(c"snd_strerror", StrerrorFn),
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

const PLAYBACK: c_int = 0;
const FLOAT_LE: c_int = 14;
const RW_INTERLEAVED: c_int = 3;
/// The mix's rate: the server (PipeWire, PulseAudio) or ALSA's plug layer
/// converts if the hardware runs at another.
const RATE: u32 = 48_000;
/// The latency asked for, in microseconds.
const LATENCY_US: c_uint = 20_000;

/// An open PCM and its period, in frames.
struct Open {
    pcm: Pcm,
    period: usize,
}

fn error(alsa: &Alsa, code: c_int) -> String {
    unsafe { CStr::from_ptr((alsa.strerror)(code)) }
        .to_string_lossy()
        .into_owned()
}

fn open_pcm(alsa: &Alsa) -> Result<Open, String> {
    let name = std::env::var("ZGX_AUDIO_DEVICE").unwrap_or_else(|_| "default".into());
    let name = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
    let mut pcm: Pcm = std::ptr::null_mut();
    unsafe {
        let code = (alsa.open)(&mut pcm, name.as_ptr(), PLAYBACK, 0);
        if code < 0 {
            return Err(format!("opening ALSA's output: {}", error(alsa, code)));
        }
        let code = (alsa.set_params)(pcm, FLOAT_LE, RW_INTERLEAVED, 2, RATE, 1, LATENCY_US);
        if code < 0 {
            (alsa.close)(pcm);
            return Err(format!("setting float stereo: {}", error(alsa, code)));
        }
        let (mut buffer, mut period) = (0, 0);
        if (alsa.get_params)(pcm, &mut buffer, &mut period) < 0 || period == 0 {
            period = 480;
        }
        Ok(Open {
            pcm,
            period: period as usize,
        })
    }
}

/// The device, open and silent until [`Device::start`].
pub struct Device {
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

struct SendPcm(Open);
// The PCM moves to the audio thread once and is used only there.
unsafe impl Send for SendPcm {}

impl Device {
    pub fn open() -> Result<Self, String> {
        let alsa = alsa()?;
        let first = SendPcm(open_pcm(alsa)?);
        let gate = Arc::new(Gate::default());
        let (start, started) = mpsc::channel::<(Mixer, Watch)>();
        let running = gate.clone();
        let thread = std::thread::Builder::new()
            .name("zgx-audio".into())
            .spawn(move || {
                let first = first;
                let Ok((mixer, watch)) = started.recv() else {
                    unsafe { (alsa.close)(first.0.pcm) };
                    return;
                };
                play(alsa, first.0, mixer, &watch, &running);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            thread,
            gate,
            start,
        })
    }
    pub fn rate(&self) -> u32 {
        RATE
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

/// The audio thread: plays while sounds come, closes after silence, opens
/// again when woken.
fn play(alsa: &Alsa, first: Open, mut mixer: Mixer, watch: &Watch, gate: &Gate) {
    let mut open = Some(first);
    // Made before the first buffer: the loop allocates nothing.
    let mut buffer = vec![0f32; open.as_ref().map_or(480, |o| o.period) * 2];
    loop {
        let Some(device) = &open else {
            if !gate.wait(watch) {
                return;
            }
            match open_pcm(alsa) {
                Ok(device) => {
                    if device.period * 2 > buffer.len() {
                        buffer.resize(device.period * 2, 0.);
                    }
                    open = Some(device);
                }
                Err(error) => {
                    eprintln!("zgx-audio: {error}");
                    // What waited is let go of (no device to play it on), and
                    // it's closed again: the next sound tries again.
                    mixer.render(&mut buffer);
                    gate.begin_close();
                    gate.closed(watch);
                }
            }
            continue;
        };
        if gate.quitting() {
            unsafe { (alsa.close)(device.pcm) };
            return;
        }
        let out = &mut buffer[..device.period * 2];
        mixer.render(out);
        let mut done = 0;
        let mut failed = false;
        while done < device.period {
            let written = unsafe {
                (alsa.writei)(
                    device.pcm,
                    out[done * 2..].as_ptr().cast(),
                    (device.period - done) as c_ulong,
                )
            };
            if written < 0 {
                if unsafe { (alsa.recover)(device.pcm, written as c_int, 1) } < 0 {
                    failed = true;
                    break;
                }
                continue;
            }
            done += written as usize;
        }
        if (failed || watch.idle()) && gate.begin_close() {
            unsafe {
                if !failed {
                    (alsa.drain)(device.pcm);
                }
                (alsa.close)(device.pcm);
            }
            open = None;
            gate.closed(watch);
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
