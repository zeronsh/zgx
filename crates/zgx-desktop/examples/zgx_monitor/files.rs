//! The monitor's files, kept apart from its window so their tests run on
//! their own (`tests/monitor_files.rs`): where its scripts are, and the
//! labels apps write for it (the format is in `zgx_monitor.rs`'s module
//! documentation).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

/// The scripts' entry: `--scripts <dir>`'s `app.luau`; shipped in a macOS
/// app bundle (`<name>.app/Contents/MacOS/<executable>`), the bundle's
/// `Contents/Resources/<executable>/app.luau` when it has one; otherwise
/// `checkout`'s.
pub fn script(args: &[String], exe: Option<&Path>, checkout: &Path) -> PathBuf {
    if let Some(dir) = args
        .iter()
        .position(|arg| arg == "--scripts")
        .and_then(|i| args.get(i + 1))
    {
        return Path::new(dir).join("app.luau");
    }
    let shipped = exe.and_then(|exe| {
        let macos = exe.parent()?;
        let contents = macos.parent()?;
        let bundled = macos.file_name()? == "MacOS"
            && contents.file_name()? == "Contents"
            && contents.parent()?.extension()? == "app";
        let path = contents
            .join("Resources")
            .join(exe.file_name()?)
            .join("app.luau");
        (bundled && path.exists()).then_some(path)
    });
    shipped.unwrap_or_else(|| checkout.join("app.luau"))
}

/// How long the heartbeat counts as fresh: an app writes labels only while
/// it is at most this old. (The apps' side: the monitor only renews it.)
#[allow(dead_code)]
pub const FRESH: Duration = Duration::from_secs(5);
/// How often the monitor renews the heartbeat: well inside `FRESH`, and not
/// at every sample.
pub const RENEW: Duration = Duration::from_secs(2);

/// A process as the monitor can tell: gone, running since a time, or
/// running for all it knows (it may not read the process).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Process {
    Gone,
    Started(SystemTime),
    Unknown,
}

/// What the apps in view call their processes, and count of their own.
#[derive(Default)]
pub struct Labels {
    pub names: HashMap<i32, Rc<str>>,
    /// An app's pid, what it counts and how many.
    pub counts: Vec<(i32, Rc<str>, f32)>,
}

/// What one labels file says.
#[derive(Default)]
pub struct Said {
    /// `<pid>\t<label>` lines.
    pub names: Vec<(i32, Rc<str>)>,
    /// `#\t<label>\t<count>` lines.
    pub counts: Vec<(Rc<str>, f32)>,
}

/// One labels file's lines. Other lines are skipped.
pub fn parse(text: &str) -> Said {
    let mut said = Said::default();
    for line in text.lines() {
        if let Some(count) = line.strip_prefix("#\t") {
            if let Some((label, count)) = count.split_once('\t')
                && !label.is_empty()
                && let Ok(count) = count.parse::<f32>()
                && count.is_finite()
            {
                said.counts.push((label.into(), count));
            }
        } else if let Some((pid, label)) = line.split_once('\t')
            && let Ok(pid) = pid.parse::<i32>()
            && pid > 0
            && !label.is_empty()
        {
            said.names.push((pid, label.into()));
        }
    }
    said
}

/// A labels file as last read: when it was written, and what it said.
struct File {
    written: SystemTime,
    said: Said,
}

/// Reads the labels apps write into `directory`, keeping its heartbeat
/// fresh.
pub struct LabelReader {
    directory: PathBuf,
    /// When it last renewed the heartbeat.
    renewed: Option<Instant>,
    /// Each owner's file, read again only once it is written again.
    files: HashMap<i32, File>,
}
impl LabelReader {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            renewed: None,
            files: HashMap::new(),
        }
    }

    /// The labels every app's file says now. `process` tells a pid's
    /// process. A file is its owner's only while the owner runs and started
    /// before it was written: one left by an app that quit, or by an earlier
    /// process with the owner's pid, is removed. A file names only processes
    /// started before it was written, so a pid reused since is not named.
    pub fn read(&mut self, process: impl Fn(i32) -> Process) -> Labels {
        self.renew();
        let mut labels = Labels::default();
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            self.files.clear();
            return labels;
        };
        let mut present = Vec::new();
        for entry in entries.flatten() {
            let Some(owner) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix("labels-"))
                .and_then(|pid| pid.parse::<i32>().ok())
                .filter(|pid| *pid > 0)
            else {
                continue;
            };
            let Ok(written) = entry.metadata().and_then(|m| m.modified()) else {
                continue;
            };
            match process(owner) {
                Process::Started(started) if started <= written => {}
                // Can't tell whose it is: neither used nor removed.
                Process::Unknown => continue,
                // Its app quit, or the pid is another process's now.
                Process::Gone | Process::Started(_) => {
                    let _ = std::fs::remove_file(entry.path());
                    continue;
                }
            }
            present.push(owner);
            if self.files.get(&owner).is_none_or(|f| f.written != written) {
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                let said = parse(&text);
                self.files.insert(owner, File { written, said });
            }
            let file = &self.files[&owner];
            for (pid, label) in &file.said.names {
                if matches!(process(*pid), Process::Started(started) if started <= file.written) {
                    labels.names.insert(*pid, label.clone());
                }
            }
            for (label, count) in &file.said.counts {
                labels.counts.push((owner, label.clone(), *count));
            }
        }
        self.files.retain(|owner, _| present.contains(owner));
        labels
    }

    /// Renews the heartbeat once it is `RENEW` old (its directory made,
    /// private, as needed).
    fn renew(&mut self) {
        if self.renewed.is_some_and(|at| at.elapsed() < RENEW) {
            return;
        }
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        let _ = builder.create(&self.directory);
        let heartbeat = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.directory.join("heartbeat"));
        if let Ok(file) = heartbeat
            && file.set_modified(SystemTime::now()).is_ok()
        {
            self.renewed = Some(Instant::now());
        }
    }
}
