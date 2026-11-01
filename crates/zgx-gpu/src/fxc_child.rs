//! An app's shaders compiled by FXC in a process of their own (Windows).
//!
//! FXC's optimizer has overflowed every stack on some modules (#1136), and a
//! stack overflow ends the process from any thread: no guard in the process
//! survives it, and catching it can leave FXC's own locks held. zgx's own
//! shaders are compiled under a gate (`every_shader`); a shader fill or layer
//! shader an app writes no gate has seen. Those compile here instead: the
//! app's own executable is started again as a compiler (`ZGX_FXC_CHILD` in its
//! environment, answered by `child_main` before the app's `main` runs), handed
//! the HLSL on stdin, and writes the bytecode to stdout. A child that crashes
//! or hangs fails that shader's pipeline, which then draws nothing; the app
//! goes on. Its output is kept on disk (`cache`), so each shader is compiled
//! this way once a machine; the bytecode reaches Direct3D 12 as a passthrough
//! module (`dx12_fill`), so no wgpu patch is needed.
//!
//! Where the executable can't be started again (a sandbox refusing it), or
//! isn't zgx's (zgx built into a DLL another program loads), the compile
//! happens in the process as before.
//!
//! The initializer is the executable's own: a C runtime initializer runs for
//! the module it is linked into, and only checks `CHILD`, which nothing sets
//! but `compile_in_child`, for the child alone (never in its own
//! environment), and the child exits before starting anything.
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Set in the child: it compiles and exits before the app's `main`.
const CHILD: &str = "ZGX_FXC_CHILD";
/// Set to `0`, an app's shaders compile in its own process (as before).
const OPT_OUT: &str = "ZGX_FXC_IN_CHILD";
/// How long a compile may take before the child is stopped and the shader
/// fails: FXC takes seconds on zgx's largest module.
const LIMIT: Duration = Duration::from_secs(30);
/// Set in a test: the child ends as FXC's overflow would end it, before
/// compiling anything (`std::process::abort`).
const TEST_ABORT: &str = "ZGX_FXC_CHILD_TEST_ABORT";
/// Compiles handed to a child this process (tests read it).
static COMPILED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// An FXC compile: HLSL, its entry point, target (`ps_5_1`) and flags.
#[derive(Debug)]
pub(crate) struct Job<'a> {
    pub source: &'a str,
    pub entry_point: &'a str,
    pub target: &'a str,
    pub flags: u32,
}

/// Whether an app's shaders may compile in a child here: not opted out, and
/// this code the executable's own (`in_the_executable`).
pub(crate) fn available() -> bool {
    !std::env::var_os(OPT_OUT).is_some_and(|v| v == "0") && in_the_executable()
}

/// `job`'s bytecode: from the cache, else compiled in a child (kept in the
/// cache); FXC's error, or the child's end, as an error. `None`: no child
/// could be started, so the caller compiles in the process.
pub(crate) fn compile(job: &Job<'_>) -> Option<Result<Vec<u8>, String>> {
    let key = cache::key(job);
    if let Some(code) = cache::read(&key) {
        return Some(Ok(code));
    }
    let compiled = compile_in_child(job)?;
    if let Ok(code) = &compiled {
        cache::write(&key, code);
    }
    Some(compiled)
}

/// The job, as the child reads it: flags, target and entry point on lines of
/// their own, then the source.
fn encode(job: &Job<'_>) -> Vec<u8> {
    format!(
        "{}\n{}\n{}\n{}",
        job.flags, job.target, job.entry_point, job.source
    )
    .into_bytes()
}

/// Whether this code is the process's executable's own: started again, the
/// executable answers `CHILD` (`child_main`). Built into a DLL a host loads
/// (a plugin), the executable is the host's, which would start whole: the
/// compile then stays in the process.
fn in_the_executable() -> bool {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleHandleExW, GetModuleHandleW,
    };
    let mut own = HMODULE::default();
    // SAFETY: an address in this module, its handle not kept counted.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(child_main as *const u16),
            &mut own,
        )
    };
    // SAFETY: the process's executable, always loaded.
    let exe = unsafe { GetModuleHandleW(None) };
    found.is_ok() && exe.is_ok_and(|exe| exe == own)
}

