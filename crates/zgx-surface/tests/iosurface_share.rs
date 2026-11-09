//! IOSurfaces between a host and its client (macOS): their Mach ports over
//! the session's Mach channel, only to the session's own process (its token,
//! its pid and pid version), malformed messages destroyed whole; and a
//! client process (this test binary, started again with one test) drawing
//! with wgpu Metal into IOSurfaces the host allocated, which the host reads
//! back with no CPU copy between.
#![cfg(target_os = "macos")]
use std::time::Duration;
use zgx_surface::iosurface::{ClientChannel, HostChannel, Surface};

/// A session whose program is this process, checked in.
fn connected() -> (HostChannel, ClientChannel) {
    let host = HostChannel::new().unwrap();
    host.started(std::process::id());
    let client = ClientChannel::connect(host.name(), &host.token()).unwrap();
    host.accept(Duration::from_secs(2)).unwrap();
    assert!(host.is_connected());
    (host, client)
}

#[test]
fn a_generation_s_surfaces_cross_the_mach_channel_in_order() {
    let (host, client) = connected();
    let old = Surface::new_bgra(8, 8).unwrap();
    host.send(1, &[&old]).unwrap();
    let surfaces: Vec<Surface> = (0..3)
        .map(|i| Surface::new_bgra(64 + i, 32).unwrap())
        .collect();
    host.send(2, &surfaces.iter().collect::<Vec<_>>()).unwrap();
    // Generation 1's message, never asked for, is dropped on the way.
    let got = client.receive(2, Duration::from_secs(2)).unwrap();
    assert_eq!(
        got.iter().map(Surface::id).collect::<Vec<_>>(),
        surfaces.iter().map(Surface::id).collect::<Vec<_>>()
    );
    assert_eq!((got[2].width(), got[2].height()), (66, 32));
    // Nothing more waits.
    assert!(client.receive(3, Duration::from_millis(50)).is_err());
}

#[test]
fn a_check_in_reaches_only_the_session_whose_token_it_carries() {
    let host = HostChannel::new().unwrap();
    host.started(std::process::id());
    let other = HostChannel::new().unwrap();
    // Another session's token, and one no session has.
    let _theirs = ClientChannel::connect(host.name(), &other.token()).unwrap();
    let _no_one = ClientChannel::connect(host.name(), &"0".repeat(32)).unwrap();
    assert!(host.accept(Duration::from_millis(200)).is_err());
    assert!(!host.is_connected());
    // Timed out, the session's token is forgotten: its own check-in, late,
    // reaches no one.
    let _late = ClientChannel::connect(host.name(), &host.token()).unwrap();
    assert!(host.accept(Duration::from_millis(200)).is_err());
}

#[test]
fn a_check_in_from_another_process_than_the_one_started_is_refused() {
    let host = HostChannel::new().unwrap();
    // Started as another process (a pid past macOS's largest): this one,
    // holding the token as if it had read the child's environment, is not it.
    host.started(u32::MAX - 1);
    let _client = ClientChannel::connect(host.name(), &host.token()).unwrap();
    assert!(host.accept(Duration::from_millis(200)).is_err());
    assert!(!host.is_connected());
}

#[test]
fn an_exited_program_s_session_takes_no_check_in() {
    let host = HostChannel::new().unwrap();
    host.started(std::process::id());
    host.ended();
    let _client = ClientChannel::connect(host.name(), &host.token()).unwrap();
    assert!(host.accept(Duration::from_millis(200)).is_err());
}

/// Mach, as the test needs it beside the crate's own.
mod raw {
    pub type Port = u32;
    unsafe extern "C" {
        pub static bootstrap_port: Port;
        pub static mach_task_self_: Port;
        pub fn bootstrap_look_up(bp: Port, name: *const std::ffi::c_char, port: *mut Port) -> i32;
        pub fn mach_port_allocate(task: Port, right: u32, name: *mut Port) -> i32;
        pub fn mach_port_mod_refs(task: Port, name: Port, right: u32, delta: i32) -> i32;
        pub fn mach_port_deallocate(task: Port, name: Port) -> i32;
        pub fn mach_msg(
            msg: *mut u32,
            option: i32,
            send_size: u32,
            rcv_size: u32,
            rcv_name: Port,
            timeout: u32,
            notify: Port,
        ) -> i32;
    }
}

