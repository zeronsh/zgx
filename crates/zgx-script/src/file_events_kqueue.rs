//! Changes to a set of files, from the kernel (kqueue's vnode events, iOS has
//! no FSEvents): a watching host waits for one with nothing running meanwhile,
//! where polling stats every file a few times a second. Each file and the
//! directory holding it are watched, so a file an editor replaces (a save as
//! write-and-rename) is seen as well as one written in place. A directory's
//! event names no entry, so its watched files are looked at to see which
//! changed: an editor's swap file written beside them tells nobody.
use std::{
    collections::HashMap,
    ffi::{CString, OsString},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// What identifies a file's contents: its inode, length and when it changed.
type Stamp = Option<(u64, u64, i64, i64)>;

fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.ino(), meta.size(), meta.mtime(), meta.mtime_nsec()))
}

/// One watched descriptor.
enum Watch {
    /// A file, or a directory watched whole: any event counts. The
    /// descriptor is held for its events: closing it ends them.
    Any { _open: OwnedFd },
    /// The directory holding watched files: an event counts if one of them
    /// changed, each kept with what it last looked like.
    Entries {
        _open: OwnedFd,
        dir: PathBuf,
        names: Vec<(OsString, Stamp)>,
    },
}

/// What `kevent` says of a descriptor is found by it here.
type Watches = HashMap<i32, Watch>;

/// `EVFILT_USER`'s identifier: this asks the thread to look at `done`.
const WAKE: usize = 1;

pub(crate) struct FileEvents {
    queue: Arc<OwnedFd>,
    done: Arc<AtomicBool>,
    watches: Arc<Mutex<Watches>>,
    /// What is watched now: files, then directories.
    watched: (Vec<PathBuf>, Vec<PathBuf>),
    changed: zgx::task::LatestReceiver<()>,
}

