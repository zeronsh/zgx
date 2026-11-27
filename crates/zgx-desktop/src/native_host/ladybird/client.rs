//! The host process: started with the first Ladybird page, one for the app,
//! its messages read on a thread of their own (which sleeps in `recvmsg`
//! while pages are idle) and handed to the UI thread.

use super::protocol::{self, FromHost, Message, ToHost};
use std::{
    collections::HashMap,
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

/// One of a page's backing stores, mapped (read only) from the host's
/// shared memory.
pub(super) struct Mapping {
    pointer: *const u8,
    size: usize,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pitch: u32,
    pub(super) format: u8,
}
// SAFETY: a read-only mapping, unmapped once, when the last holder drops it.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}
impl Mapping {
    fn new(
        fd: OwnedFd,
        size: usize,
        width: u32,
        height: u32,
        pitch: u32,
        format: u8,
    ) -> Option<Self> {
        let needed = pitch as usize * height as usize;
        if size < needed || pitch < width * 4 || size == 0 {
            return None;
        }
        // SAFETY: mapping `size` bytes of the descriptor the host sent, read
        // only and shared; the descriptor may close after.
        let pointer = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if pointer == libc::MAP_FAILED {
            return None;
        }
        Some(Self {
            pointer: pointer as *const u8,
            size,
            width,
            height,
            pitch,
            format,
        })
    }
    /// Its bytes. Ladybird's Compositor doesn't paint into a backing store
    /// zgx has not released (`ToHost::Done`), so what this reads of the
    /// frame shown holds still.
    pub(super) fn bytes(&self) -> &[u8] {
        // SAFETY: the mapping is `size` bytes, alive while `self` is.
        unsafe { std::slice::from_raw_parts(self.pointer, self.size) }
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: the mapping made in `new`, unmapped once.
        unsafe { libc::munmap(self.pointer as *mut _, self.size) };
    }
}

/// A page's frame as the reader thread keeps it for the painter.
#[derive(Default)]
pub(super) struct Frame {
    stores: HashMap<i32, Arc<Mapping>>,
    /// The backing store showing the newest frame, and its id.
    pub(super) front: Option<(i32, Arc<Mapping>)>,
    /// What changed since the painter last uploaded, in the store's pixels.
    pub(super) damage: Option<(u32, u32, u32, u32)>,
    /// The UI thread was told and hasn't painted yet.
    pub(super) told: bool,
}

/// What the reader thread hands the UI thread.
pub(super) enum Delivery {
    /// A page's frame changed: paint it.
    Frame(u64),
    /// A snapshot's pixels (RGBA, premultiplied or not), or why there are
    /// none.
    Snapshot {
        view: u64,
        request: u64,
        image: Result<(u32, u32, Vec<u8>, bool), String>,
    },
    /// Anything else the host said.
    Message(FromHost),
    /// The host went away.
    Gone,
}

pub(super) struct Host {
    writer: Mutex<UnixStream>,
    pub(super) frames: Mutex<HashMap<u64, Arc<Mutex<Frame>>>>,
}

static HOST: OnceLock<Option<Arc<Host>>> = OnceLock::new();
static DELIVERIES: Mutex<Vec<Delivery>> = Mutex::new(Vec::new());
type Waker = Arc<dyn Fn() + Send + Sync>;
static WAKERS: Mutex<Vec<(u64, Waker)>> = Mutex::new(Vec::new());

/// The host's executable in an install of zgx-ladybird-sys's: the one
/// `$ZGX_LADYBIRD_DIR` names, else `zgx-ladybird` beside the app's
/// executable (where an app ships it); `bin/zgx-ladybird-host` in it
/// (`MacOS/` on macOS).
pub(super) fn executable() -> Option<PathBuf> {
    let dir = std::env::var_os("ZGX_LADYBIRD_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            let exe = std::env::current_exe().ok()?;
            Some(exe.parent()?.join("zgx-ladybird"))
        })?;
    let bin = if cfg!(target_os = "macos") {
        "MacOS"
    } else {
        "bin"
    };
    let path = dir.join(bin).join("zgx-ladybird-host");
    path.is_file().then_some(path)
}

