//! Every message encodes and decodes to itself; a frame the reader doesn't
//! know is skipped, and a broken one is an error, never a panic.
use zgx_surface::{
    BufferDesc, BufferKind, Button, ClientMessage, Cursor, DeviceIds, Generation, HostMessage,
    KeyEvent, Modifiers, Offer, PixelFormat, VulkanImage, WheelUnit,
    protocol::{DecodeError, Hello, Welcome},
};

fn host_round_trip(message: HostMessage) -> HostMessage {
    let encoded = message.encode();
    assert!(encoded.handles.is_empty(), "{message:?} carries no handle");
    HostMessage::decode(&encoded.bytes, Vec::new())
        .expect("decodes")
        .expect("a known kind")
}

#[test]
fn host_messages_survive_their_encoding() {
    let ids = DeviceIds {
        device_uuid: [7; 16],
        driver_uuid: [9; 16],
    };
    let messages = vec![
        HostMessage::Welcome(Welcome {
            version: 1,
            kind: BufferKind::Vulkan,
            device: Some(ids),
        }),
        HostMessage::Welcome(Welcome {
            version: 1,
            kind: BufferKind::Shm,
            device: None,
        }),
        HostMessage::Release {
            generation: 4,
            index: 2,
        },
        HostMessage::Resize {
            width: 1920,
            height: 1080,
            scale: 1.25,
        },
        HostMessage::Visible(false),
        HostMessage::Focus(true),
        HostMessage::Key(KeyEvent {
            code: "KeyW".into(),
            down: true,
            repeat: true,
            modifiers: Modifiers {
                shift: true,
                control: false,
                alt: true,
                meta: false,
            },
        }),
        HostMessage::Text("é漢😀".into()),
        HostMessage::PointerMove { x: 12.5, y: -3. },
        HostMessage::PointerMotion { dx: -4., dy: 2.5 },
        HostMessage::Button {
            button: Button::Back,
            down: false,
            x: 1.,
            y: 2.,
        },
        HostMessage::Button {
            button: Button::Other(9),
            down: true,
            x: 0.,
            y: 0.,
        },
        HostMessage::Wheel {
            dx: 0.,
            dy: -120.,
            unit: WheelUnit::Pixels,
        },
        HostMessage::Wheel {
            dx: 1.,
            dy: 0.,
            unit: WheelUnit::Lines,
        },
        HostMessage::PointerLock(true),
        HostMessage::PointerLeave,
        HostMessage::Close,
    ];
    for message in messages {
        let expected = format!("{message:?}");
        assert_eq!(format!("{:?}", host_round_trip(message)), expected);
    }
}

#[test]
fn client_messages_survive_their_encoding() {
    let messages = vec![
        ClientMessage::Hello(Hello {
            version: 1,
            offers: vec![
                Offer::Vulkan {
                    devices: vec![
                        DeviceIds {
                            device_uuid: [1; 16],
                            driver_uuid: [2; 16],
                        },
                        DeviceIds::default(),
                    ],
                },
                Offer::Dxgi { adapters: vec![42] },
                Offer::IoSurface {
                    devices: vec![0x1_0000_0abc],
                },
                Offer::Shm,
            ],
        }),
        ClientMessage::Present {
            generation: 3,
            index: 1,
        },
        ClientMessage::SetCursor(Cursor::Hidden),
        ClientMessage::SetCursor(Cursor::Default),
        ClientMessage::LockPointer(true),
        ClientMessage::Title("Horizon — 1.21".into()),
    ];
    for message in messages {
        let encoded = message.encode();
        let decoded = ClientMessage::decode(&encoded.bytes, Vec::new())
            .unwrap()
            .unwrap();
        assert_eq!(decoded, message);
    }
}

