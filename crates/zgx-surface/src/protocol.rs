//! The messages between a host and a client, and their encoding.
//!
//! A message on the wire is a frame: its length (`u32`, of what follows
//! the length), its kind (`u16`), how many handles ride with it (`u16`),
//! then its payload. Integers and floats are little-endian; a string or a
//! list is its length (`u32`) then its items. Handles (file descriptors on
//! Unix, `HANDLE`s on Windows) go beside the bytes (`SCM_RIGHTS`, or
//! duplicated into the child); a payload names each by its place among the
//! frame's handles. A kind the reader doesn't know is skipped whole, so a
//! newer host or client may add messages; a version (`PROTOCOL_VERSION`)
//! says what each side speaks.
use crate::Handle;
use std::fmt;

/// The protocol both sides speak: `Hello` and `Welcome` carry it.
pub const PROTOCOL_VERSION: u32 = 1;
/// The largest frame either side accepts (a title, a key's code): anything
/// longer is a broken peer.
pub const MAX_FRAME: usize = 1 << 20;
/// Bytes before a frame's payload: its length, kind and handle count.
pub const HEADER: usize = 8;

/// How a buffer's pixels are shared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BufferKind {
    /// Shared memory (a memfd on Linux, a file mapping on Windows): the
    /// client writes pixels, the host uploads them. Every platform.
    Shm,
    /// A Vulkan image the host allocated, exported as an opaque file
    /// descriptor (`VK_KHR_external_memory_fd`), imported by a client on
    /// the same physical device. Linux.
    Vulkan,
    /// An `IOSurface` (macOS): reserved, not yet implemented.
    IoSurface,
    /// A DXGI shared handle (Windows): reserved, not yet implemented.
    Dxgi,
}
impl BufferKind {
    fn code(self) -> u8 {
        match self {
            BufferKind::Shm => 0,
            BufferKind::Vulkan => 1,
            BufferKind::IoSurface => 2,
            BufferKind::Dxgi => 3,
        }
    }
    fn from_code(code: u8) -> Result<Self, DecodeError> {
        Ok(match code {
            0 => BufferKind::Shm,
            1 => BufferKind::Vulkan,
            2 => BufferKind::IoSurface,
            3 => BufferKind::Dxgi,
            _ => return Err(DecodeError::Invalid("buffer kind")),
        })
    }
}

/// A physical GPU, as Vulkan names it (`VkPhysicalDeviceIDProperties`):
/// memory exported on one imports only on the same device and driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct DeviceIds {
    /// `deviceUUID`.
    pub device_uuid: [u8; 16],
    /// `driverUUID`.
    pub driver_uuid: [u8; 16],
}

/// What a client can take, in its order of preference (`Hello::offers`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Shared memory it writes pixels into.
    Shm,
    /// Vulkan images, on any of these devices (its adapters').
    Vulkan {
        /// Each device it could open.
        devices: Vec<DeviceIds>,
    },
    /// IOSurfaces (macOS), reserved.
    IoSurface,
    /// DXGI shared handles (Windows), on any of these adapters (LUIDs),
    /// reserved.
    Dxgi {
        /// Each adapter's LUID.
        adapters: Vec<u64>,
    },
}
impl Offer {
    /// The kind of buffer the offer takes.
    pub fn kind(&self) -> BufferKind {
        match self {
            Offer::Shm => BufferKind::Shm,
            Offer::Vulkan { .. } => BufferKind::Vulkan,
            Offer::IoSurface => BufferKind::IoSurface,
            Offer::Dxgi { .. } => BufferKind::Dxgi,
        }
    }
}

/// A client's first message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// The protocol it speaks (`PROTOCOL_VERSION`).
    pub version: u32,
    /// What it can take, best first.
    pub offers: Vec<Offer>,
}

