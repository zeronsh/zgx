//! A script's `print` reaches stderr a whole line at a time, whatever else
//! writes there meanwhile (libwayland's `WAYLAND_DEBUG` trace, from another
//! thread): a probe reading `LOCK true` read `LOCK true[3664360.153]
//! wl_pointer@45.frame()` once, the line and its newline written apart.
//! Its own process: the test runs itself again as a child, its stderr a
//! pipe, while a thread there writes to stderr as fast as it can.
use std::{io::Read, process::Command};

const LINES: usize = 3000;
const CHILD: &str = "ZGX_PRINT_WHOLE_LINES_CHILD";

#[test]
fn a_printed_line_reaches_stderr_whole() {
    if std::env::var_os(CHILD).is_some() {
        return child();
    }
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_printed_line_reaches_stderr_whole",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert!(child.wait().unwrap().success());
    let printed: Vec<&str> = out.lines().filter(|line| line.contains("LINE ")).collect();
    let broken: Vec<&&str> = printed
        .iter()
        .filter(|line| {
            !(line.starts_with("LINE ") && line[5..].bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    assert!(
        broken.is_empty(),
        "{} of {} printed lines came out with other output inside them, e.g. {:?}",
        broken.len(),
        printed.len(),
        &broken[..broken.len().min(3)]
    );
    assert_eq!(printed.len(), LINES, "every line printed came out");
}

/// The child: a thread writing to stderr in raw `write`s (as C does), and
/// a script printing `LINES` lines meanwhile.
fn child() {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let noise = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            #[cfg(windows)]
            let mut stderr = {
                use std::os::windows::io::AsHandle;
                std::fs::File::from(std::io::stderr().as_handle().try_clone_to_owned().unwrap())
            };
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let text = b"[1.000] wl_pointer@45.frame()\n";
                #[cfg(unix)]
                // SAFETY: a write of a buffer it holds to stderr's descriptor.
                unsafe {
                    libc::write(2, text.as_ptr().cast(), text.len())
                };
                #[cfg(windows)]
                {
                    use std::io::Write;
                    // A cloned OS handle bypasses Rust's stderr lock, as
                    // another library's writes do on Unix.
                    stderr.write_all(text).unwrap();
                }
            }
        })
    };
    let runtime = zgx::reactive::Runtime::new();
    let host = zgx_script::ScriptHost::inline(
        &runtime,
        "app",
        format!("for i = 1, {LINES} do print(\"LINE \" .. i) end\nreturn ui.text {{ \"\" }}"),
    );
    host.reload().unwrap();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    noise.join().unwrap();
}
