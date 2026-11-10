//! The process's own counters on macOS (`proc_pid_rusage`,
//! `RUSAGE_INFO_V4`): CPU time of every thread, instructions retired and
//! the physical footprint, read from inside as the 3D comparison's arms
//! read them. Elsewhere they read as zero (and the bench says so).

/// The counters at one moment.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    /// CPU time of every thread, user and system, nanoseconds.
    pub cpu_ns: f64,
    /// Instructions retired by the process.
    pub instructions: u64,
    /// Physical footprint, bytes.
    pub footprint: u64,
}

/// Whether [`sample`] reads real counters here.
pub const AVAILABLE: bool = cfg!(target_os = "macos");

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
        // SAFETY: `info` is a `rusage_info_v4` (flavor 4), written whole by
        // the call; the pid is this process's.
        let status = unsafe { proc_pid_rusage(getpid(), 4, &mut info) };
        assert_eq!(status, 0, "proc_pid_rusage");
        info
    }
    pub fn ns(ticks: u64) -> f64 {
        let mut base = Timebase { numer: 0, denom: 0 };
        // SAFETY: writes the two fields of `base`.
        unsafe { mach_timebase_info(&mut base) };
        ticks as f64 * base.numer as f64 / base.denom.max(1) as f64
    }
}

/// The counters now.
pub fn sample() -> Sample {
    #[cfg(target_os = "macos")]
    {
        let info = mac::read();
        Sample {
            cpu_ns: mac::ns(info.user_time + info.system_time),
            instructions: info.instructions,
            footprint: info.phys_footprint,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Sample::default()
    }
}

/// The `p`th percentile (0 to 100) of `values`, by nearest rank.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((p / 100.) * sorted.len() as f64).ceil().max(1.) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

/// FNV-1a of `bytes`: the bench's frame hash.
pub fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_by_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 50.), 50.);
        assert_eq!(percentile(&v, 99.), 99.);
        assert_eq!(percentile(&v, 100.), 100.);
        assert_eq!(percentile(&[3.], 90.), 3.);
    }

    #[test]
    fn the_counters_move() {
        if !AVAILABLE {
            return;
        }
        let a = sample();
        let mut x = 0u64;
        for i in 0..1_000_000u64 {
            x = x.wrapping_add(i * i);
        }
        std::hint::black_box(x);
        let b = sample();
        assert!(b.instructions > a.instructions);
    }
}
