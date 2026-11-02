//! Serial mode: pin the process to one core, every thread through it, so
//! what parallelism hides shows up as latency (the HUD's phase split, the
//! stress test's pacing). Reversible; the original mask is kept.
//!
//! Windows pins the whole process; Linux walks `/proc/self/task` and pins
//! each thread (a process-wide call is not offered); macOS has no public
//! API, and serial mode reports itself unavailable there.

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(super) type SerialMask = usize;
#[cfg(target_os = "linux")]
pub(super) type SerialMask = libc::cpu_set_t;

#[cfg(target_os = "windows")]
mod affinity {
    use super::SerialMask;

    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
        fn GetProcessAffinityMask(
            process: *mut core::ffi::c_void,
            mask: *mut usize,
            system: *mut usize,
        ) -> i32;
        fn SetProcessAffinityMask(process: *mut core::ffi::c_void, mask: usize) -> i32;
    }

    pub fn supported() -> bool {
        true
    }
    /// Pin to one core; the mask the process had, if it could be read.
    pub fn pin_to_one_core() -> Option<SerialMask> {
        unsafe {
            let process = GetCurrentProcess();
            let mut mask = 0usize;
            let mut system = 0usize;
            if GetProcessAffinityMask(process, &raw mut mask, &raw mut system) == 0 {
                return None;
            }
            (SetProcessAffinityMask(process, 1) != 0).then_some(mask)
        }
    }
    /// Put `mask` back. Whether it took.
    pub fn restore(mask: SerialMask) -> bool {
        unsafe { SetProcessAffinityMask(GetCurrentProcess(), mask) != 0 }
    }
}

#[cfg(target_os = "linux")]
mod affinity {
    use std::fs;

    use super::SerialMask;

    const SIZE: usize = std::mem::size_of::<libc::cpu_set_t>();

    /// Every thread of the process, by the kernel's task ids.
    fn tasks() -> Vec<i32> {
        fs::read_dir("/proc/self/task")
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| entry.file_name().to_string_lossy().parse::<i32>().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn supported() -> bool {
        true
    }
    /// Pin every thread to core 0; the mask the calling thread had.
    pub fn pin_to_one_core() -> Option<SerialMask> {
        let mut before = unsafe { std::mem::zeroed::<SerialMask>() };
        unsafe {
            if libc::sched_getaffinity(0, SIZE, &raw mut before) != 0 {
                return None;
            }
        }
        let mut one = unsafe { std::mem::zeroed::<SerialMask>() };
        unsafe {
            libc::CPU_ZERO(&mut one);
            libc::CPU_SET(0, &mut one);
        }
        let mut pinned = Vec::new();
        for task in tasks() {
            match unsafe { libc::sched_setaffinity(task, SIZE, &raw const one) } {
                0 => pinned.push(task),
                _ if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) => {}
                _ => {
                    for task in pinned {
                        unsafe { libc::sched_setaffinity(task, SIZE, &raw const before) };
                    }
                    return None;
                }
            }
        }
        Some(before)
    }
    /// Put `mask` back on every thread. Whether it took.
    pub fn restore(mask: SerialMask) -> bool {
        tasks()
            .into_iter()
            .all(|task| unsafe { libc::sched_setaffinity(task, SIZE, &raw const mask) == 0 })
    }
}

#[cfg(target_os = "macos")]
mod affinity {
    use super::SerialMask;

    pub fn supported() -> bool {
        false
    }
    pub fn pin_to_one_core() -> Option<SerialMask> {
        None
    }
    pub fn restore(_mask: SerialMask) -> bool {
        false
    }
}

pub(super) use affinity::{pin_to_one_core, restore, supported};