/// The host, started now if it wasn't (`None` where there's no build of it,
/// or it couldn't start: pages are placeholders then).
pub(super) fn host() -> Option<Arc<Host>> {
    HOST.get_or_init(|| {
        let Some(path) = executable() else {
            eprintln!(
                "zgx: Ladybird web views unavailable: no zgx-ladybird-host in ZGX_LADYBIRD_DIR or zgx-ladybird beside the executable (crates/zgx-ladybird-sys builds one)"
            );
            return None;
        };
        match start(&path) {
            Ok(host) => Some(host),
            Err(error) => {
                eprintln!("zgx: Ladybird web views unavailable: {}: {error}", path.display());
                None
            }
        }
    })
    .clone()
}

/// The host's descriptor in the child: 3, as `--zgx-socket 3` says.
const CHILD_FD: RawFd = 3;

fn start(path: &std::path::Path) -> std::io::Result<Arc<Host>> {
    let (ours, theirs) = UnixStream::pair()?;
    let theirs_fd = theirs.as_raw_fd();
    let mut command = std::process::Command::new(path);
    command.args(["--zgx-socket", "3"]);
    // SAFETY: between fork and exec only async-signal-safe calls: dup2 the
    // host's end to fd 3, which leaves the copy open across exec; where it
    // already is fd 3 dup2 does nothing, so its close-on-exec is cleared.
    unsafe {
        command.pre_exec(move || {
            let result = if theirs_fd == CHILD_FD {
                libc::fcntl(CHILD_FD, libc::F_SETFD, 0)
            } else {
                libc::dup2(theirs_fd, CHILD_FD)
            };
            if result < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    drop(theirs);
    let reader = ours.try_clone()?;
    let host = Arc::new(Host {
        writer: Mutex::new(ours),
        frames: Mutex::new(HashMap::new()),
    });
    let read_host = host.clone();
    std::thread::Builder::new()
        .name("zgx-ladybird".into())
        .spawn(move || {
            read(reader, &read_host);
            let _ = child.wait();
            deliver(Delivery::Gone);
        })?;
    Ok(host)
}

impl Host {
    pub(super) fn send(&self, message: Message) {
        let bytes = message.finish();
        // A host gone reads nothing: its pages show what they last drew,
        // and `Delivery::Gone` says so.
        let _ = self.writer.lock().unwrap().write_all(&bytes);
    }
    /// The page `view`'s frame, shared with the reader thread.
    pub(super) fn frame(&self, view: u64) -> Arc<Mutex<Frame>> {
        self.frames.lock().unwrap().entry(view).or_default().clone()
    }
    pub(super) fn forget(&self, view: u64) {
        self.frames.lock().unwrap().remove(&view);
        self.send(Message::new(ToHost::CloseView, view));
    }
}

/// Read the host's messages until it goes away.
fn read(stream: UnixStream, host: &Host) {
    let mut pending: Vec<u8> = Vec::new();
    let mut fds: std::collections::VecDeque<OwnedFd> = Default::default();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let Some(read) = receive(&stream, &mut buffer, &mut fds) else {
            return;
        };
        pending.extend_from_slice(&buffer[..read]);
        let mut offset = 0;
        while pending.len() - offset >= 5 {
            let length =
                u32::from_le_bytes(pending[offset..offset + 4].try_into().unwrap()) as usize;
            if pending.len() - offset - 4 < length {
                break;
            }
            let kind = pending[offset + 4];
            if let Some(message) = protocol::parse(kind, &pending[offset + 5..offset + 4 + length])
            {
                let fd = if protocol::carries_fd(&message) {
                    fds.pop_front()
                } else {
                    None
                };
                take(host, message, fd);
            }
            offset += 4 + length;
        }
        pending.drain(..offset);
    }
}

/// One `recvmsg`: bytes into `buffer`, descriptors onto `fds`. `None` at the
/// end of the stream.
fn receive(
    stream: &UnixStream,
    buffer: &mut [u8],
    fds: &mut std::collections::VecDeque<OwnedFd>,
) -> Option<usize> {
    loop {
        let mut iov = libc::iovec {
            iov_base: buffer.as_mut_ptr() as *mut _,
            iov_len: buffer.len(),
        };
        // Room for a few descriptors a read.
        let mut control = [0u64; 16];
        // SAFETY: a zeroed msghdr pointing at our iovec and control buffer.
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr() as *mut _;
        message.msg_controllen = std::mem::size_of_val(&control) as _;
        // SAFETY: receiving into the buffers above.
        let n = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, 0) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return None;
        }
        // SAFETY: walking the control messages the kernel wrote.
        unsafe {
            let mut header = libc::CMSG_FIRSTHDR(&message);
            while !header.is_null() {
                if (*header).cmsg_level == libc::SOL_SOCKET
                    && (*header).cmsg_type == libc::SCM_RIGHTS
                {
                    let data = libc::CMSG_DATA(header) as *const RawFd;
                    let count = ((*header).cmsg_len as usize - libc::CMSG_LEN(0) as usize)
                        / std::mem::size_of::<RawFd>();
                    for i in 0..count {
                        fds.push_back(OwnedFd::from_raw_fd(data.add(i).read_unaligned()));
                    }
                }
                header = libc::CMSG_NXTHDR(&message, header);
            }
        }
        return (n > 0).then_some(n as usize);
    }
}

