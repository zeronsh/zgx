//! A Vulkan image exported by one device and imported by another, as a
//! host and its client do (two wgpu instances, so two `VkInstance`s and
//! `VkDevice`s, its memory's descriptor sent over the surface's socket):
//! what the client draws, the host reads, no CPU copy between. Skipped
//! where there is no Vulkan device that can share memory.
#![cfg(all(target_os = "linux", feature = "wgpu29"))]
use zgx_surface::{
    BufferDesc, Generation, HostMessage, PixelFormat, transport,
    wgpu29::{self, wgpu},
};

fn device(instance: &wgpu::Instance, adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
    let _ = instance;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap()
}

fn vulkan_instance() -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN;
    wgpu::Instance::new(desc)
}

#[test]
fn what_the_client_draws_the_host_reads() {
    let host_instance = vulkan_instance();
    let host_adapters =
        pollster::block_on(host_instance.enumerate_adapters(wgpu::Backends::VULKAN));
    let Some(host_adapter) = host_adapters
        .into_iter()
        .find(|a| a.get_info().device_type != wgpu::DeviceType::Cpu)
    else {
        eprintln!("skipped: no Vulkan GPU");
        return;
    };
    let (host_device, host_queue) = device(&host_instance, &host_adapter);
    if !wgpu29::can_share(&host_device) {
        eprintln!("skipped: the device can't export memory");
        return;
    }
    let ids = wgpu29::device_ids_of(&host_device).expect("a Vulkan device's ids");
    let size = (64, 32);
    let (host_texture, handle, desc) = wgpu29::export(&host_device, size.0, size.1).unwrap();

    // The handle crosses a real socket, as it would to another process.
    let ((mut host_tx, _host_rx), (_client_tx, mut client_rx)) = transport::pair().unwrap();
    host_tx
        .send_host(&HostMessage::Buffers(Generation {
            generation: 1,
            width: size.0,
            height: size.1,
            format: PixelFormat::Bgra8Unorm,
            buffers: vec![zgx_surface::Buffer {
                index: 0,
                handle: Some(handle),
                desc,
            }],
        }))
        .unwrap();
    let Some(HostMessage::Buffers(mut generation)) = client_rx.recv_host().unwrap() else {
        panic!("buffers");
    };
    let buffer = generation.buffers.remove(0);
    assert!(matches!(buffer.desc, BufferDesc::Vulkan(_)));

    // The client: its own instance, on the adapter with the host's ids.
    let client_instance = vulkan_instance();
    let client_adapters =
        pollster::block_on(client_instance.enumerate_adapters(wgpu::Backends::VULKAN));
    assert!(
        matches!(wgpu29::vulkan_offer(&client_adapters), Some(zgx_surface::Offer::Vulkan { devices }) if devices.contains(&ids))
    );
    let client_adapter = wgpu29::adapter_for(client_adapters, &ids).expect("the host's device");
    let (client_device, client_queue) = device(&client_instance, &client_adapter);
    let client_texture = unsafe { wgpu29::import(&client_device, buffer) }.unwrap();
    let mut encoder = client_device.create_command_encoder(&Default::default());
    {
        let view = client_texture.create_view(&Default::default());
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
    let submission = client_queue.submit([encoder.finish()]);
    client_device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();

    // The host reads its own texture: the client's pixels.
    let readback = wgpu29::Readback::new(&host_device, size.0, size.1);
    let mut encoder = host_device.create_command_encoder(&Default::default());
    readback.copy(&mut encoder, &host_texture);
    host_queue.submit([encoder.finish()]);
    let mut pixels = vec![0u8; (size.0 * size.1 * 4) as usize];
    readback
        .write(&host_device, &mut pixels, (size.0 * 4) as usize)
        .unwrap();
    // BGRA, unorm: 0.6, 0.4, 0.2 round to 153, 102, 51.
    for pixel in pixels.as_chunks::<4>().0 {
        assert_eq!(*pixel, [153, 102, 51, 255]);
    }
}
