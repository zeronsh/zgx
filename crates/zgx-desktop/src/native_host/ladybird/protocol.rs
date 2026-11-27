//! The messages between zgx and its Ladybird host (`zgx-ladybird-host`), over
//! one stream socket; the host's `Protocol.h` (crates/zgx-ladybird-sys/host)
//! is this file's twin: keep them in step.
//!
//! A message is a little-endian `u32` length (of what follows), a `u8` kind
//! and its fields: integers and floats as themselves, strings as a `u32` byte
//! count and UTF-8. A message carrying a file descriptor (`Store`) sends it
//! with its first byte (`SCM_RIGHTS`).

/// What zgx asks of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum ToHost {
    CreateView = 1,
    CloseView = 2,
    LoadUrl = 3,
    LoadHtml = 4,
    Reload = 5,
    Stop = 6,
    Traverse = 7,
    Resize = 8,
    Mouse = 9,
    Wheel = 10,
    Key = 11,
    Focus = 12,
    Done = 13,
    Zoom = 14,
    RunScript = 15,
    ImeMark = 16,
    ImeCommit = 17,
    ImeUnmark = 18,
    Visible = 19,
    Snapshot = 20,
    ReloadIgnoringCache = 21,
    Edit = 22,
    ClipboardData = 23,
}

/// A message to the host, written field by field.
pub(super) struct Message(Vec<u8>);
impl Message {
    pub(super) fn new(kind: ToHost, view: u64) -> Self {
        let mut bytes = Vec::with_capacity(32);
        bytes.extend_from_slice(&[0; 4]);
        bytes.push(kind as u8);
        bytes.extend_from_slice(&view.to_le_bytes());
        Self(bytes)
    }
    pub(super) fn u8(mut self, value: u8) -> Self {
        self.0.push(value);
        self
    }
    pub(super) fn u32(mut self, value: u32) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }
    pub(super) fn u64(mut self, value: u64) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }
    pub(super) fn i32(mut self, value: i32) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }
    pub(super) fn f64(mut self, value: f64) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }
    pub(super) fn str(mut self, value: &str) -> Self {
        self.0
            .extend_from_slice(&(value.len() as u32).to_le_bytes());
        self.0.extend_from_slice(value.as_bytes());
        self
    }
    /// The message's bytes, its length filled in.
    pub(super) fn finish(mut self) -> Vec<u8> {
        let length = (self.0.len() - 4) as u32;
        self.0[..4].copy_from_slice(&length.to_le_bytes());
        self.0
    }
}

/// What the host tells zgx.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum FromHost {
    Ready,
    /// A backing store of the view's (its descriptor came with it).
    Store {
        view: u64,
        id: i32,
        width: u32,
        height: u32,
        pitch: u32,
        format: u8,
        premultiplied: bool,
        size: u64,
    },
    /// The backing store `id` shows the view's new frame, changed in `damage`
    /// from the one before.
    Present {
        view: u64,
        id: i32,
        damage: (i32, i32, i32, i32),
        width: u32,
        height: u32,
        scale: f64,
    },
    Title {
        view: u64,
        title: String,
    },
    Url {
        view: u64,
        url: String,
    },
    Loading {
        view: u64,
        loading: bool,
    },
    LoadFinished {
        view: u64,
        url: String,
    },
    History {
        view: u64,
        back: bool,
        forward: bool,
    },
    Cursor {
        view: u64,
        cursor: u8,
    },
    NewWindow {
        view: u64,
        url: String,
    },
    /// WebContent ended: 0 crashed, 1 out of memory, 2 killed.
    Crashed {
        view: u64,
        reason: u8,
    },
    /// A page copied: the app's clipboard takes it.
    ClipboardWrite {
        text: String,
        html: String,
    },
    /// The page's icon: width, height, pitch, format, premultiplied and its
    /// bytes, or none.
    Favicon {
        view: u64,
        image: Option<(u32, u32, u32, u8, bool, Vec<u8>)>,
    },
    /// A page pastes: the app answers `request` with its clipboard
    /// (`ToHost::ClipboardData`).
    ClipboardRead {
        request: u64,
    },
    Editing {
        view: u64,
        editing: bool,
    },
    /// The link under the pointer; empty off links.
    LinkHovered {
        view: u64,
        url: String,
    },
    /// The page's theme colour, or none.
    ThemeColor {
        view: u64,
        color: Option<(u8, u8, u8, u8)>,
    },
    /// A snapshot asked for (`ToHost::Snapshot`): its buffer (whose
    /// descriptor came with it), or none.
    Snapshot {
        view: u64,
        request: u64,
        image: Option<SnapshotImage>,
    },
}

/// A snapshot's shared buffer, as the host describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SnapshotImage {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pitch: u32,
    pub(super) format: u8,
    pub(super) premultiplied: bool,
    pub(super) size: u64,
}

struct Fields<'a>(&'a [u8]);
impl Fields<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }
    fn i32(&mut self) -> Option<i32> {
        self.take().map(i32::from_le_bytes)
    }
    fn u64(&mut self) -> Option<u64> {
        self.take().map(u64::from_le_bytes)
    }
    fn f64(&mut self) -> Option<f64> {
        self.take().map(f64::from_le_bytes)
    }
    fn bytes(&mut self, length: usize) -> Option<Vec<u8>> {
        if self.0.len() < length {
            return None;
        }
        let (bytes, rest) = self.0.split_at(length);
        self.0 = rest;
        Some(bytes.to_vec())
    }
    fn str(&mut self) -> Option<String> {
        let length = self.u32()? as usize;
        if self.0.len() < length {
            return None;
        }
        let (text, rest) = self.0.split_at(length);
        self.0 = rest;
        Some(String::from_utf8_lossy(text).into_owned())
    }
}