fn compile_in_child(job: &Job<'_>) -> Option<Result<Vec<u8>, String>> {
    let exe = std::env::current_exe().ok()?;
    let started = Instant::now();
    COMPILED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut child = Command::new(exe)
        .env(CHILD, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        // Can't be started again: compile here, as before.
        .ok()?;
    // Written whole before the output is read: FXC reads all of its source
    // first.
    let wrote = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(&encode(job)));
    let (mut stdout, mut stderr) = (child.stdout.take()?, child.stderr.take()?);
    let reader = std::thread::spawn(move || {
        let (mut out, mut err) = (Vec::new(), String::new());
        let _ = stdout.read_to_end(&mut out);
        let _ = stderr.read_to_string(&mut err);
        (out, err)
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < LIMIT => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let (out, err) = reader.join().unwrap_or_default();
    zgx::profile::note(|| {
        format!(
            "an app's shader compiled by FXC in a process of its own: {:?}",
            started.elapsed()
        )
    });
    Some(match status {
        None => Err(format!(
            "FXC took longer than {}s in its own process",
            LIMIT.as_secs()
        )),
        Some(status) if status.success() && !out.is_empty() && matches!(wrote, Some(Ok(()))) => {
            Ok(out)
        }
        Some(status) if status.code() == Some(1) => Err(format!("FXC D3DCompile error: {err}")),
        Some(status) => Err(format!(
            "FXC ended its own process ({status}) compiling this shader: it draws nothing"
        )),
    })
}

/// FXC's `D3DCompile`, loaded by name in the child alone.
type D3DCompile = unsafe extern "system" fn(
    source: *const core::ffi::c_void,
    size: usize,
    name: *const u8,
    defines: *const core::ffi::c_void,
    include: *mut core::ffi::c_void,
    entry: *const u8,
    target: *const u8,
    flags1: u32,
    flags2: u32,
    code: *mut Option<windows::Win32::Graphics::Direct3D::ID3DBlob>,
    errors: *mut Option<windows::Win32::Graphics::Direct3D::ID3DBlob>,
) -> windows::core::HRESULT;

/// In the child (`CHILD` set), before the app's `main`: compile the job on
/// stdin, write the bytecode to stdout, and exit (0 compiled, 1 FXC's
/// error, on stderr). Elsewhere one environment lookup, nothing more.
extern "C" fn child_main() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    if std::env::var_os(TEST_ABORT).is_some() {
        std::process::abort();
    }
    std::process::exit(match child_compile() {
        Ok(code) => {
            let mut stdout = std::io::stdout().lock();
            match stdout.write_all(&code).and_then(|()| stdout.flush()) {
                Ok(()) => 0,
                Err(_) => 2,
            }
        }
        Err(message) => {
            let _ = std::io::stderr().write_all(message.as_bytes());
            1
        }
    });
}

fn child_compile() -> Result<Vec<u8>, String> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::{PCSTR, w};
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    let mut parts = input.splitn(4, |&b| b == b'\n');
    let mut field = || -> Result<Vec<u8>, String> {
        parts
            .next()
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "a malformed job".to_string())
    };
    let flags: u32 = String::from_utf8_lossy(&field()?)
        .parse()
        .map_err(|_| "a malformed job".to_string())?;
    let (mut target, mut entry, source) = (field()?, field()?, field()?);
    target.push(0);
    entry.push(0);
    // SAFETY: a system DLL loaded by name, and its exported function of the
    // signature FXC documents; the blobs are read while they live.
    unsafe {
        let library = LoadLibraryW(w!("d3dcompiler_47.dll")).map_err(|e| e.to_string())?;
        let compile = GetProcAddress(library, PCSTR(c"D3DCompile".as_ptr().cast()))
            .ok_or("no D3DCompile in d3dcompiler_47.dll")?;
        let compile: D3DCompile = std::mem::transmute(compile);
        let (mut code, mut errors) = (None, None);
        let result = compile(
            source.as_ptr().cast(),
            source.len(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            entry.as_ptr(),
            target.as_ptr(),
            flags,
            0,
            &mut code,
            &mut errors,
        );
        let bytes = |blob: &windows::Win32::Graphics::Direct3D::ID3DBlob| {
            std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
                .to_vec()
        };
        if result.is_err() {
            return Err(errors.as_ref().map_or_else(
                || result.to_string(),
                |e| String::from_utf8_lossy(&bytes(e)).into_owned(),
            ));
        }
        code.as_ref()
            .map(bytes)
            .ok_or_else(|| "FXC wrote no code".to_string())
    }
}

