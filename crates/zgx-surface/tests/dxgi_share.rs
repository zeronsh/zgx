//! Direct3D 12 textures shared with another process, as a host and its
//! client do: this test's binary is started again as the client (its
//! connection's pipes named in `ZGX_SURFACE_PIPE`, the textures' NT handles
//! duplicated into it), welcomed on the host's adapter by LUID, draws a
//! colour into the buffer it acquires with its own wgpu device and presents
//! it; the host reads that colour from its own texture, no CPU copy
//! between. WARP (CI's runner has no GPU) shares resources as a GPU does.
#![cfg(all(windows, feature = "wgpu29"))]
use std::{os::windows::io::AsRawHandle, sync::mpsc, time::Duration};
use zgx_surface::{
    BufferDesc, BufferKind, Client, ClientMessage, Generation, HostMessage, Offer, PixelFormat,
    protocol::{PROTOCOL_VERSION, Welcome},
    transport,
    wgpu29::{self, wgpu},
};

const SIZE: (u32, u32) = (64, 32);
/// What the client draws: BGRA unorm 0.6, 0.4, 0.2 round to 153, 102, 51.
const COLOUR: wgpu::Color = wgpu::Color {
    r: 0.2,
    g: 0.4,
    b: 0.6,
    a: 1.,
};
const CLIENT: &str = "dxgi_share::the_client_process";

fn dx12_adapters() -> (wgpu::Instance, Vec<wgpu::Adapter>) {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::DX12;
    let instance = wgpu::Instance::new(desc);
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
    (instance, adapters)
}

fn device(adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap()
}

/// The client, when this binary is started as one (else it has nothing to
/// do): offers Direct3D 12 on its adapters, draws `COLOUR` into a buffer of
/// the host's and presents it, then waits to be closed.
#[test]
fn the_client_process() {
    if !zgx_surface::in_surface() {
        return;
    }
    let (_instance, adapters) = dx12_adapters();
    let offer = wgpu29::dxgi_offer(&adapters).expect("a Direct3D 12 adapter");
    let mut client = Client::connect(vec![offer, Offer::Shm]).unwrap();
    let welcome = client.welcome().clone();
    assert_eq!(welcome.kind, BufferKind::Dxgi);
    let luid = welcome.adapter.expect("the host's adapter");
    let adapter = wgpu29::adapter_for_luid(adapters, luid).expect("the host's adapter");
    let (device, queue) = device(&adapter);
    let textures = loop {
        assert!(client.wait(Some(Duration::from_secs(30))), "buffers");
        if let Some(HostMessage::Buffers(generation)) = client.poll() {
            break generation
                .buffers
                .into_iter()
                .map(|buffer| {
                    assert_eq!(buffer.desc, BufferDesc::Dxgi);
                    // Safety: on the adapter the welcome named.
                    unsafe { wgpu29::import(&device, buffer) }.unwrap()
                })
                .collect::<Vec<_>>();
        }
    };
    let index = client.acquire().expect("a free buffer");
    let texture = &textures[index as usize];
    assert_eq!((texture.width(), texture.height()), SIZE);
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = texture.create_view(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(COLOUR),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    let submission = queue.submit([encoder.finish()]);
    // v1 shares no fence: the frame is finished before it's presented.
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    client.present(index).unwrap();
    while client.wait(Some(Duration::from_secs(30))) {
        if matches!(client.poll(), Some(HostMessage::Close)) {
            return;
        }
    }
}

#[test]
fn what_another_process_draws_the_host_reads() {
    let (_instance, adapters) = dx12_adapters();
    let Some(adapter) = adapters.into_iter().next() else {
        eprintln!("skipped: no Direct3D 12 adapter");
        return;
    };
    eprintln!("on {}", adapter.get_info().name);
    let (device, queue) = device(&adapter);
    let luid = wgpu29::device_luid_of(&device).expect("a Direct3D 12 device's LUID");
    assert_eq!(wgpu29::adapter_luid(&adapter), Some(luid));

    let spawning = transport::Spawning::new().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args([CLIENT, "--exact", "--nocapture", "--test-threads=1"]);
    spawning.prepare(&mut command);
    let mut child = command.spawn().unwrap();
    let (mut sender, mut receiver) = spawning.started(child.as_raw_handle()).unwrap();

    // On a thread of its own, so a client that never connects fails the
    // test instead of hanging it.
    let (done, finished) = mpsc::channel();
    let host = std::thread::spawn(move || {
        let hello = match receiver
            .recv_client()
            .unwrap()
            .expect("the client connects")
        {
            ClientMessage::Hello(hello) => hello,
            other => panic!("hello first, not {other:?}"),
        };
        assert_eq!(hello.version, PROTOCOL_VERSION);
        assert!(
            matches!(&hello.offers[0], Offer::Dxgi { adapters } if adapters.contains(&luid)),
            "the client offers the host's adapter: {:?}",
            hello.offers
        );
        sender
            .send_host(&HostMessage::Welcome(Welcome {
                version: PROTOCOL_VERSION,
                kind: BufferKind::Dxgi,
                device: None,
                adapter: Some(luid),
            }))
            .unwrap();
        let mut textures = Vec::new();
        let mut buffers = Vec::new();
        for index in 0..3 {
            let (texture, handle, desc) = wgpu29::export_dxgi(&device, SIZE.0, SIZE.1).unwrap();
            textures.push(texture);
            buffers.push(zgx_surface::Buffer {
                index,
                handle: Some(handle),
                desc,
            });
        }
        sender
            .send_host(&HostMessage::Buffers(Generation {
                generation: 1,
                width: SIZE.0,
                height: SIZE.1,
                format: PixelFormat::Bgra8Unorm,
                buffers,
            }))
            .unwrap();
        let index = loop {
            match receiver
                .recv_client()
                .unwrap()
                .expect("the client presents")
            {
                ClientMessage::Present { generation, index } => {
                    assert_eq!(generation, 1);
                    break index;
                }
                ClientMessage::Hello(_) => panic!("one hello"),
                _ => {}
            }
        };
        // The host reads its own texture: the client's pixels.
        let readback = wgpu29::Readback::new(&device, SIZE.0, SIZE.1);
        let mut encoder = device.create_command_encoder(&Default::default());
        readback.copy(&mut encoder, &textures[index as usize]);
        queue.submit([encoder.finish()]);
        let mut pixels = vec![0u8; (SIZE.0 * SIZE.1 * 4) as usize];
        readback
            .write(&device, &mut pixels, (SIZE.0 * 4) as usize)
            .unwrap();
        sender.send_host(&HostMessage::Close).unwrap();
        let _ = done.send(());
        pixels
    });
    if let Err(mpsc::RecvTimeoutError::Timeout) = finished.recv_timeout(Duration::from_secs(120)) {
        let _ = child.kill();
        panic!("the session didn't finish in two minutes");
    }
    let pixels = match host.join() {
        Ok(pixels) => pixels,
        Err(panic) => {
            let _ = child.kill();
            std::panic::resume_unwind(panic);
        }
    };
    for pixel in pixels.as_chunks::<4>().0 {
        assert_eq!(*pixel, [153, 102, 51, 255]);
    }
    let status = child.wait().unwrap();
    assert!(status.success(), "the client exits cleanly: {status}");
}