/// The host's answer to `Hello`: how buffers will be shared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// The protocol the host speaks.
    pub version: u32,
    /// The kind of every buffer it sends.
    pub kind: BufferKind,
    /// With `Vulkan`, the device the buffers live on: the client draws on
    /// the adapter with these ids.
    pub device: Option<DeviceIds>,
}

/// A buffer's pixels: premultiplied alpha, rows top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// Blue, green, red, alpha; 8 bits each, unorm (zgx's own frames').
    Bgra8Unorm,
    /// Red, green, blue, alpha; 8 bits each, unorm.
    Rgba8Unorm,
}

/// A Vulkan image as the host made it: the client makes its own with the
/// same parameters (`VkImageCreateInfo`, its external memory info naming
/// `handle_type`, and a format list of `view_formats` if any) and imports
/// the memory into it. Raw Vulkan values, so neither side's bindings matter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VulkanImage {
    /// `VkFormat`.
    pub format: i32,
    /// Formats it may be viewed as (`VkImageFormatListCreateInfo`); empty
    /// for none.
    pub view_formats: Vec<i32>,
    /// `VkImageCreateFlags`.
    pub flags: u32,
    /// `VkImageUsageFlags`.
    pub usage: u32,
    /// `VkImageTiling`.
    pub tiling: i32,
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// `VkMemoryAllocateInfo::allocationSize` of the exported memory.
    pub allocation_size: u64,
    /// Its memory type's index (the same device, so the same types).
    pub memory_type_index: u32,
    /// Whether it is a dedicated allocation (`VkMemoryDedicatedAllocateInfo`).
    pub dedicated: bool,
    /// `VkExternalMemoryHandleTypeFlagBits`: the handle's type.
    pub handle_type: u32,
}

/// What a buffer is, beside its handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BufferDesc {
    /// Pixels at `offset` in the shared memory, `stride` bytes a row,
    /// `len` bytes in all.
    Shm {
        /// Where its first row starts.
        offset: u64,
        /// Bytes from one row to the next.
        stride: u32,
        /// The mapping's length.
        len: u64,
    },
    /// A Vulkan image.
    Vulkan(VulkanImage),
    /// An IOSurface by its global id (macOS), reserved.
    IoSurface {
        /// `IOSurfaceGetID`.
        id: u32,
    },
    /// A DXGI shared handle (Windows), reserved.
    Dxgi,
}

/// One buffer of a generation.
#[derive(Debug)]
pub struct Buffer {
    /// Its place in the ring: what `Present` and `Release` name.
    pub index: u32,
    /// Its memory: a file descriptor or handle. None for kinds named
    /// otherwise (an IOSurface's id).
    pub handle: Option<Handle>,
    /// What it is.
    pub desc: BufferDesc,
}

/// A ring of buffers at one size: the host makes a new generation for
/// each new size, and buffers of an older one are no longer shown.
#[derive(Debug)]
pub struct Generation {
    /// Counts up from 1 over the session.
    pub generation: u32,
    /// Width in device pixels.
    pub width: u32,
    /// Height in device pixels.
    pub height: u32,
    /// Their pixels' format.
    pub format: PixelFormat,
    /// The ring.
    pub buffers: Vec<Buffer>,
}

/// The modifier keys held during a key or pointer event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// Shift.
    pub shift: bool,
    /// Control.
    pub control: bool,
    /// Alt, or Option.
    pub alt: bool,
    /// Command on macOS, the Windows or Super key elsewhere.
    pub meta: bool,
}
impl Modifiers {
    fn bits(self) -> u8 {
        u8::from(self.shift)
            | u8::from(self.control) << 1
            | u8::from(self.alt) << 2
            | u8::from(self.meta) << 3
    }
    fn from_bits(bits: u8) -> Self {
        Self {
            shift: bits & 1 != 0,
            control: bits & 2 != 0,
            alt: bits & 4 != 0,
            meta: bits & 8 != 0,
        }
    }
}

