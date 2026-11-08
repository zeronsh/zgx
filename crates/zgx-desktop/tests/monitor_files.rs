//! The resource monitor example's files (`examples/zgx_monitor/files.rs`):
//! where it finds its scripts, and the labels apps write for it.
#[path = "../examples/zgx_monitor/files.rs"]
#[allow(dead_code)]
mod files;

use files::{FRESH, LabelReader, Process, RENEW, parse, script};
use std::{
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime},
};

/// An empty directory of this test's own, removed when dropped.
struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("zgx-monitor-files-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn names(list: &[(i32, Rc<str>)]) -> Vec<(i32, &str)> {
    list.iter().map(|(pid, label)| (*pid, &**label)).collect()
}

#[test]
fn labels_parse_names_and_counts_and_skip_the_rest() {
    let said = parse(
        "12\tRenderer: Docs\n\
         #\tTabs\t3\n\
         13\t\n\
         x\tNot a pid\n\
         -4\tNegative\n\
         #\tWindows\tmany\n\
         #\t\t2\n\
         #\tNaN\tNaN\n\
         just text\n\
         14\tGPU\twith a tab\n",
    );
    assert_eq!(
        names(&said.names),
        [(12, "Renderer: Docs"), (14, "GPU\twith a tab")]
    );
    assert_eq!(
        said.counts
            .iter()
            .map(|(label, count)| (&**label, *count))
            .collect::<Vec<_>>(),
        [("Tabs", 3.)]
    );
}

#[test]
fn a_file_is_its_owners_while_the_owner_runs_from_before_it() {
    let dir = Scratch::new("owners");
    let at = |seconds: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + seconds);
    let write = |owner: i32, text: &str, written: SystemTime| {
        let path = dir.0.join(format!("labels-{owner}"));
        std::fs::write(&path, text).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(written).unwrap();
    };
    // 10 runs, from before its file; it names 11, started before the file,
    // and 12, started after it (a pid reused since).
    write(10, "11\tRenderer\n12\tStale\n#\tTabs\t2\n", at(100));
    // 20 has quit; 30 is a process started after its file was written (the
    // file's app quit and another process has its pid); 40 can't be read.
    write(20, "21\tGone\n", at(100));
    write(30, "31\tReused\n", at(100));
    write(40, "41\tUnreadable\n", at(100));
    let process = |pid: i32| match pid {
        10 | 11 => Process::Started(at(50)),
        12 | 30 => Process::Started(at(150)),
        40 => Process::Unknown,
        _ => Process::Gone,
    };
    let mut reader = LabelReader::new(dir.0.clone());
    let labels = reader.read(process);
    assert_eq!(labels.names.len(), 1, "{:?}", labels.names);
    assert_eq!(labels.names.get(&11).map(|l| &**l), Some("Renderer"));
    assert_eq!(
        labels
            .counts
            .iter()
            .map(|(owner, label, count)| (*owner, &**label, *count))
            .collect::<Vec<_>>(),
        [(10, "Tabs", 2.)]
    );
    let exists = |owner: i32| dir.0.join(format!("labels-{owner}")).exists();
    assert!(exists(10) && !exists(20) && !exists(30) && exists(40));

    // Read again only once written again: the same time, other words, are
    // the file as read; a new time, the new words.
    write(10, "11\tChanged\n", at(100));
    let labels = reader.read(process);
    assert_eq!(labels.names.get(&11).map(|l| &**l), Some("Renderer"));
    write(10, "11\tChanged\n", at(101));
    let labels = reader.read(process);
    assert_eq!(labels.names.get(&11).map(|l| &**l), Some("Changed"));
    assert!(labels.counts.is_empty());
}

#[test]
fn the_heartbeat_is_renewed_not_rewritten_at_every_read() {
    let scratch = Scratch::new("heartbeat");
    // A directory the reader makes, private.
    let dir = scratch.0.join("zgx-monitor");
    let heartbeat = dir.join("heartbeat");
    let mut reader = LabelReader::new(dir.clone());
    reader.read(|_| Process::Gone);
    assert!(RENEW < FRESH);
    let modified = || std::fs::metadata(&heartbeat).unwrap().modified().unwrap();
    assert!(modified().elapsed().unwrap_or_default() < RENEW);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
    // Read again at once: left as it is.
    let old = SystemTime::now() - Duration::from_secs(60);
    std::fs::File::options()
        .write(true)
        .open(&heartbeat)
        .unwrap()
        .set_modified(old)
        .unwrap();
    reader.read(|_| Process::Gone);
    assert_eq!(modified(), old);
    assert_eq!(std::fs::read(&heartbeat).unwrap(), b"");
}

#[test]
fn scripts_come_from_the_flag_a_bundle_or_the_checkout() {
    let scratch = Scratch::new("scripts");
    let checkout = Path::new("/checkout/zgx_monitor");
    let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let none = args(&["zgx_monitor"]);
    // `--scripts <dir>` wins.
    assert_eq!(
        script(
            &args(&["zgx_monitor", "--scripts", "/elsewhere"]),
            None,
            checkout
        ),
        Path::new("/elsewhere/app.luau")
    );
    // An app bundle's resources, named for its executable.
    let contents = scratch.0.join("Any.app/Contents");
    let exe = contents.join("MacOS/monitor");
    let shipped = contents.join("Resources/monitor/app.luau");
    std::fs::create_dir_all(shipped.parent().unwrap()).unwrap();
    assert_eq!(
        script(&none, Some(&exe), checkout),
        checkout.join("app.luau"),
        "a bundle without the scripts runs the checkout's"
    );
    std::fs::write(&shipped, "").unwrap();
    assert_eq!(script(&none, Some(&exe), checkout), shipped);
    // Only a bundle's: not any directory two up from the executable.
    let loose = scratch.0.join("target/release/monitor");
    std::fs::create_dir_all(scratch.0.join("target/Resources/monitor")).unwrap();
    std::fs::write(scratch.0.join("target/Resources/monitor/app.luau"), "").unwrap();
    assert_eq!(
        script(&none, Some(&loose), checkout),
        checkout.join("app.luau")
    );
    assert_eq!(script(&none, None, checkout), checkout.join("app.luau"));
}