/// The message of `kind` with `fields`; `None` for one this zgx doesn't know
/// or a malformed one (which the reader skips).
pub(super) fn parse(kind: u8, fields: &[u8]) -> Option<FromHost> {
    let mut f = Fields(fields);
    if kind == 101 {
        return Some(FromHost::Ready);
    }
    let view = f.u64()?;
    Some(match kind {
        102 => FromHost::Store {
            view,
            id: f.i32()?,
            width: f.u32()?,
            height: f.u32()?,
            pitch: f.u32()?,
            format: f.u8()?,
            premultiplied: f.u8()? != 0,
            size: f.u64()?,
        },
        103 => FromHost::Present {
            view,
            id: f.i32()?,
            damage: (f.i32()?, f.i32()?, f.i32()?, f.i32()?),
            width: f.u32()?,
            height: f.u32()?,
            scale: f.f64()?,
        },
        104 => FromHost::Title {
            view,
            title: f.str()?,
        },
        105 => FromHost::Url {
            view,
            url: f.str()?,
        },
        106 => FromHost::Loading {
            view,
            loading: f.u8()? != 0,
        },
        107 => FromHost::LoadFinished {
            view,
            url: f.str()?,
        },
        108 => FromHost::History {
            view,
            back: f.u8()? != 0,
            forward: f.u8()? != 0,
        },
        109 => FromHost::Cursor {
            view,
            cursor: f.u8()?,
        },
        110 => FromHost::NewWindow {
            view,
            url: f.str()?,
        },
        111 => FromHost::Crashed {
            view,
            reason: f.u8().unwrap_or(0),
        },
        116 => FromHost::ClipboardWrite {
            text: f.str()?,
            html: f.str()?,
        },
        117 => FromHost::ClipboardRead { request: f.u64()? },
        118 => FromHost::Favicon {
            view,
            image: if f.u8()? != 0 {
                let (width, height, pitch, format, premultiplied) =
                    (f.u32()?, f.u32()?, f.u32()?, f.u8()?, f.u8()? != 0);
                let length = f.u32()? as usize;
                let bytes = f.bytes(length)?;
                Some((width, height, pitch, format, premultiplied, bytes))
            } else {
                None
            },
        },
        112 => FromHost::Editing {
            view,
            editing: f.u8()? != 0,
        },
        115 => {
            let request = f.u64()?;
            let image = if f.u8()? != 0 {
                Some(SnapshotImage {
                    width: f.u32()?,
                    height: f.u32()?,
                    pitch: f.u32()?,
                    format: f.u8()?,
                    premultiplied: f.u8()? != 0,
                    size: f.u64()?,
                })
            } else {
                None
            };
            FromHost::Snapshot {
                view,
                request,
                image,
            }
        }
        113 => FromHost::LinkHovered {
            view,
            url: f.str()?,
        },
        114 => {
            let has = f.u8()? != 0;
            let color = (f.u8()?, f.u8()?, f.u8()?, f.u8()?);
            FromHost::ThemeColor {
                view,
                color: has.then_some(color),
            }
        }
        _ => return None,
    })
}

/// Whether `message` came with a file descriptor.
pub(super) fn carries_fd(message: &FromHost) -> bool {
    matches!(
        message,
        FromHost::Store { .. } | FromHost::Snapshot { image: Some(_), .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_its_length_kind_view_and_fields() {
        let bytes = Message::new(ToHost::LoadUrl, 7)
            .str("https://a.test/")
            .finish();
        assert_eq!(
            u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
            bytes.len() - 4
        );
        assert_eq!(bytes[4], 3);
        assert_eq!(u64::from_le_bytes(bytes[5..13].try_into().unwrap()), 7);
        assert_eq!(&bytes[17..], b"https://a.test/");
    }

    #[test]
    fn host_messages_parse_and_bad_ones_dont() {
        let mut fields = 9u64.to_le_bytes().to_vec();
        for value in [4i32, 1, 2, 30, 40] {
            fields.extend_from_slice(&value.to_le_bytes());
        }
        fields.extend_from_slice(&800u32.to_le_bytes());
        fields.extend_from_slice(&600u32.to_le_bytes());
        fields.extend_from_slice(&2f64.to_le_bytes());
        assert_eq!(
            parse(103, &fields),
            Some(FromHost::Present {
                view: 9,
                id: 4,
                damage: (1, 2, 30, 40),
                width: 800,
                height: 600,
                scale: 2.,
            })
        );
        assert_eq!(parse(103, &fields[..20]), None, "cut short");
        assert_eq!(parse(250, &fields), None, "unknown");
        let mut title = 9u64.to_le_bytes().to_vec();
        title.extend_from_slice(&3u32.to_le_bytes());
        title.extend_from_slice(b"Hi!");
        assert_eq!(
            parse(104, &title),
            Some(FromHost::Title {
                view: 9,
                title: "Hi!".into()
            })
        );
    }
}