/// A key going down or up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    /// The physical key, as the DOM's `KeyboardEvent.code` (`"KeyW"`,
    /// `"Space"`, `"ShiftLeft"`, `"Digit1"`): the same key whatever the
    /// layout. Empty when the platform didn't say.
    pub code: String,
    /// Down, or up.
    pub down: bool,
    /// Held and repeating.
    pub repeat: bool,
    /// The modifiers held.
    pub modifiers: Modifiers,
}

/// A pointer button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    /// The main button.
    Left,
    /// The context button.
    Right,
    /// The middle button or wheel press.
    Middle,
    /// The back side button.
    Back,
    /// The forward side button.
    Forward,
    /// Another, by the platform's number.
    Other(u16),
}

/// What a wheel's deltas count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WheelUnit {
    /// Lines (notches of a wheel).
    Lines,
    /// Device pixels (a trackpad, or a wheel the platform converted).
    Pixels,
}

/// The pointer's look over the surface (`ClientMessage::SetCursor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cursor {
    /// The arrow.
    Default,
    /// A pointing hand.
    Pointer,
    /// The I-beam.
    Text,
    /// A crosshair.
    Crosshair,
    /// Something can be moved.
    Move,
    /// An open hand.
    Grab,
    /// A closed hand.
    Grabbing,
    /// Not allowed.
    NotAllowed,
    /// Busy.
    Wait,
    /// Busy, but usable.
    Progress,
    /// Help.
    Help,
    /// Resizes left and right.
    EwResize,
    /// Resizes up and down.
    NsResize,
    /// Resizes top right to bottom left.
    NeswResize,
    /// Resizes top left to bottom right.
    NwseResize,
    /// No pointer shows (as CSS's `none`).
    Hidden,
}
const CURSORS: [Cursor; 16] = [
    Cursor::Default,
    Cursor::Pointer,
    Cursor::Text,
    Cursor::Crosshair,
    Cursor::Move,
    Cursor::Grab,
    Cursor::Grabbing,
    Cursor::NotAllowed,
    Cursor::Wait,
    Cursor::Progress,
    Cursor::Help,
    Cursor::EwResize,
    Cursor::NsResize,
    Cursor::NeswResize,
    Cursor::NwseResize,
    Cursor::Hidden,
];

/// What the host tells the client.
#[derive(Debug)]
pub enum HostMessage {
    /// The answer to `Hello`.
    Welcome(Welcome),
    /// A new ring of buffers, to draw into from now on.
    Buffers(Generation),
    /// The host no longer reads buffer `index` of `generation`: the
    /// client may draw into it again.
    Release {
        /// Its generation.
        generation: u32,
        /// Its index.
        index: u32,
    },
    /// The surface's size, in device pixels, and its device pixels a
    /// point. A generation at that size follows.
    Resize {
        /// Width.
        width: u32,
        /// Height.
        height: u32,
        /// Device pixels a point.
        scale: f32,
    },
    /// Whether any of it shows: a hidden surface's frames are not drawn,
    /// so a client should stop drawing them (zgx's rule: idle windows do no
    /// work).
    Visible(bool),
    /// It took (`true`) or lost keyboard focus.
    Focus(bool),
    /// A key went down or up while it had focus.
    Key(KeyEvent),
    /// Text typed (after the layout and dead keys) or an input method's
    /// commit: apart from keys, as the DOM's `input`.
    Text(String),
    /// The pointer moved to `x`, `y`, in device pixels from its top left.
    PointerMove {
        /// Across.
        x: f32,
        /// Down.
        y: f32,
    },
    /// The mouse moved by `dx`, `dy` while the pointer is locked: raw
    /// motion (about a point a count), as the DOM's `movementX`.
    PointerMotion {
        /// Across, positive to the right.
        dx: f32,
        /// Down, positive downward.
        dy: f32,
    },
    /// A button went down or up at `x`, `y` (device pixels).
    Button {
        /// Which.
        button: Button,
        /// Down, or up.
        down: bool,
        /// Across.
        x: f32,
        /// Down.
        y: f32,
    },
    /// A wheel or trackpad scroll: positive `dy` scrolls down (content
    /// moves up), positive `dx` right, as the DOM's `deltaY`, `deltaX`.
    Wheel {
        /// Across.
        dx: f32,
        /// Down.
        dy: f32,
        /// What they count.
        unit: WheelUnit,
    },
    /// The pointer lock began (`true`) or ended: the client asked
    /// (`LockPointer`), or Escape, a lost focus or the host let it go.
    PointerLock(bool),
    /// The pointer left the surface.
    PointerLeave,
    /// The display the surface shows on refreshes this many times a second:
    /// as it's welcomed, and again as it moves to another display. A client
    /// pacing itself draws at this rate (more would never show).
    Refresh(f32),
    /// The session is closing: finish and exit. The host kills a client
    /// still running a moment later.
    Close,
}