#[test]
fn a_malformed_check_in_is_destroyed_whole_and_the_rendezvous_goes_on() {
    use raw::*;
    let host = HostChannel::new().unwrap();
    host.started(std::process::id());
    let token: Vec<u32> = (0..4)
        .map(|i| u32::from_str_radix(&host.token()[i * 8..i * 8 + 8], 16).unwrap())
        .collect();
    let name = std::ffi::CString::new(host.name()).unwrap();
    // Safety: plain Mach calls on ports this test owns.
    unsafe {
        let task = mach_task_self_;
        let mut service = 0;
        assert_eq!(
            bootstrap_look_up(bootstrap_port, name.as_ptr(), &mut service),
            0
        );
        let mut port = 0;
        assert_eq!(mach_port_allocate(task, 1, &mut port), 0);
        // A check-in's shape, but its right a send-once one (made from
        // `port`), which a check-in never carries: destroying the message
        // destroys that right, and the kernel tells `port` so
        // (MACH_NOTIFY_SEND_ONCE). Kept, no notice would come.
        let mut message = vec![
            19 | 0x8000_0000, // MACH_MSGH_BITS(COPY_SEND, 0) | COMPLEX
            56,
            service,
            0,
            0,
            0x7a67_7301,
            1,
            port,
            0,
            21 << 16, // MACH_MSG_TYPE_MAKE_SEND_ONCE, a port descriptor
        ];
        message.extend(&token);
        assert_eq!(
            mach_msg(message.as_mut_ptr(), 1 | 0x10, 56, 0, 0, 2000, 0),
            0
        );
        mach_port_deallocate(task, service);
        let mut notice = vec![0u32; 64];
        let received = mach_msg(notice.as_mut_ptr(), 2 | 0x100, 0, 256, port, 2000, 0);
        assert_eq!(received, 0, "no notice: the send-once right was kept");
        assert_eq!(notice[5], 0o107, "MACH_NOTIFY_SEND_ONCE");
        mach_port_mod_refs(task, port, 1, -1);
    }
    // The rendezvous still takes the session's real check-in.
    let _client = ClientChannel::connect(host.name(), &host.token()).unwrap();
    host.accept(Duration::from_secs(2)).unwrap();
}

#[cfg(feature = "wgpu29")]
mod gpu {
    use super::*;
    use zgx_surface::{
        BufferKind, Client, ClientMessage, Generation, HostMessage, Offer, PixelFormat,
        protocol::{PROTOCOL_VERSION, Welcome},
        transport,
        wgpu29::{self, wgpu},
    };

    /// Set in the client process this test starts.
    const CLIENT: &str = "ZGX_SURFACE_TEST_CLIENT";
    const SIZE: (u32, u32) = (64, 32);
    const RING: u32 = 3;

    fn metal_instance() -> wgpu::Instance {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::METAL;
        wgpu::Instance::new(desc)
    }

