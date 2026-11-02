//! What a bench does for itself, left out of the instruction gate's count:
//! `outside` stops cachegrind's counting for its closure (a client request,
//! as valgrind's `CACHEGRIND_STOP_INSTRUMENTATION` and `_START_` make it),
//! so a bench's own model (data made beforehand for every update) is not
//! counted as the engine's. Run without valgrind the requests do nothing.

/// Runs `f` with cachegrind's counting stopped, then counts again.
pub fn outside<T>(f: impl FnOnce() -> T) -> T {
    request(STOP);
    let out = f();
    request(START);
    out
}

// cachegrind.h: VG_USERREQ_TOOL_BASE('C', 'G'), then the one after it.
const START: usize = 0x4347_0000;
const STOP: usize = 0x4347_0001;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn request(code: usize) {
    let args = [code, 0, 0, 0, 0, 0];
    let mut result = 0usize;
    // SAFETY: valgrind's client request sequence (valgrind.h): rotations
    // that leave the register as it was, then a no-op move. Natively it does
    // nothing; under valgrind it hands `args` to the tool and sets `result`.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::asm!(
            "rol rdi, 3", "rol rdi, 13", "rol rdi, 61", "rol rdi, 51", "xchg rbx, rbx",
            inout("rdx") result, in("rax") args.as_ptr(), options(nostack),
        );
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        std::arch::asm!(
            "ror x12, x12, #3", "ror x12, x12, #13", "ror x12, x12, #51", "ror x12, x12, #61",
            "orr x10, x10, x10",
            inout("x3") result, in("x4") args.as_ptr(), options(nostack),
        );
    }
    let _ = result;
}

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
fn request(_: usize) {}