/// What the client tells the host.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMessage {
    /// Its first message.
    Hello(Hello),
    /// Buffer `index` of `generation` holds a finished frame (its GPU work
    /// done): show it. The client draws into it no more until `Release`.
    Present {
        /// Its generation.
        generation: u32,
        /// Its index.
        index: u32,
    },
    /// How the pointer looks over the surface.
    SetCursor(Cursor),
    /// Ask to lock the pointer (`true`; granted only just after a press or
    /// key, as the web's) or let it go. `PointerLock` says what happened.
    LockPointer(bool),
    /// Its title (a window's would be).
    Title(String),
}

/// Why a frame didn't decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// It ended early.
    Short,
    /// A field held what it can't.
    Invalid(&'static str),
    /// It named a handle the frame doesn't carry.
    MissingHandle,
    /// Longer than `MAX_FRAME`.
    TooLong,
}
impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Short => f.write_str("message ended early"),
            DecodeError::Invalid(what) => write!(f, "invalid {what}"),
            DecodeError::MissingHandle => f.write_str("message names a handle it doesn't carry"),
            DecodeError::TooLong => f.write_str("message too long"),
        }
    }
}
impl std::error::Error for DecodeError {}
impl From<DecodeError> for std::io::Error {
    fn from(error: DecodeError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, error)
    }
}

// Kinds on the wire. Client to host below 64, host to client from 64.
const HELLO: u16 = 1;
const PRESENT: u16 = 2;
const SET_CURSOR: u16 = 3;
const LOCK_POINTER: u16 = 4;
const TITLE: u16 = 5;
const WELCOME: u16 = 64;
const BUFFERS: u16 = 65;
const RELEASE: u16 = 66;
const RESIZE: u16 = 67;
const VISIBLE: u16 = 68;
const FOCUS: u16 = 69;
const KEY: u16 = 70;
const TEXT: u16 = 71;
const POINTER_MOVE: u16 = 72;
const POINTER_MOTION: u16 = 73;
const BUTTON: u16 = 74;
const WHEEL: u16 = 75;
const POINTER_LOCK: u16 = 76;
const POINTER_LEAVE: u16 = 77;
const CLOSE: u16 = 78;
const REFRESH: u16 = 79;

/// A frame being written: its bytes, and the handles that go with it.
pub struct Encoded<'a> {
    /// The frame, header included.
    pub bytes: Vec<u8>,
    /// Its handles, in the order the payload names them.
    pub handles: Vec<&'a Handle>,
}