fn empty_event() -> libc::kevent {
    libc::kevent {
        ident: 0,
        filter: 0,
        flags: 0,
        fflags: 0,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}

impl FileEvents {
    /// A kqueue and the thread that waits on it; `None` if the kernel gives
    /// none.
    pub fn new() -> Option<Self> {
        // SAFETY: takes nothing; a new descriptor or -1.
        let queue = unsafe { libc::kqueue() };
        if queue < 0 {
            return None;
        }
        // SAFETY: the descriptor just made, owned by nothing else.
        let queue = Arc::new(unsafe { OwnedFd::from_raw_fd(queue) });
        let wake = libc::kevent {
            ident: WAKE,
            filter: libc::EVFILT_USER,
            flags: libc::EV_ADD | libc::EV_CLEAR,
            ..empty_event()
        };
        // SAFETY: one initialized change, no events asked for.
        let added = unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                &wake,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        if added < 0 {
            return None;
        }
        let (tell, changed) = zgx::task::latest::<()>();
        let done = Arc::new(AtomicBool::new(false));
        let watches = Arc::new(Mutex::new(Watches::new()));
        let (ended, heard, open) = (done.clone(), watches.clone(), queue.clone());
        let spawned = std::thread::Builder::new()
            .name("zgx-script file events".into())
            .spawn(move || {
                let mut events = [empty_event(); 32];
                loop {
                    // SAFETY: room for 32 events, written for the call only,
                    // on the queue `open` keeps open; no timeout: it sleeps.
                    let n = unsafe {
                        libc::kevent(
                            open.as_raw_fd(),
                            std::ptr::null(),
                            0,
                            events.as_mut_ptr(),
                            32,
                            std::ptr::null(),
                        )
                    };
                    if ended.load(Ordering::Acquire)
                        || (n < 0
                            && std::io::Error::last_os_error().kind()
                                != std::io::ErrorKind::Interrupted)
                    {
                        return;
                    }
                    let mut any = false;
                    let mut watches = heard.lock().unwrap();
                    for event in &events[..n.max(0) as usize] {
                        if event.filter == libc::EVFILT_VNODE {
                            any |= concerns(&mut watches, event.ident as i32);
                        }
                    }
                    if any {
                        // What was told is not news the next time a directory
                        // stirs, however it was heard.
                        refresh(&mut watches);
                    }
                    drop(watches);
                    if any {
                        tell.send(());
                    }
                }
            });
        // Not spawned: the closure, and its share of the queue, went with it.
        if spawned.is_err() {
            return None;
        }
        Some(Self {
            queue,
            done,
            watches,
            watched: Default::default(),
            changed,
        })
    }

    /// Watch `paths` (and their directories) and the entries of `whole` from
    /// now on, instead of what was watched before. Watching what is already
    /// watched adds nothing, unless a file was replaced (its descriptor still
    /// follows the old one).
    pub fn watch(&mut self, mut paths: Vec<PathBuf>, mut whole: Vec<PathBuf>, replaced: bool) {
        paths.sort();
        paths.dedup();
        whole.sort();
        whole.dedup();
        if (&paths, &whole) == (&self.watched.0, &self.watched.1) && !replaced {
            return;
        }
        let mut entries: HashMap<PathBuf, Vec<(OsString, Stamp)>> = HashMap::new();
        for path in &paths {
            if let (Some(dir), Some(name)) = (path.parent(), path.file_name()) {
                entries
                    .entry(dir.to_path_buf())
                    .or_default()
                    .push((name.to_owned(), stamp(path)));
            }
        }
        let mut watches = Watches::new();
        for path in paths.iter().chain(&whole) {
            if let Some(fd) = self.open(path) {
                watches.insert(fd.as_raw_fd(), Watch::Any { _open: fd });
            }
        }
        for (dir, names) in entries {
            if !whole.contains(&dir)
                && let Some(fd) = self.open(&dir)
            {
                watches.insert(
                    fd.as_raw_fd(),
                    Watch::Entries {
                        _open: fd,
                        dir,
                        names,
                    },
                );
            }
        }
        // What was watched closes here, its events going with it; events
        // read before this find their descriptor gone, or (reused) look again.
        *self.watches.lock().unwrap() = watches;
        self.watched = (paths, whole);
    }

    /// `path` opened for its events alone, its changes asked for.
    fn open(&self, path: &Path) -> Option<OwnedFd> {
        let name = CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: a NUL-terminated path; a new descriptor or -1.
        let fd = unsafe { libc::open(name.as_ptr(), libc::O_EVTONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        // SAFETY: the descriptor just made, owned by nothing else.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let change = libc::kevent {
            ident: fd.as_raw_fd() as usize,
            filter: libc::EVFILT_VNODE,
            flags: libc::EV_ADD | libc::EV_CLEAR,
            fflags: libc::NOTE_WRITE
                | libc::NOTE_EXTEND
                | libc::NOTE_ATTRIB
                | libc::NOTE_DELETE
                | libc::NOTE_RENAME
                | libc::NOTE_REVOKE,
            ..empty_event()
        };
        // SAFETY: one initialized change, no events asked for.
        let added = unsafe {
            libc::kevent(
                self.queue.as_raw_fd(),
                &change,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        (added >= 0).then_some(fd)
    }

    /// Resolves when something watched has changed since the last time;
    /// false once nothing can say so again.
    pub async fn changed(&mut self) -> bool {
        self.changed.recv().await.is_some()
    }
}

/// Whether an event on descriptor `fd` says something watched changed.
fn concerns(watches: &mut Watches, fd: i32) -> bool {
    match watches.get_mut(&fd) {
        None => false,
        Some(Watch::Any { .. }) => true,
        Some(Watch::Entries { dir, names, .. }) => {
            let mut any = false;
            for (name, last) in names {
                let now = stamp(&dir.join(&*name));
                any |= now != *last;
                *last = now;
            }
            any
        }
    }
}

/// What each directory's watched files look like now.
fn refresh(watches: &mut Watches) {
    for watch in watches.values_mut() {
        if let Watch::Entries { dir, names, .. } = watch {
            for (name, last) in names {
                *last = stamp(&dir.join(&*name));
            }
        }
    }
}

impl Drop for FileEvents {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        let wake = libc::kevent {
            ident: WAKE,
            filter: libc::EVFILT_USER,
            fflags: libc::NOTE_TRIGGER,
            ..empty_event()
        };
        // SAFETY: one initialized change to the queue this holds, no events
        // asked for.
        unsafe {
            libc::kevent(
                self.queue.as_raw_fd(),
                &wake,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
    }
}

#[cfg(test)]
mod tests {
    use super::FileEvents;
    use std::{
        future::Future,
        path::Path,
        pin::pin,
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
        time::{Duration, Instant},
    };

    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    /// Whether a change is reported within `limit`.
    fn heard(events: &mut FileEvents, limit: Duration) -> bool {
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut changed = pin!(events.changed());
        let until = Instant::now() + limit;
        loop {
            if let Poll::Ready(more) = changed.as_mut().poll(&mut cx) {
                return more;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            std::thread::park_timeout(left);
        }
    }

    /// Let a change's last events arrive (one write can be read as two).
    fn settle(events: &mut FileEvents) {
        while heard(events, Duration::from_millis(100)) {}
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("zgx-file-events-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SOON: Duration = Duration::from_secs(5);
    const QUIET: Duration = Duration::from_millis(200);

    #[test]
    fn writes_and_replacements_of_watched_files_are_heard_and_others_are_not() {
        let dir = scratch("files");
        let app = dir.join("app.luau");
        std::fs::write(&app, "return 1").unwrap();
        let mut events = FileEvents::new().unwrap();
        events.watch(vec![app.clone()], Vec::new(), false);

        // An editor's swap file beside it wakes nothing.
        std::fs::write(dir.join(".app.luau.swp"), "swap").unwrap();
        assert!(!heard(&mut events, QUIET));

        std::fs::write(&app, "return 2").unwrap();
        assert!(heard(&mut events, SOON));
        settle(&mut events);

        // Saved as write-and-rename: a new file under the old name.
        let temp = dir.join("app.luau.tmp");
        std::fs::write(&temp, "return 3").unwrap();
        assert!(!heard(&mut events, QUIET));
        std::fs::rename(&temp, &app).unwrap();
        assert!(heard(&mut events, SOON));
        settle(&mut events);
        events.watch(vec![app.clone()], Vec::new(), true);
        // The replacement's own writes are heard.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&app)
            .and_then(|mut f| std::io::Write::write_all(&mut f, b"\n"))
            .unwrap();
        assert!(heard(&mut events, SOON));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_watched_directory_hears_its_first_entry() {
        let dir = scratch("dirs");
        let plugins = dir.join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        let mut events = FileEvents::new().unwrap();
        events.watch(Vec::new(), vec![plugins.clone()], false);
        assert!(!heard(&mut events, QUIET));
        std::fs::write(plugins.join("first.luau"), "").unwrap();
        assert!(heard(&mut events, SOON));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_is_heard_once_unwatched() {
        let dir = scratch("unwatched");
        let (a, b) = (dir.join("a.luau"), dir.join("b").join("b.luau"));
        std::fs::create_dir(b.parent().unwrap()).unwrap();
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let mut events = FileEvents::new().unwrap();
        events.watch(vec![a.clone(), b.clone()], Vec::new(), false);
        events.watch(vec![a.clone()], Vec::new(), false);
        std::fs::write(&b, "changed").unwrap();
        assert!(!heard(&mut events, QUIET));
        std::fs::write(Path::new(&a), "changed").unwrap();
        assert!(heard(&mut events, SOON));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