#[cfg(unix)]
#[test]
fn a_generation_carries_its_handles_in_order() {
    use std::os::fd::AsRawFd;
    let (a, _) = zgx_surface::shm::Mapping::create(64).unwrap();
    let (b, _) = zgx_surface::shm::Mapping::create(128).unwrap();
    let image = VulkanImage {
        format: 44,
        view_formats: vec![44, 50],
        flags: 8,
        usage: 0x17,
        tiling: 0,
        width: 640,
        height: 480,
        allocation_size: 1 << 20,
        memory_type_index: 3,
        dedicated: true,
        handle_type: 1,
    };
    let message = HostMessage::Buffers(Generation {
        generation: 2,
        width: 640,
        height: 480,
        format: PixelFormat::Bgra8Unorm,
        buffers: vec![
            zgx_surface::Buffer {
                index: 0,
                handle: Some(a),
                desc: BufferDesc::Shm {
                    offset: 0,
                    stride: 16,
                    len: 64,
                },
            },
            zgx_surface::Buffer {
                index: 1,
                handle: None,
                desc: BufferDesc::IoSurface {
                    id: 77,
                    surface: None,
                },
            },
            zgx_surface::Buffer {
                index: 2,
                handle: Some(b),
                desc: BufferDesc::Vulkan(image.clone()),
            },
        ],
    });
    let encoded = message.encode();
    assert_eq!(encoded.handles.len(), 2);
    let raw: Vec<i32> = encoded.handles.iter().map(|h| h.as_raw_fd()).collect();
    let handles = encoded
        .handles
        .iter()
        .map(|h| h.try_clone().unwrap())
        .collect();
    let HostMessage::Buffers(generation) = HostMessage::decode(&encoded.bytes, handles)
        .unwrap()
        .unwrap()
    else {
        panic!("not buffers");
    };
    assert_eq!(
        (generation.generation, generation.width, generation.height),
        (2, 640, 480)
    );
    assert!(generation.buffers[1].handle.is_none());
    assert_eq!(generation.buffers[2].desc, BufferDesc::Vulkan(image));
    // Each handle is the one its buffer named: the same open file.
    let inode = |fd: i32| {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        unsafe { libc::fstat(fd, &mut stat) };
        stat.st_ino
    };
    let got = |i: usize| inode(generation.buffers[i].handle.as_ref().unwrap().as_raw_fd());
    assert_eq!(got(0), inode(raw[0]));
    assert_eq!(got(2), inode(raw[1]));
}

#[test]
fn an_unknown_kind_is_skipped_and_a_broken_frame_is_an_error() {
    // A kind from a newer protocol: length 4 (kind, count), no payload.
    let unknown = [4, 0, 0, 0, 0xEE, 0x0E, 0, 0];
    assert!(HostMessage::decode(&unknown, Vec::new()).unwrap().is_none());
    assert!(
        ClientMessage::decode(&unknown, Vec::new())
            .unwrap()
            .is_none()
    );

    let whole = HostMessage::Text("hello".into()).encode().bytes;
    // Cut short: the length says more than there is.
    assert_eq!(
        HostMessage::decode(&whole[..whole.len() - 2], Vec::new()).unwrap_err(),
        DecodeError::Short
    );
    // A list claiming more items than the frame could hold.
    let mut forged = ClientMessage::Hello(Hello {
        version: 1,
        offers: vec![],
    })
    .encode()
    .bytes;
    forged[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        ClientMessage::decode(&forged, Vec::new()).unwrap_err(),
        DecodeError::Short
    );
    // A buffer naming a handle that didn't come.
    let mut buffers = HostMessage::Buffers(Generation {
        generation: 1,
        width: 1,
        height: 1,
        format: PixelFormat::Rgba8Unorm,
        buffers: vec![zgx_surface::Buffer {
            index: 0,
            handle: None,
            desc: BufferDesc::Dxgi,
        }],
    })
    .encode()
    .bytes;
    // The buffer's handle slot follows its index.
    let at = 8 + 4 + 4 + 4 + 1 + 4 + 4;
    buffers[at..at + 2].copy_from_slice(&0u16.to_le_bytes());
    assert_eq!(
        HostMessage::decode(&buffers, Vec::new()).unwrap_err(),
        DecodeError::MissingHandle
    );
    // Longer than any frame may be.
    let mut huge = whole.clone();
    huge[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(zgx_surface::protocol::header(huge[..8].try_into().unwrap()).is_err());
}
