//! A whole session over a socket pair, shared memory as the buffers: the
//! client's frames arrive in the host's mapping, buffers go round the ring
//! (presented, shown, released, acquired again), a frame replaced before it
//! shows goes back at once, and a resize starts a generation whose buffers
//! the client draws at the new size while the old generation's are
//! forgotten.
#![cfg(unix)]
use std::time::Duration;
use zgx_surface::{
    BufferDesc, BufferKind, Client, ClientMessage, Generation, HostMessage, Offer, PixelFormat,
    protocol::{PROTOCOL_VERSION, Welcome},
    ring::{HostRing, Slot},
    shm::Mapping,
    transport,
};

/// What the client drew into buffer `index`: its frame number, generation
/// and index in the first pixel, the rest the frame number.
fn pattern(frame: u32, generation: u32, index: u32) -> [u8; 4] {
    [frame as u8, generation as u8, index as u8, 255]
}

/// The client: draws a numbered frame into each buffer it acquires until
/// the host closes. Returns how many frames it drew of each generation.
fn client(sender: transport::Sender, receiver: transport::Receiver) -> Vec<(u32, u32)> {
    let mut client = Client::connect_over(sender, receiver, vec![Offer::Shm]).unwrap();
    assert_eq!(client.welcome().kind, BufferKind::Shm);
    let _ = client.set_title("shm test");
    let mut maps: Vec<(Mapping, u32)> = Vec::new();
    let mut size = (0, 0);
    let mut frame = 0;
    let mut drawn: Vec<(u32, u32)> = Vec::new();
    loop {
        while let Some(event) = client.poll() {
            match event {
                HostMessage::Buffers(generation) => {
                    size = (generation.width, generation.height);
                    maps = generation
                        .buffers
                        .into_iter()
                        .map(|buffer| {
                            let BufferDesc::Shm { stride, len, .. } = buffer.desc else {
                                panic!("shm buffers");
                            };
                            (
                                Mapping::map(&buffer.handle.unwrap(), len as usize).unwrap(),
                                stride,
                            )
                        })
                        .collect();
                    drawn.push((client.generation(), 0));
                }
                HostMessage::Close => return drawn,
                _ => {}
            }
        }
        if client.is_closed() {
            return drawn;
        }
        match client.acquire() {
            Some(index) if !maps.is_empty() => {
                frame += 1;
                let (map, stride) = &maps[index as usize];
                let bytes = unsafe { map.bytes_mut() };
                for y in 0..size.1 as usize {
                    for x in 0..size.0 as usize {
                        let at = y * *stride as usize + x * 4;
                        bytes[at..at + 4].copy_from_slice(&pattern(
                            frame,
                            client.generation(),
                            index,
                        ));
                    }
                }
                client.present(index).unwrap();
                drawn.last_mut().unwrap().1 += 1;
            }
            _ => {
                client.wait(Some(Duration::from_secs(5)));
            }
        }
    }
}

struct Host {
    sender: transport::Sender,
    receiver: transport::Receiver,
    ring: HostRing,
    maps: Vec<Mapping>,
    size: (u32, u32),
}
impl Host {
    fn generation(&mut self, size: (u32, u32)) {
        let generation = self.ring.next_generation(3);
        self.size = size;
        let len = (size.0 * size.1 * 4) as usize;
        let mut buffers = Vec::new();
        self.maps.clear();
        for index in 0..3 {
            let (handle, map) = Mapping::create(len).unwrap();
            self.maps.push(map);
            buffers.push(zgx_surface::Buffer {
                index,
                handle: Some(handle),
                desc: BufferDesc::Shm {
                    offset: 0,
                    stride: size.0 * 4,
                    len: len as u64,
                },
            });
        }
        self.sender
            .send_host(&HostMessage::Resize {
                width: size.0,
                height: size.1,
                scale: 1.,
            })
            .unwrap();
        self.sender
            .send_host(&HostMessage::Buffers(Generation {
                generation,
                width: size.0,
                height: size.1,
                format: PixelFormat::Bgra8Unorm,
                buffers,
            }))
            .unwrap();
    }
    /// The next present; a frame it replaced released.
    fn next_present(&mut self) -> Slot {
        loop {
            match self
                .receiver
                .recv_client()
                .unwrap()
                .expect("the client is connected")
            {
                ClientMessage::Present { generation, index } => {
                    let slot = Slot { generation, index };
                    if let Some(skipped) = self.ring.present(slot).expect("never presented twice") {
                        self.release(skipped);
                    }
                    return slot;
                }
                ClientMessage::Title(title) => assert_eq!(title, "shm test"),
                other => panic!("unexpected {other:?}"),
            }
        }
    }
    fn release(&mut self, slot: Slot) {
        self.sender
            .send_host(&HostMessage::Release {
                generation: slot.generation,
                index: slot.index,
            })
            .unwrap();
    }
    /// Show the newest frame: check its pixels, give it back (as the
    /// painter does once it has uploaded it). Its frame number.
    fn show(&mut self) -> Option<u32> {
        let slot = self.ring.take_newest()?;
        if slot.generation != self.ring.generation() {
            return None;
        }
        let bytes = unsafe { self.maps[slot.index as usize].bytes() };
        let first: [u8; 4] = bytes[..4].try_into().unwrap();
        assert_eq!(
            first[1..],
            pattern(0, slot.generation, slot.index)[1..],
            "the buffer presented"
        );
        let last = bytes.len() - 4;
        assert_eq!(bytes[last..], first, "the whole frame arrived");
        assert_eq!(bytes.len(), (self.size.0 * self.size.1 * 4) as usize);
        self.release(slot);
        Some(u32::from(first[0]))
    }
}

