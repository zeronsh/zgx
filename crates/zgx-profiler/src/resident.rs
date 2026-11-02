/// The process's resident memory, mebibytes: what the OS has given it.
#[cfg(windows)]
pub fn mib() -> f32 {
    #[repr(C)]
    struct Counters {
        cb: u32,
        _page_faults: u32,
        _peak_working_set: usize,
        working_set: usize,
        _quota_peak_paged: usize,
        _quota_paged: usize,
        _quota_peak_nonpaged: usize,
        _quota_nonpaged: usize,
        _pagefile: usize,
        _peak_pagefile: usize,
    }
    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(
            process: *mut core::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
    }
    let mut counters: Counters = unsafe { std::mem::zeroed() };
    counters.cb = size_of::<Counters>() as u32;
    // SAFETY: `counters` is the struct its `cb` describes, for this process.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    match ok {
        0 => 0.,
        _ => counters.working_set as f32 / (1024. * 1024.),
    }
}

/// The process's resident memory, mebibytes.
#[cfg(target_os = "macos")]
pub fn mib() -> f32 {
    #[repr(C)]
    struct RusageInfoV4 {
        uuid: [u8; 16],
        fields: [u64; 40],
    }
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV4) -> i32;
    }
    let mut info = RusageInfoV4 {
        uuid: [0; 16],
        fields: [0; 40],
    };
    // SAFETY: RUSAGE_INFO_V4 fills the struct; field 6 is ri_resident_size.
    let ok = unsafe { proc_pid_rusage(std::process::id() as i32, 4, &mut info) };
    match ok {
        0 => info.fields[6] as f32 / (1024. * 1024.),
        _ => 0.,
    }
}

/// The process's resident memory, mebibytes.
#[cfg(target_os = "linux")]
pub fn mib() -> f32 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let resident_pages: f32 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|pages| pages.parse().ok())
        .unwrap_or(0.);
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(0) as f32;
    resident_pages * page / (1024. * 1024.)
}

/// The process's resident memory, mebibytes.
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn mib() -> f32 {
    0.
}