    /// Kills the client if the host's side of the test fails first.
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn what_a_client_process_draws_into_an_iosurface_the_host_reads() {
        let instance = metal_instance();
        let Some(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()
        else {
            eprintln!("skipped: no Metal adapter");
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        assert!(wgpu29::can_share_iosurface(&device));
        let ids = wgpu29::iosurface_ids_of(&device).unwrap();

        // Start this binary again as the client, with the socket and the
        // Mach channel as `App::spawn` gives a program.
        let spawning = transport::Spawning::new().unwrap();
        let mach = HostChannel::new().unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "iosurface_share::gpu::the_client_process",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CLIENT, "1");
        spawning.prepare(&mut command);
        mach.prepare(&mut command);
        let child = Child(command.spawn().unwrap());
        mach.started(child.0.id());
        let (mut tx, mut rx) = spawning.started().unwrap();

        let Some(ClientMessage::Hello(hello)) = rx.recv_client().unwrap() else {
            panic!("the client's hello");
        };
        let registry_id = ids.metal_registry_id().unwrap();
        assert!(
            matches!(&hello.offers[0], Offer::IoSurface { devices } if devices.contains(&registry_id)),
            "{:?}",
            hello.offers
        );
        mach.accept(Duration::from_secs(5)).unwrap();
        tx.send_host(&HostMessage::Welcome(Welcome {
            version: PROTOCOL_VERSION,
            kind: BufferKind::IoSurface,
            device: Some(ids),
            adapter: None,
        }))
        .unwrap();

        let mut textures = Vec::new();
        let mut surfaces = Vec::new();
        let mut buffers = Vec::new();
        for index in 0..RING {
            let (texture, surface, desc) =
                wgpu29::export_iosurface(&device, SIZE.0, SIZE.1).unwrap();
            textures.push(texture);
            surfaces.push(surface);
            buffers.push(zgx_surface::Buffer {
                index,
                handle: None,
                desc,
            });
        }
        // The ports first, then the `Buffers` naming them (as the painter).
        mach.send(1, &surfaces.iter().collect::<Vec<_>>()).unwrap();
        tx.send_host(&HostMessage::Buffers(Generation {
            generation: 1,
            width: SIZE.0,
            height: SIZE.1,
            format: PixelFormat::Bgra8Unorm,
            buffers,
        }))
        .unwrap();

        let index = loop {
            match rx.recv_client().unwrap() {
                Some(ClientMessage::Present {
                    generation: 1,
                    index,
                }) => break index,
                Some(_) => {}
                None => panic!("the client ended before presenting"),
            }
        };

        // The host reads its own texture over the surface: the client's pixels.
        let readback = wgpu29::Readback::new(&device, SIZE.0, SIZE.1);
        let mut encoder = device.create_command_encoder(&Default::default());
        readback.copy(&mut encoder, &textures[index as usize]);
        queue.submit([encoder.finish()]);
        let mut pixels = vec![0u8; (SIZE.0 * SIZE.1 * 4) as usize];
        readback
            .write(&device, &mut pixels, (SIZE.0 * 4) as usize)
            .unwrap();
        // BGRA, unorm: 0.6, 0.4, 0.2 round to 153, 102, 51.
        for pixel in pixels.as_chunks::<4>().0 {
            assert_eq!(*pixel, [153, 102, 51, 255]);
        }

        tx.send_host(&HostMessage::Close).unwrap();
        tx.shutdown();
        let mut child = child;
        let status = child.0.wait().unwrap();
        assert!(status.success(), "the client failed: {status}");
    }

    /// The client half, in the process the test above starts; nothing when
    /// run otherwise.
    #[test]
    fn the_client_process() {
        if std::env::var_os(CLIENT).is_none() {
            return;
        }
        let instance = metal_instance();
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::METAL));
        let mut offers: Vec<Offer> = wgpu29::iosurface_offer(&adapters).into_iter().collect();
        offers.push(Offer::Shm);
        let mut client = Client::connect(offers).unwrap();
        assert_eq!(client.welcome().kind, BufferKind::IoSurface);
        let ids = client.welcome().device.unwrap();
        let adapter = wgpu29::adapter_for(adapters, &ids).expect("the host's Metal device");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        assert_eq!(wgpu29::iosurface_ids_of(&device), Some(ids));

        let generation = loop {
            match client.poll() {
                Some(HostMessage::Buffers(generation)) => break generation,
                Some(HostMessage::Close) => panic!("closed before buffers"),
                Some(_) => {}
                None => assert!(client.wait(Some(Duration::from_secs(10))), "no buffers"),
            }
        };
        assert_eq!((generation.width, generation.height), SIZE);
        let textures: Vec<wgpu::Texture> = generation
            .buffers
            .into_iter()
            // Safety: on the adapter the welcome named.
            .map(|buffer| unsafe { wgpu29::import(&device, buffer) }.unwrap())
            .collect();
        let index = client.acquire().unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let view = textures[index as usize].create_view(&Default::default());
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.2,
                            g: 0.4,
                            b: 0.6,
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        let submission = queue.submit([encoder.finish()]);
        // v1 shares no event: the frame is finished before it's presented.
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        client.present(index).unwrap();
        // Until the host is done with it.
        loop {
            match client.poll() {
                Some(HostMessage::Close) => return,
                Some(_) => {}
                None => {
                    if !client.wait(Some(Duration::from_secs(10))) {
                        return;
                    }
                }
            }
        }
    }
}
