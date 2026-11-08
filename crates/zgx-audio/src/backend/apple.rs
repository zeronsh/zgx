//! Output on macOS and iOS: the system's output audio unit (the default
//! device's on macOS, Remote I/O on iOS), pulling float stereo from the mixer
//! on its own real-time thread. AudioToolbox is called directly: it's a few
//! functions, and every Mac and iPhone has it.
use crate::mixer::Mixer;
use std::ffi::c_void;

type OsStatus = i32;
type AudioUnit = *mut c_void;

#[repr(C)]
struct ComponentDescription {
    kind: u32,
    sub_kind: u32,
    manufacturer: u32,
    flags: u32,
    flags_mask: u32,
}
#[repr(C)]
#[derive(Default)]
struct StreamDescription {
    sample_rate: f64,
    format: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}
#[repr(C)]
struct Buffer {
    channels: u32,
    bytes: u32,
    data: *mut c_void,
}
#[repr(C)]
struct BufferList {
    count: u32,
    buffers: [Buffer; 1],
}
type RenderProc = unsafe extern "C" fn(
    *mut c_void,
    *mut u32,
    *const c_void,
    u32,
    u32,
    *mut BufferList,
) -> OsStatus;
#[repr(C)]
struct RenderCallback {
    proc_: RenderProc,
    context: *mut c_void,
}

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioComponentFindNext(after: *mut c_void, desc: *const ComponentDescription)
    -> *mut c_void;
    fn AudioComponentInstanceNew(component: *mut c_void, out: *mut AudioUnit) -> OsStatus;
    fn AudioComponentInstanceDispose(unit: AudioUnit) -> OsStatus;
    fn AudioUnitSetProperty(
        unit: AudioUnit,
        id: u32,
        scope: u32,
        element: u32,
        data: *const c_void,
        size: u32,
    ) -> OsStatus;
    fn AudioUnitGetProperty(
        unit: AudioUnit,
        id: u32,
        scope: u32,
        element: u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> OsStatus;
    fn AudioUnitInitialize(unit: AudioUnit) -> OsStatus;
    fn AudioUnitUninitialize(unit: AudioUnit) -> OsStatus;
    fn AudioOutputUnitStart(unit: AudioUnit) -> OsStatus;
    fn AudioOutputUnitStop(unit: AudioUnit) -> OsStatus;
}

const fn code(name: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*name)
}
const OUTPUT: u32 = code(b"auou");
#[cfg(target_os = "macos")]
const SUB_KIND: u32 = code(b"def ");
#[cfg(not(target_os = "macos"))]
const SUB_KIND: u32 = code(b"rioc");
const APPLE: u32 = code(b"appl");
const LINEAR_PCM: u32 = code(b"lpcm");
const FLOAT_PACKED: u32 = 1 | 8;
const STREAM_FORMAT: u32 = 8;
const RENDER_CALLBACK: u32 = 23;
const SCOPE_INPUT: u32 = 1;
const SCOPE_OUTPUT: u32 = 2;

fn check(status: OsStatus, doing: &str) -> Result<(), String> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("{doing}: OSStatus {status}"))
    }
}

/// The device, open and silent until [`Device::start`].
pub struct Device {
    unit: AudioUnit,
    rate: u32,
}

/// Output running: stops and closes when dropped.
pub struct Output {
    unit: AudioUnit,
    mixer: *mut Mixer,
}

impl Device {
    pub fn open() -> Result<Self, String> {
        let desc = ComponentDescription {
            kind: OUTPUT,
            sub_kind: SUB_KIND,
            manufacturer: APPLE,
            flags: 0,
            flags_mask: 0,
        };
        unsafe {
            let component = AudioComponentFindNext(std::ptr::null_mut(), &desc);
            if component.is_null() {
                return Err("no output audio unit".into());
            }
            let mut unit: AudioUnit = std::ptr::null_mut();
            check(
                AudioComponentInstanceNew(component, &mut unit),
                "opening the output",
            )?;
            // The hardware's rate: the mix is made at it, so nothing resamples
            // on the audio thread.
            let mut hardware = StreamDescription::default();
            let mut size = size_of::<StreamDescription>() as u32;
            let rate = if AudioUnitGetProperty(
                unit,
                STREAM_FORMAT,
                SCOPE_OUTPUT,
                0,
                (&raw mut hardware).cast(),
                &mut size,
            ) == 0
                && hardware.sample_rate >= 8000.
            {
                hardware.sample_rate.round() as u32
            } else {
                48_000
            };
            let format = StreamDescription {
                sample_rate: f64::from(rate),
                format: LINEAR_PCM,
                format_flags: FLOAT_PACKED,
                bytes_per_packet: 8,
                frames_per_packet: 1,
                bytes_per_frame: 8,
                channels: 2,
                bits: 32,
                reserved: 0,
            };
            let set = AudioUnitSetProperty(
                unit,
                STREAM_FORMAT,
                SCOPE_INPUT,
                0,
                (&raw const format).cast(),
                size_of::<StreamDescription>() as u32,
            );
            if let Err(error) = check(set, "setting float stereo") {
                AudioComponentInstanceDispose(unit);
                return Err(error);
            }
            Ok(Self { unit, rate })
        }
    }
    pub fn rate(&self) -> u32 {
        self.rate
    }
    pub fn start(self, mixer: Mixer) -> Result<Output, String> {
        let unit = self.unit;
        std::mem::forget(self);
        let mixer = Box::into_raw(Box::new(mixer));
        let output = Output { unit, mixer };
        let callback = RenderCallback {
            proc_: render,
            context: mixer.cast(),
        };
        unsafe {
            check(
                AudioUnitSetProperty(
                    unit,
                    RENDER_CALLBACK,
                    SCOPE_INPUT,
                    0,
                    (&raw const callback).cast(),
                    size_of::<RenderCallback>() as u32,
                ),
                "setting the render callback",
            )?;
            check(AudioUnitInitialize(unit), "starting the output")?;
            check(AudioOutputUnitStart(unit), "starting the output")?;
        }
        Ok(output)
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        unsafe { AudioComponentInstanceDispose(self.unit) };
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        unsafe {
            // Stopped returns once the callback has finished: the mixer is
            // ours again.
            AudioOutputUnitStop(self.unit);
            AudioUnitUninitialize(self.unit);
            AudioComponentInstanceDispose(self.unit);
            drop(Box::from_raw(self.mixer));
        }
    }
}

/// Called on the audio unit's real-time thread for each buffer.
unsafe extern "C" fn render(
    context: *mut c_void,
    _flags: *mut u32,
    _time: *const c_void,
    _bus: u32,
    frames: u32,
    data: *mut BufferList,
) -> OsStatus {
    unsafe {
        let mixer = &mut *context.cast::<Mixer>();
        let list = &mut *data;
        if list.count == 0 {
            return 0;
        }
        let buffer = &mut list.buffers[0];
        let samples = (buffer.bytes as usize / 4).min(frames as usize * 2);
        let out = std::slice::from_raw_parts_mut(buffer.data.cast::<f32>(), samples);
        mixer.render(out);
    }
    0
}