/// A message, on the reader thread: frames are kept here, the rest goes to
/// the UI thread.
fn take(host: &Host, message: FromHost, fd: Option<OwnedFd>) {
    match message {
        FromHost::Store {
            view,
            id,
            width,
            height,
            pitch,
            format,
            size,
            ..
        } => {
            let Some(fd) = fd else { return };
            let Some(mapping) = Mapping::new(fd, size as usize, width, height, pitch, format)
            else {
                return;
            };
            let frame = host.frame(view);
            let mut frame = frame.lock().unwrap();
            frame.stores.insert(id, Arc::new(mapping));
            // A resize makes new stores under new ids: keep a few.
            if frame.stores.len() > 8 {
                let front = frame.front.as_ref().map(|(id, _)| *id);
                let mut ids: Vec<i32> = frame
                    .stores
                    .keys()
                    .copied()
                    .filter(|i| Some(*i) != front && *i != id)
                    .collect();
                ids.sort_unstable();
                for old in ids.into_iter().take(frame.stores.len() - 8) {
                    frame.stores.remove(&old);
                }
            }
        }
        FromHost::Present {
            view, id, damage, ..
        } => {
            let frame = host.frame(view);
            let tell = {
                let mut frame = frame.lock().unwrap();
                let Some(store) = frame.stores.get(&id).cloned() else {
                    return;
                };
                let resized = frame.front.as_ref().is_none_or(|(_, front)| {
                    (front.width, front.height) != (store.width, store.height)
                });
                let (x, y, w, h) = damage;
                let rect = if resized {
                    (0, 0, store.width, store.height)
                } else {
                    clip(x, y, w, h, store.width, store.height)
                };
                frame.damage = union(frame.damage, rect);
                frame.front = Some((id, store));
                !std::mem::replace(&mut frame.told, true)
            };
            if tell {
                deliver(Delivery::Frame(view));
            }
        }
        FromHost::Snapshot {
            view,
            request,
            image,
        } => {
            let image = match (image, fd) {
                (Some(image), Some(fd)) => Mapping::new(
                    fd,
                    image.size as usize,
                    image.width,
                    image.height,
                    image.pitch,
                    image.format,
                )
                .map(|mapping| {
                    (
                        image.width,
                        image.height,
                        rgba(&mapping),
                        image.premultiplied && matches!(image.format, 2 | 4),
                    )
                })
                .ok_or_else(|| "Ladybird's snapshot couldn't be read".to_owned()),
                _ => Err("Ladybird took no snapshot of the page".to_owned()),
            };
            deliver(Delivery::Snapshot {
                view,
                request,
                image,
            });
        }
        other => deliver(Delivery::Message(other)),
    }
}

/// A store's pixels as RGBA rows without padding: Ladybird's BGR stores
/// swapped, and those without alpha (BGRx, RGBx) opaque.
pub(super) fn rgba(store: &Mapping) -> Vec<u8> {
    rgba_rows(
        store.bytes(),
        store.width,
        store.height,
        store.pitch,
        store.format,
    )
}

