//! A PNG is kept as its file, so one whose header is good and whose pixels
//! aren't is found out as it is drawn, long after it was answered OK. It
//! draws nothing and stops nothing: no frame fails, nobody is woken to draw
//! it again, and what comes after it draws. (That it is tried only once is
//! `zgx::image`'s `a_broken_file_is_tried_once`.) Through the whole renderer
//! on wgpu's no-op backend (`ZGX_GPU_NOOP`, read from the environment, so
//! this file's one test has the process to itself).
use base64::Engine;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zgx::{
    compose::prelude::*,
    image::ImageData,
    scene::{NodeKind, Rect},
    terminal::{Terminal, TerminalOptions, TerminalRenderer, terminal},
    text_layout::FontFamily,
    theme::{ThemeOverride, tok},
    widgets::Ui,
};
use zgx_gpu::GpuRenderer;

/// A `side` by `side` PNG of noise, so its pixels are most of the file.
fn png(side: u32) -> Vec<u8> {
    let mut file = Vec::new();
    let mut encoder = png::Encoder::new(&mut file, side, side);
    encoder.set_color(png::ColorType::Rgba);
    let mut writer = encoder.write_header().unwrap();
    let mut seed = side;
    let pixels: Vec<u8> = (0..side * side * 4)
        .map(|_| {
            seed = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
            (seed >> 24) as u8
        })
        .collect();
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    file
}

fn kitty(keys: &str, file: &[u8]) -> Vec<u8> {
    let data = base64::engine::general_purpose::STANDARD.encode(file);
    format!("\x1b_G{keys},q=2;{data}\x1b\\").into_bytes()
}

fn pictures(ui: &Ui) -> Vec<Arc<ImageData>> {
    let scene = ui.scene.borrow();
    scene
        .paint_items()
        .filter_map(|item| match &item.kind {
            NodeKind::Image(image) => Some(image.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_png_with_bad_pixels_draws_nothing_and_stops_nothing() {
    // SAFETY: nothing else in this process reads or writes the environment
    // yet: this is its only test, and it hasn't started anything.
    unsafe { std::env::set_var("ZGX_GPU_NOOP", "1") };
    let wakes = Arc::new(AtomicUsize::new(0));
    let told = wakes.clone();
    zgx::image::set_decoded_wake(move || {
        told.fetch_add(1, Ordering::SeqCst);
    });
    let mut ui = Ui::new(800., 600.);
    let mut gpu = GpuRenderer::new(800, 600).expect("a renderer");
    let font = zgx_gpu::text::FontData::new(Arc::<[u8]>::from(zgx_workload::FONT_BYTES))
        .expect("the bundled font");
    font.install(&mut gpu.text_system().borrow_mut());
    gpu.install_text(&mut ui.scene.borrow_mut());
    let term = Terminal::new(&ui.runtime, TerminalOptions::default());
    let replay = zgx_terminal::replay_terminal(&term, ui.runtime.clone());
    ui.mount(provide(
        TerminalRenderer::new(zgx_terminal::terminal_view),
        theme_scope_with(
            || ThemeOverride::new().with(tok::FONT_MONO, FontFamily::from(zgx_workload::FONT)),
            terminal(&term).w(800.).h(600.),
        ),
    ));
    let whole = [Rect::new(0., 0., 800., 600.)];
    let frame = |ui: &Ui, gpu: &mut GpuRenderer| {
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &whole).expect("a frame");
    };
    // Waits for the worker to finish what it was given, in order, and tell
    // the host; draws as the host then does.
    let decoded = |ui: &Ui, gpu: &mut GpuRenderer, wakes_now: usize| {
        for _ in 0..1000 {
            if wakes.load(Ordering::SeqCst) >= wakes_now {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        assert_eq!(wakes.load(Ordering::SeqCst), wakes_now);
        assert!(gpu.images_decoded(), "a frame waited for it");
        frame(ui, gpu);
    };
    frame(&ui, &mut gpu);

    // Bad at each size: decoded off the drawing thread, and in the frame.
    let (large, small) = (png(400), png(64));
    let cut = |file: &[u8]| file[..file.len() * 3 / 4].to_vec();
    replay.feed(&kitty("a=T,f=100,c=8,r=4,i=1", &cut(&large)));
    replay.feed(&kitty("a=T,f=100,c=8,r=4,i=2", &cut(&small)));
    frame(&ui, &mut gpu);
    let bad = pictures(&ui);
    assert_eq!(bad.len(), 2, "both are placed");

    // A good one, given to the same worker after the bad one: decoded, it
    // is what wakes the host, once. The bad one, tried first, woke nobody.
    replay.feed(&kitty("a=T,f=100,c=8,r=4,i=3", &large));
    frame(&ui, &mut gpu);
    decoded(&ui, &mut gpu, 1);
    assert_eq!(pictures(&ui).len(), 3);
    assert_eq!(gpu.debug_cache_stats().image_textures, 1, "the good one");
    for image in &bad {
        assert_eq!(gpu.debug_image_size(image), None, "it draws nothing");
        assert!(image.pixels_at((64, 64)).is_none(), "it has no pixels");
    }

    // Frames go on with them in view, with nothing more to wait for.
    replay.feed(b"after them\r\n");
    for _ in 0..4 {
        frame(&ui, &mut gpu);
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(wakes.load(Ordering::SeqCst), 1, "no one woken for them");

    // Sent again under its id, whole, it draws where it was placed.
    replay.feed(&kitty("a=t,f=100,i=1", &large));
    frame(&ui, &mut gpu);
    decoded(&ui, &mut gpu, 2);
    assert_eq!(pictures(&ui).len(), 3);
    assert_eq!(gpu.debug_cache_stats().image_textures, 2);

    // Deleted, it goes like any other.
    replay.feed(b"\x1b_Ga=d,d=I,i=2,q=2\x1b\\");
    frame(&ui, &mut gpu);
    assert_eq!(pictures(&ui).len(), 2);
    term.close();
}