struct Out<'a> {
    bytes: Vec<u8>,
    handles: Vec<&'a Handle>,
}
impl<'a> Out<'a> {
    fn new(kind: u16) -> Self {
        let mut bytes = Vec::with_capacity(32);
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&[0; 2]);
        Self {
            bytes,
            handles: Vec::new(),
        }
    }
    fn u8(&mut self, v: u8) {
        self.bytes.push(v);
    }
    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    fn u16(&mut self, v: u16) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, v: &str) {
        self.u32(v.len() as u32);
        self.bytes.extend_from_slice(v.as_bytes());
    }
    fn ids(&mut self, ids: &DeviceIds) {
        self.bytes.extend_from_slice(&ids.device_uuid);
        self.bytes.extend_from_slice(&ids.driver_uuid);
    }
    /// A handle: its place among the frame's, or `u16::MAX` for none.
    fn handle(&mut self, handle: Option<&'a Handle>) {
        match handle {
            Some(handle) => {
                self.u16(self.handles.len() as u16);
                self.handles.push(handle);
            }
            None => self.u16(u16::MAX),
        }
    }
    fn finish(mut self) -> Encoded<'a> {
        let len = (self.bytes.len() - 4) as u32;
        self.bytes[..4].copy_from_slice(&len.to_le_bytes());
        let count = self.handles.len() as u16;
        self.bytes[6..8].copy_from_slice(&count.to_le_bytes());
        Encoded {
            bytes: self.bytes,
            handles: self.handles,
        }
    }
}

struct In<'b> {
    bytes: &'b [u8],
    handles: Vec<Option<Handle>>,
}
impl In<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        if self.bytes.len() < N {
            return Err(DecodeError::Short);
        }
        let (head, rest) = self.bytes.split_at(N);
        self.bytes = rest;
        Ok(head.try_into().unwrap())
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take::<1>()?[0])
    }
    fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::Invalid("bool")),
        }
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.take()?))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.take()?))
    }
    fn len(&mut self) -> Result<usize, DecodeError> {
        let len = self.u32()? as usize;
        if len > self.bytes.len() {
            return Err(DecodeError::Short);
        }
        Ok(len)
    }
    fn str(&mut self) -> Result<String, DecodeError> {
        let len = self.len()?;
        let (head, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        String::from_utf8(head.to_vec()).map_err(|_| DecodeError::Invalid("string"))
    }
    fn ids(&mut self) -> Result<DeviceIds, DecodeError> {
        Ok(DeviceIds {
            device_uuid: self.take()?,
            driver_uuid: self.take()?,
        })
    }
    fn handle(&mut self) -> Result<Option<Handle>, DecodeError> {
        let at = self.u16()?;
        if at == u16::MAX {
            return Ok(None);
        }
        self.handles
            .get_mut(at as usize)
            .and_then(Option::take)
            .map(Some)
            .ok_or(DecodeError::MissingHandle)
    }
    /// A list's length, each item at least `min` bytes (so a forged length
    /// can't ask for more memory than the frame holds).
    fn count(&mut self, min: usize) -> Result<usize, DecodeError> {
        let count = self.u32()? as usize;
        if count.saturating_mul(min) > self.bytes.len() {
            return Err(DecodeError::Short);
        }
        Ok(count)
    }
}

/// A frame's header: its whole length (header included), kind and how
/// many handles it carries. Errors if the length is out of bounds.
pub fn header(bytes: &[u8; HEADER]) -> Result<(usize, u16, u16), DecodeError> {
    let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize + 4;
    if len > MAX_FRAME {
        return Err(DecodeError::TooLong);
    }
    if len < HEADER {
        return Err(DecodeError::Short);
    }
    let kind = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
    let handles = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
    Ok((len, kind, handles))
}

fn button_code(button: Button) -> u16 {
    match button {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
        Button::Back => 3,
        Button::Forward => 4,
        Button::Other(n) => 5u16.saturating_add(n),
    }
}
fn button_from(code: u16) -> Button {
    match code {
        0 => Button::Left,
        1 => Button::Right,
        2 => Button::Middle,
        3 => Button::Back,
        4 => Button::Forward,
        n => Button::Other(n - 5),
    }
}