/// Ladybird's pixels, `pitch` bytes a row in `format`, as RGBA rows without
/// padding (`rgba`); empty where `bytes` is too short for them.
pub(super) fn rgba_rows(bytes: &[u8], width: u32, height: u32, pitch: u32, format: u8) -> Vec<u8> {
    let (width, height, pitch) = (width as usize, height as usize, pitch as usize);
    if pitch < width * 4 || bytes.len() < pitch * height.saturating_sub(1) + width * 4 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        for pixel in bytes[row * pitch..row * pitch + width * 4]
            .as_chunks::<4>()
            .0
        {
            let (r, b) = if matches!(format, 1 | 2) {
                (pixel[2], pixel[0])
            } else {
                (pixel[0], pixel[2])
            };
            let a = if matches!(format, 1 | 3) {
                255
            } else {
                pixel[3]
            };
            out.extend_from_slice(&[r, pixel[1], b, a]);
        }
    }
    out
}

fn clip(x: i32, y: i32, w: i32, h: i32, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let x0 = x.clamp(0, width as i32) as u32;
    let y0 = y.clamp(0, height as i32) as u32;
    let x1 = (x.saturating_add(w)).clamp(0, width as i32) as u32;
    let y1 = (y.saturating_add(h)).clamp(0, height as i32) as u32;
    (x0, y0, x1 - x0, y1 - y0)
}

fn union(a: Option<(u32, u32, u32, u32)>, b: (u32, u32, u32, u32)) -> Option<(u32, u32, u32, u32)> {
    if b.2 == 0 || b.3 == 0 {
        return a;
    }
    let Some(a) = a else { return Some(b) };
    let x0 = a.0.min(b.0);
    let y0 = a.1.min(b.1);
    let x1 = (a.0 + a.2).max(b.0 + b.2);
    let y1 = (a.1 + a.3).max(b.1 + b.3);
    Some((x0, y0, x1 - x0, y1 - y0))
}

fn deliver(delivery: Delivery) {
    DELIVERIES.lock().unwrap().push(delivery);
    let waker = WAKERS
        .lock()
        .unwrap()
        .first()
        .map(|(_, waker)| waker.clone());
    if let Some(waker) = waker {
        waker();
    }
}

/// What the reader thread delivered since the last call, in order.
pub(super) fn take_deliveries() -> Vec<Delivery> {
    std::mem::take(&mut *DELIVERIES.lock().unwrap())
}

/// Give back deliveries taken but not for the taker, ahead of newer ones.
#[cfg(test)]
pub(super) fn put_back(deliveries: Vec<Delivery>) {
    if deliveries.is_empty() {
        return;
    }
    let mut queue = DELIVERIES.lock().unwrap();
    let newer = std::mem::replace(&mut *queue, deliveries);
    queue.extend(newer);
}

/// Keeps a window's waker among those deliveries wake, until dropped.
pub(crate) struct WakerGuard(u64);
impl Drop for WakerGuard {
    fn drop(&mut self) {
        WAKERS.lock().unwrap().retain(|(id, _)| *id != self.0);
    }
}
/// Wake a window with `wake` when the host's thread delivers.
pub(crate) fn add_waker(wake: Waker) -> WakerGuard {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    WAKERS.lock().unwrap().push((id, wake));
    WakerGuard(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_is_clipped_to_the_store_and_unites() {
        assert_eq!(clip(-5, 10, 20, 500, 100, 200), (0, 10, 15, 190));
        assert_eq!(clip(150, 0, 10, 10, 100, 200), (100, 0, 0, 10));
        assert_eq!(union(None, (1, 2, 3, 4)), Some((1, 2, 3, 4)));
        assert_eq!(
            union(Some((0, 0, 10, 10)), (20, 5, 10, 10)),
            Some((0, 0, 30, 15))
        );
        assert_eq!(
            union(Some((0, 0, 10, 10)), (20, 5, 0, 10)),
            Some((0, 0, 10, 10)),
            "empty adds nothing"
        );
    }
}