#[test]
fn frames_go_round_the_ring_and_a_resize_starts_a_generation() {
    let ((host_tx, host_rx), (client_tx, client_rx)) = transport::pair().unwrap();
    let client = std::thread::spawn(move || client(client_tx, client_rx));
    let mut host = Host {
        sender: host_tx,
        receiver: host_rx,
        ring: HostRing::new(),
        maps: Vec::new(),
        size: (0, 0),
    };
    let hello = match host.receiver.recv_client().unwrap().unwrap() {
        ClientMessage::Hello(hello) => hello,
        other => panic!("hello first, not {other:?}"),
    };
    assert_eq!(hello.version, PROTOCOL_VERSION);
    assert_eq!(hello.offers, vec![Offer::Shm]);
    host.sender
        .send_host(&HostMessage::Welcome(Welcome {
            version: PROTOCOL_VERSION,
            kind: BufferKind::Shm,
            device: None,
            adapter: None,
        }))
        .unwrap();
    host.generation((32, 16));

    // Ten frames shown one by one: each acquired buffer comes back.
    let mut last = 0;
    let mut indices = std::collections::HashSet::new();
    for _ in 0..10 {
        let slot = host.next_present();
        indices.insert(slot.index);
        let frame = host.show().expect("a frame of this generation");
        assert!(frame > last, "frames arrive in order");
        last = frame;
    }
    assert_eq!(indices.len(), 3, "every buffer went round");

    // Two presented before one shows: the first goes back unread.
    let first = host.next_present();
    let second = host.next_present();
    assert_ne!(first, second);
    assert_eq!(host.show(), Some(last + 2), "the newest shows");
    last += 2;

    // A new size: the client draws the new generation at its size.
    host.generation((8, 24));
    let generation = host.ring.generation();
    loop {
        let slot = host.next_present();
        if slot.generation == generation {
            break;
        }
        // A frame of the old generation, in flight: forgotten.
        assert!(host.show().is_none());
    }
    let frame = host.show().expect("the new generation's frame");
    assert!(frame > last);
    // And it keeps going round at the new size.
    for _ in 0..4 {
        host.next_present();
        host.show().unwrap();
    }

    host.sender.send_host(&HostMessage::Close).unwrap();
    let drawn = client.join().unwrap();
    assert_eq!(drawn.len(), 2, "two generations: {drawn:?}");
    assert_eq!(drawn[1].0, generation);
    assert!(drawn[0].1 >= 12 && drawn[1].1 >= 5, "{drawn:?}");
}

#[test]
fn the_client_hears_the_host_close() {
    let ((host_tx, host_rx), (client_tx, client_rx)) = transport::pair().unwrap();
    let client = std::thread::spawn(move || {
        let mut client = Client::connect_over(client_tx, client_rx, vec![Offer::Shm]).unwrap();
        // The connection ending is a close, once.
        let mut closes = 0;
        while client.wait(Some(Duration::from_secs(5))) {
            while let Some(event) = client.poll() {
                if matches!(event, HostMessage::Close) {
                    closes += 1;
                }
            }
        }
        assert!(client.is_closed());
        closes
    });
    let (mut sender, mut receiver) = (host_tx, host_rx);
    assert!(matches!(
        receiver.recv_client().unwrap(),
        Some(ClientMessage::Hello(_))
    ));
    sender
        .send_host(&HostMessage::Welcome(Welcome {
            version: PROTOCOL_VERSION,
            kind: BufferKind::Shm,
            device: None,
            adapter: None,
        }))
        .unwrap();
    drop(sender);
    drop(receiver);
    assert_eq!(client.join().unwrap(), 1);
}