impl HostMessage {
    /// The frame for this message.
    pub fn encode(&self) -> Encoded<'_> {
        let mut out;
        match self {
            HostMessage::Welcome(welcome) => {
                out = Out::new(WELCOME);
                out.u32(welcome.version);
                out.u8(welcome.kind.code());
                out.bool(welcome.device.is_some());
                if let Some(ids) = &welcome.device {
                    out.ids(ids);
                }
            }
            HostMessage::Buffers(generation) => {
                out = Out::new(BUFFERS);
                out.u32(generation.generation);
                out.u32(generation.width);
                out.u32(generation.height);
                out.u8(match generation.format {
                    PixelFormat::Bgra8Unorm => 0,
                    PixelFormat::Rgba8Unorm => 1,
                });
                out.u32(generation.buffers.len() as u32);
                for buffer in &generation.buffers {
                    out.u32(buffer.index);
                    out.handle(buffer.handle.as_ref());
                    match &buffer.desc {
                        BufferDesc::Shm {
                            offset,
                            stride,
                            len,
                        } => {
                            out.u8(0);
                            out.u64(*offset);
                            out.u32(*stride);
                            out.u64(*len);
                        }
                        BufferDesc::Vulkan(image) => {
                            out.u8(1);
                            out.i32(image.format);
                            out.u32(image.view_formats.len() as u32);
                            for format in &image.view_formats {
                                out.i32(*format);
                            }
                            out.u32(image.flags);
                            out.u32(image.usage);
                            out.i32(image.tiling);
                            out.u32(image.width);
                            out.u32(image.height);
                            out.u64(image.allocation_size);
                            out.u32(image.memory_type_index);
                            out.bool(image.dedicated);
                            out.u32(image.handle_type);
                        }
                        BufferDesc::IoSurface { id } => {
                            out.u8(2);
                            out.u32(*id);
                        }
                        BufferDesc::Dxgi => out.u8(3),
                    }
                }
            }
            HostMessage::Release { generation, index } => {
                out = Out::new(RELEASE);
                out.u32(*generation);
                out.u32(*index);
            }
            HostMessage::Resize {
                width,
                height,
                scale,
            } => {
                out = Out::new(RESIZE);
                out.u32(*width);
                out.u32(*height);
                out.f32(*scale);
            }
            HostMessage::Visible(visible) => {
                out = Out::new(VISIBLE);
                out.bool(*visible);
            }
            HostMessage::Focus(focused) => {
                out = Out::new(FOCUS);
                out.bool(*focused);
            }
            HostMessage::Key(key) => {
                out = Out::new(KEY);
                out.str(&key.code);
                out.bool(key.down);
                out.bool(key.repeat);
                out.u8(key.modifiers.bits());
            }
            HostMessage::Text(text) => {
                out = Out::new(TEXT);
                out.str(text);
            }
            HostMessage::PointerMove { x, y } => {
                out = Out::new(POINTER_MOVE);
                out.f32(*x);
                out.f32(*y);
            }
            HostMessage::PointerMotion { dx, dy } => {
                out = Out::new(POINTER_MOTION);
                out.f32(*dx);
                out.f32(*dy);
            }
            HostMessage::Button { button, down, x, y } => {
                out = Out::new(BUTTON);
                out.u16(button_code(*button));
                out.bool(*down);
                out.f32(*x);
                out.f32(*y);
            }
            HostMessage::Wheel { dx, dy, unit } => {
                out = Out::new(WHEEL);
                out.f32(*dx);
                out.f32(*dy);
                out.u8(match unit {
                    WheelUnit::Lines => 0,
                    WheelUnit::Pixels => 1,
                });
            }
            HostMessage::PointerLock(locked) => {
                out = Out::new(POINTER_LOCK);
                out.bool(*locked);
            }
            HostMessage::PointerLeave => out = Out::new(POINTER_LEAVE),
            HostMessage::Refresh(hz) => {
                out = Out::new(REFRESH);
                out.f32(*hz);
            }
            HostMessage::Close => out = Out::new(CLOSE),
        }
        out.finish()
    }

    /// The message a frame holds (`frame` from its header on), with the
    /// handles that came with it; `None` for a kind this version doesn't
    /// know, to skip.
    pub fn decode(frame: &[u8], handles: Vec<Handle>) -> Result<Option<Self>, DecodeError> {
        let (kind, mut r) = reader(frame, handles)?;
        let message = match kind {
            WELCOME => {
                let version = r.u32()?;
                let kind = BufferKind::from_code(r.u8()?)?;
                let device = if r.bool()? { Some(r.ids()?) } else { None };
                HostMessage::Welcome(Welcome {
                    version,
                    kind,
                    device,
                })
            }
            BUFFERS => {
                let generation = r.u32()?;
                let width = r.u32()?;
                let height = r.u32()?;
                let format = match r.u8()? {
                    0 => PixelFormat::Bgra8Unorm,
                    1 => PixelFormat::Rgba8Unorm,
                    _ => return Err(DecodeError::Invalid("pixel format")),
                };
                let count = r.count(7)?;
                let mut buffers = Vec::with_capacity(count);
                for _ in 0..count {
                    let index = r.u32()?;
                    let handle = r.handle()?;
                    let desc = match r.u8()? {
                        0 => BufferDesc::Shm {
                            offset: r.u64()?,
                            stride: r.u32()?,
                            len: r.u64()?,
                        },
                        1 => {
                            let format = r.i32()?;
                            let views = r.count(4)?;
                            let view_formats =
                                (0..views).map(|_| r.i32()).collect::<Result<_, _>>()?;
                            BufferDesc::Vulkan(VulkanImage {
                                format,
                                view_formats,
                                flags: r.u32()?,
                                usage: r.u32()?,
                                tiling: r.i32()?,
                                width: r.u32()?,
                                height: r.u32()?,
                                allocation_size: r.u64()?,
                                memory_type_index: r.u32()?,
                                dedicated: r.bool()?,
                                handle_type: r.u32()?,
                            })
                        }
                        2 => BufferDesc::IoSurface { id: r.u32()? },
                        3 => BufferDesc::Dxgi,
                        _ => return Err(DecodeError::Invalid("buffer")),
                    };
                    buffers.push(Buffer {
                        index,
                        handle,
                        desc,
                    });
                }
                HostMessage::Buffers(Generation {
                    generation,
                    width,
                    height,
                    format,
                    buffers,
                })
            }
            RELEASE => HostMessage::Release {
                generation: r.u32()?,
                index: r.u32()?,
            },
            RESIZE => HostMessage::Resize {
                width: r.u32()?,
                height: r.u32()?,
                scale: r.f32()?,
            },
            VISIBLE => HostMessage::Visible(r.bool()?),
            FOCUS => HostMessage::Focus(r.bool()?),
            KEY => HostMessage::Key(KeyEvent {
                code: r.str()?,
                down: r.bool()?,
                repeat: r.bool()?,
                modifiers: Modifiers::from_bits(r.u8()?),
            }),
            TEXT => HostMessage::Text(r.str()?),
            POINTER_MOVE => HostMessage::PointerMove {
                x: r.f32()?,
                y: r.f32()?,
            },
            POINTER_MOTION => HostMessage::PointerMotion {
                dx: r.f32()?,
                dy: r.f32()?,
            },
            BUTTON => HostMessage::Button {
                button: button_from(r.u16()?),
                down: r.bool()?,
                x: r.f32()?,
                y: r.f32()?,
            },
            WHEEL => HostMessage::Wheel {
                dx: r.f32()?,
                dy: r.f32()?,
                unit: match r.u8()? {
                    0 => WheelUnit::Lines,
                    1 => WheelUnit::Pixels,
                    _ => return Err(DecodeError::Invalid("wheel unit")),
                },
            },
            POINTER_LOCK => HostMessage::PointerLock(r.bool()?),
            POINTER_LEAVE => HostMessage::PointerLeave,
            REFRESH => HostMessage::Refresh(r.f32()?),
            CLOSE => HostMessage::Close,
            _ => return Ok(None),
        };
        Ok(Some(message))
    }
}

