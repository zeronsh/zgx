//! macOS: the system's default output changes while a sound plays, and the
//! sound goes on, on the new device. Silence plays (zeros: nothing is
//! heard); the default moves to another output for about a second and back.
//! The original default is put back however the run ends, short of the
//! process being killed (it prints the device to put back by hand then).
//!
//! `cargo run -p zgx-audio --example device_change`
#[cfg(target_os = "macos")]
mod mac {
    use std::{ffi::c_void, time::Duration};
    use zgx_audio::{Engine, Info};

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }
    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
        ) -> i32;
        fn AudioObjectGetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
        fn AudioObjectSetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: u32,
            data: *const c_void,
        ) -> i32;
    }
    const fn code(name: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*name)
    }
    const SYSTEM: u32 = 1;
    const GLOBAL: u32 = code(b"glob");
    const OUTPUT: u32 = code(b"outp");

    fn address(selector: &[u8; 4], scope: u32) -> Address {
        Address {
            selector: code(selector),
            scope,
            element: 0,
        }
    }
    fn default_output() -> u32 {
        let mut device = 0u32;
        let mut size = 4;
        unsafe {
            AudioObjectGetPropertyData(
                SYSTEM,
                &address(b"dOut", GLOBAL),
                0,
                std::ptr::null(),
                &mut size,
                (&raw mut device).cast(),
            )
        };
        device
    }
    fn set_default_output(device: u32) -> i32 {
        unsafe {
            AudioObjectSetPropertyData(
                SYSTEM,
                &address(b"dOut", GLOBAL),
                0,
                std::ptr::null(),
                4,
                (&raw const device).cast(),
            )
        }
    }
    fn outputs() -> Vec<u32> {
        let mut size = 0;
        let all = address(b"dev#", GLOBAL);
        unsafe { AudioObjectGetPropertyDataSize(SYSTEM, &all, 0, std::ptr::null(), &mut size) };
        let mut devices = vec![0u32; size as usize / 4];
        unsafe {
            AudioObjectGetPropertyData(
                SYSTEM,
                &all,
                0,
                std::ptr::null(),
                &mut size,
                devices.as_mut_ptr().cast(),
            )
        };
        devices.retain(|&device| {
            let mut size = 0;
            let streams = address(b"stm#", OUTPUT);
            unsafe {
                AudioObjectGetPropertyDataSize(device, &streams, 0, std::ptr::null(), &mut size)
            };
            size > 0
        });
        devices
    }
    fn name(device: u32) -> String {
        let mut bytes = [0u8; 256];
        let mut size = bytes.len() as u32;
        unsafe {
            AudioObjectGetPropertyData(
                device,
                &address(b"name", GLOBAL),
                0,
                std::ptr::null(),
                &mut size,
                bytes.as_mut_ptr().cast(),
            )
        };
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(0);
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }

    /// Puts the original default back when dropped, a panic included.
    struct Restore(u32);
    impl Drop for Restore {
        fn drop(&mut self) {
            if default_output() != self.0 {
                let status = set_default_output(self.0);
                println!("restored the default output ({}): {status}", name(self.0));
            }
        }
    }

    fn frames_over(engine: &mut Engine, wait: Duration) -> u64 {
        let before = engine.reading().frames;
        std::thread::sleep(wait);
        engine.reading().frames - before
    }

    pub fn main() -> Result<(), String> {
        let original = default_output();
        let others: Vec<u32> = outputs().into_iter().filter(|&d| d != original).collect();
        // Another output by name if there is one (a display can list its
        // speakers twice), else any other.
        let other = others
            .iter()
            .find(|&&device| name(device) != name(original))
            .or(others.first());
        let Some(&other) = other else {
            println!("only one output: nothing to switch to");
            return Ok(());
        };
        println!(
            "default output {original} ({}); switching to {other} ({}). If this is killed, put {original} back.",
            name(original),
            name(other)
        );
        let mut engine = Engine::open()?;
        // Silence filled on the audio thread: a stream keeps it playing.
        engine.stream(|_: &mut [f32], _: Info| {});
        let rate = engine.rate();
        std::thread::sleep(Duration::from_millis(300));
        let first = engine.device();
        let before = frames_over(&mut engine, Duration::from_millis(400));
        let restore = Restore(original);
        let status = set_default_output(other);
        if status != 0 {
            return Err(format!("couldn't switch the default output: {status}"));
        }
        std::thread::sleep(Duration::from_millis(300));
        let moved = engine.device();
        let during = frames_over(&mut engine, Duration::from_millis(400));
        drop(restore);
        std::thread::sleep(Duration::from_millis(300));
        let back = engine.device();
        let after = frames_over(&mut engine, Duration::from_millis(400));
        println!(
            "unit's device: {first:?} before, {moved:?} after the switch, {back:?} after switching back"
        );
        println!(
            "frames pulled in 0.4 s at {rate} Hz: {before} before, {during} on the new device, {after} back"
        );
        let expected = u64::from(rate) * 4 / 10;
        let plays = |frames: u64| frames > expected * 8 / 10;
        if moved == Some(other) && back == Some(original) && plays(during) && plays(after) {
            println!("device change: the sound followed the default output and back");
            Ok(())
        } else {
            Err("the sound didn't follow the default output".into())
        }
    }
}

fn main() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return mac::main();
    #[cfg(not(target_os = "macos"))]
    {
        println!("device_change runs on macOS");
        Ok(())
    }
}