/// Run before the app's `main` (the C runtime's initializers): the child's
/// entry, referenced from `install` so the linker keeps it.
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static CHILD_MAIN: extern "C" fn() = child_main;

/// Compiles handed to a child so far in this process.
pub(crate) fn compiled() -> usize {
    COMPILED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Keeps `CHILD_MAIN` in any binary that compiles an app's shader.
pub(crate) fn keep() -> *const () {
    &CHILD_MAIN as *const _ as *const ()
}

/// Compiled shaders kept between launches: `%LOCALAPPDATA%\zgx\fxc`, a file
/// a shader, named by a hash of everything FXC was given and FXC's own
/// version (its DLL's size and time), the key kept in full and compared
/// before the bytecode is used. Bounded: past `KEEP` files, the oldest go.
mod cache {
    use std::path::PathBuf;
    /// Shaders kept: an app's fills and layer shaders, a few dozen at most.
    const KEEP: usize = 256;

    fn dir() -> Option<PathBuf> {
        Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("zgx\\fxc"))
    }
    /// FXC's version: d3dcompiler_47.dll's size and modification time.
    fn fxc_version() -> String {
        let system = std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32\\d3dcompiler_47.dll"));
        system
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| {
                let time = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                format!("{}:{time}", meta.len())
            })
            .unwrap_or_default()
    }
    pub(super) fn key(job: &super::Job<'_>) -> String {
        format!(
            "zgx fxc 1\n{}\n{}\n{}\n{}\n{}",
            fxc_version(),
            job.flags,
            job.target,
            job.entry_point,
            job.source
        )
    }
    fn file(key: &str) -> Option<PathBuf> {
        // FNV-1a: stable across runs and builds, unlike `DefaultHasher`.
        let hash = key.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
        Some(dir()?.join(format!("{hash:016x}.dxbc")))
    }
    pub(super) fn read(key: &str) -> Option<Vec<u8>> {
        let bytes = std::fs::read(file(key)?).ok()?;
        let length = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
        let kept = bytes.get(4..4 + length)?;
        (kept == key.as_bytes()).then(|| bytes[4 + length..].to_vec())
    }
    pub(super) fn write(key: &str, code: &[u8]) {
        let (Some(path), Some(dir)) = (file(key), dir()) else {
            return;
        };
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let mut bytes = (key.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(key.as_bytes());
        bytes.extend_from_slice(code);
        // Whole, then renamed into place: another process never reads half.
        let partial = path.with_extension(format!("{}.part", std::process::id()));
        if std::fs::write(&partial, bytes).is_ok() && std::fs::rename(&partial, &path).is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        trim(&dir);
    }
    /// The oldest files go past `KEEP`.
    fn trim(dir: &std::path::Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "dxbc"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        if files.len() <= KEEP {
            return;
        }
        files.sort();
        for (_, path) in &files[..files.len() - KEEP] {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_is_flags_target_entry_then_source() {
        let job = Job {
            source: "float4 main() : SV_Target {\n return 1; }",
            entry_point: "main",
            target: "ps_5_1",
            flags: 2048,
        };
        assert_eq!(
            encode(&job),
            b"2048\nps_5_1\nmain\nfloat4 main() : SV_Target {\n return 1; }".to_vec()
        );
    }

    /// The cache keys a shader by everything FXC is given, and keeps the key
    /// whole: another source, entry, target or flags is another file.
    #[test]
    fn the_cache_keys_by_everything_fxc_is_given() {
        let job = |source, flags| Job {
            source,
            entry_point: "zgx_fill",
            target: "ps_5_1",
            flags,
        };
        let a = cache::key(&job("a", 2048));
        assert_ne!(a, cache::key(&job("b", 2048)));
        assert_ne!(a, cache::key(&job("a", 2049)));
        assert!(a.ends_with("\n2048\nps_5_1\nzgx_fill\na"), "{a}");
    }
}