impl ClientMessage {
    /// The frame for this message (no client message carries handles).
    pub fn encode(&self) -> Encoded<'static> {
        let mut out;
        match self {
            ClientMessage::Hello(hello) => {
                out = Out::new(HELLO);
                out.u32(hello.version);
                out.u32(hello.offers.len() as u32);
                for offer in &hello.offers {
                    out.u8(offer.kind().code());
                    match offer {
                        Offer::Shm | Offer::IoSurface => {}
                        Offer::Vulkan { devices } => {
                            out.u32(devices.len() as u32);
                            for ids in devices {
                                out.ids(ids);
                            }
                        }
                        Offer::Dxgi { adapters } => {
                            out.u32(adapters.len() as u32);
                            for luid in adapters {
                                out.u64(*luid);
                            }
                        }
                    }
                }
            }
            ClientMessage::Present { generation, index } => {
                out = Out::new(PRESENT);
                out.u32(*generation);
                out.u32(*index);
            }
            ClientMessage::SetCursor(cursor) => {
                out = Out::new(SET_CURSOR);
                out.u8(CURSORS.iter().position(|c| c == cursor).unwrap() as u8);
            }
            ClientMessage::LockPointer(locked) => {
                out = Out::new(LOCK_POINTER);
                out.bool(*locked);
            }
            ClientMessage::Title(title) => {
                out = Out::new(TITLE);
                out.str(title);
            }
        }
        out.finish()
    }

    /// The message a frame holds, `None` for a kind this version doesn't
    /// know. Handles sent with it are dropped: no client message has any.
    pub fn decode(frame: &[u8], handles: Vec<Handle>) -> Result<Option<Self>, DecodeError> {
        let (kind, mut r) = reader(frame, handles)?;
        let message = match kind {
            HELLO => {
                let version = r.u32()?;
                let count = r.count(1)?;
                let mut offers = Vec::with_capacity(count);
                for _ in 0..count {
                    offers.push(match BufferKind::from_code(r.u8()?)? {
                        BufferKind::Shm => Offer::Shm,
                        BufferKind::IoSurface => Offer::IoSurface,
                        BufferKind::Vulkan => {
                            let count = r.count(32)?;
                            Offer::Vulkan {
                                devices: (0..count).map(|_| r.ids()).collect::<Result<_, _>>()?,
                            }
                        }
                        BufferKind::Dxgi => {
                            let count = r.count(8)?;
                            Offer::Dxgi {
                                adapters: (0..count).map(|_| r.u64()).collect::<Result<_, _>>()?,
                            }
                        }
                    });
                }
                ClientMessage::Hello(Hello { version, offers })
            }
            PRESENT => ClientMessage::Present {
                generation: r.u32()?,
                index: r.u32()?,
            },
            SET_CURSOR => ClientMessage::SetCursor(
                *CURSORS
                    .get(r.u8()? as usize)
                    .ok_or(DecodeError::Invalid("cursor"))?,
            ),
            LOCK_POINTER => ClientMessage::LockPointer(r.bool()?),
            TITLE => ClientMessage::Title(r.str()?),
            _ => return Ok(None),
        };
        Ok(Some(message))
    }
}

/// A frame's kind and a reader of its payload.
fn reader(frame: &[u8], handles: Vec<Handle>) -> Result<(u16, In<'_>), DecodeError> {
    let head: &[u8; HEADER] = frame
        .get(..HEADER)
        .and_then(|h| h.try_into().ok())
        .ok_or(DecodeError::Short)?;
    let (len, kind, _) = header(head)?;
    let payload = frame.get(HEADER..len).ok_or(DecodeError::Short)?;
    Ok((
        kind,
        In {
            bytes: payload,
            handles: handles.into_iter().map(Some).collect(),
        },
    ))
}
