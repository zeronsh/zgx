//! The process's own counters on macOS (`proc_pid_rusage`, `RUSAGE_INFO_V4`):
//! CPU time of every thread, retired instructions and cycles, and the
//! physical footprint `footprint` reports. Each arm reads them the same way,
//! from inside, so no tool attaches to it. Included by `#[path]` in the Rust
//! arms; the Filament arm reads the same struct from <libproc.h>.
// Each binary including this uses part of it.
#![allow(dead_code)]

#[cfg(target_os = "macos")]
mod mac {
    #[repr(C)]
    #[derive(Default)]
    pub struct RusageInfoV4 {
        pub uuid: [u8; 16],
        pub user_time: u64,
        pub system_time: u64,
        pub pkg_idle_wkups: u64,
        pub interrupt_wkups: u64,
        pub pageins: u64,
        pub wired_size: u64,
        pub resident_size: u64,
        pub phys_footprint: u64,
        pub proc_start_abstime: u64,
        pub proc_exit_abstime: u64,
        pub child_user_time: u64,
        pub child_system_time: u64,
        pub child_pkg_idle_wkups: u64,
        pub child_interrupt_wkups: u64,
        pub child_pageins: u64,
        pub child_elapsed_abstime: u64,
        pub diskio_bytesread: u64,
        pub diskio_byteswritten: u64,
        pub cpu_time_qos_default: u64,
        pub cpu_time_qos_maintenance: u64,
        pub cpu_time_qos_background: u64,
        pub cpu_time_qos_utility: u64,
        pub cpu_time_qos_legacy: u64,
        pub cpu_time_qos_user_initiated: u64,
        pub cpu_time_qos_user_interactive: u64,
        pub billed_system_time: u64,
        pub serviced_system_time: u64,
        pub logical_writes: u64,
        pub lifetime_max_phys_footprint: u64,
        pub instructions: u64,
        pub cycles: u64,
        pub billed_energy: u64,
        pub serviced_energy: u64,
        pub interval_max_phys_footprint: u64,
        pub runnable_time: u64,
    }
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV4) -> i32;
        fn getpid() -> i32;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }
    pub fn read() -> RusageInfoV4 {
        let mut info = RusageInfoV4::default();
        // RUSAGE_INFO_V4 = 4
        let status = unsafe { proc_pid_rusage(getpid(), 4, &mut info) };
        assert_eq!(status, 0, "proc_pid_rusage");
        info
    }
    /// Nanoseconds in `ticks` of the CPU time fields (mach absolute time).
    pub fn ns(ticks: u64) -> f64 {
        let mut base = Timebase { numer: 0, denom: 0 };
        unsafe { mach_timebase_info(&mut base) };
        ticks as f64 * base.numer as f64 / base.denom as f64
    }
}

/// What the counters read at one moment.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    /// CPU time of every thread, user and system, in nanoseconds.
    pub cpu_ns: f64,
    pub instructions: u64,
    pub cycles: u64,
    /// Bytes, as `footprint` counts them.
    pub footprint: u64,
    pub peak_footprint: u64,
    pub wakeups: u64,
}

#[cfg(target_os = "macos")]
pub fn sample() -> Sample {
    let info = mac::read();
    Sample {
        cpu_ns: mac::ns(info.user_time + info.system_time),
        instructions: info.instructions,
        cycles: info.cycles,
        footprint: info.phys_footprint,
        peak_footprint: info.lifetime_max_phys_footprint,
        wakeups: info.pkg_idle_wkups + info.interrupt_wkups,
    }
}
#[cfg(not(target_os = "macos"))]
pub fn sample() -> Sample {
    Sample::default()
}
