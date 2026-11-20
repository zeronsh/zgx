//! What a window spends presenting a splitter drag: a sidebar and two panes,
//! each a conversation of about 80 rows (wrapping paragraphs, tables, code,
//! rounded cards with shadows, inline images), the divider between the panes
//! moved 6 points a frame. Per frame, offscreen at a Retina scale: the
//! scene's layout and damage (`flush`), the renderer's encoding (`render`),
//! the GPU's time (macOS), and what the frame drew: damaged pixels against
//! the window, draws, instances, passes, glyphs added to the atlas and
//! image uploads.
//!
//! ```sh
//! cargo run --release -p zgx-desktop --example drag_gpu_bench
//! ```
//! `TICKS` sets the frames (240), `SCALE` the device scale (2), `ROWS` the
//! rows a pane holds (80), `STEP` the points a frame (6).
use std::sync::Arc;
use zgx::{compose::prelude::*, image::ImageData, scene::BoxShadow, scene::Color, widgets::Ui};
use zgx_gpu::GpuRenderer;

#[path = "support/headless_font.rs"]
mod headless_font;

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

const PARAGRAPH: &str = "The width of a pane changes as its divider is dragged, so each paragraph \
breaks its lines again at the new measure, while the sidebar, the other pane's chrome and the \
images keep their places; what the window draws is what changed.";

fn gradient(seed: u8) -> Arc<ImageData> {
    let (w, h) = (240u32, 160u32);
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.extend([
                (x * 255 / w) as u8,
                (y * 255 / h) as u8,
                seed.wrapping_add(((x + y) % 64) as u8),
                255,
            ]);
        }
    }
    Arc::new(ImageData::new(w, h, pixels).unwrap())
}

fn card() -> View {
    column()
        .w_full()
        .p(12.)
        .rounded(12.)
        .bg(rgba(0xffffff14))
        .border(1.)
        .border_color(rgba(0xffffff1f))
        .shadow(BoxShadow {
            color: Color(0, 0, 0, 90),
            offset: zgx::scene::Transform { x: 0., y: 4. },
            blur_radius: 8.,
            spread: 0.,
        })
}

fn row_of(kind: usize, i: usize, images: &[Arc<ImageData>]) -> View {
    match kind % 5 {
        0 | 1 => text(format!("{i}. {PARAGRAPH}"))
            .text_size(14.)
            .text_wrap(true)
            .w_full(),
        2 => {
            let cells = (0..18).map(move |c| {
                div()
                    .p(6.)
                    .border(1.)
                    .border_color(rgba(0xffffff26))
                    .child(text(format!("r{} c{}", c / 3, c % 3)).text_size(12.))
            });
            card().child(
                div()
                    .w_full()
                    .grid()
                    .grid_cols(3)
                    .rounded(6.)
                    .overflow_hidden()
                    .children(cells),
            )
        }
        3 => column()
            .w_full()
            .p(10.)
            .rounded(8.)
            .bg(rgba(0x00000066))
            .child(
                text(
                    "fn drag(width: f32) -> Layout { relayout(width) }\nlet rows = visible(width);",
                )
                .text_size(12.)
                .font_family("Menlo"),
            ),
        _ => card().child(
            image("picture", images[i % images.len()].clone())
                .w(240.)
                .h(160.)
                .rounded(8.)
                .overflow_hidden(),
        ),
    }
}

fn pane(name: &'static str, rows: usize, images: Vec<Arc<ImageData>>, ui: &Ui) -> View {
    let offset = ui.signal(0.);
    column()
        .min_w(0.)
        .h_full()
        .rounded(12.)
        .bg(rgba(0x12161ea6))
        .child(
            row()
                .w_full()
                .h(44.)
                .px(14.)
                .items_center()
                .gap(8.)
                .child(text(name).text_size(14.))
                .child(div().w(70.).h(26.).rounded(13.).bg(rgba(0xffffff1a)))
                .child(div().w(26.).h(26.).rounded(13.).bg(rgba(0xffffff1a))),
        )
        .child(
            scroll(offset).grow().w_full().child(
                column()
                    .w_full()
                    .p(16.)
                    .gap(14.)
                    .children((0..rows).map(move |i| row_of(i * 7 / 3, i, &images))),
            ),
        )
}

fn main() {
    let ticks: usize = env("TICKS", 240);
    let scale: f32 = env("SCALE", 2.);
    let rows: usize = env("ROWS", 80);
    let step: f32 = env("STEP", 6.);
    let (width, height) = (1600., 1000.);
    let mut ui = Ui::new(width, height);
    let size = ((width * scale) as u32, (height * scale) as u32);
    let layer = cfg!(target_os = "macos") && zgx::env::var("PRESENT").as_deref() != Ok("headless");
    #[cfg(target_os = "macos")]
    let mut gpu = if layer {
        GpuRenderer::debug_for_layer(objc2_quartz_core::CALayer::new(), size.0, size.1)
    } else {
        GpuRenderer::new(size.0, size.1)
    }
    .expect("GPU");
    #[cfg(not(target_os = "macos"))]
    let mut gpu = GpuRenderer::new(size.0, size.1).expect("GPU");
    gpu.set_scale_factor(scale);
    headless_font::font().install(&mut gpu.text_system().borrow_mut());
    gpu.install_text(&mut ui.scene.borrow_mut());
    let images: Vec<Arc<ImageData>> = (0..6).map(|i| gradient(i * 40)).collect();
    let split = ui.signal(520.);
    let (a, b) = (split.clone(), split.clone());
    let sidebar = column()
        .w(260.)
        .h_full()
        .shrink_0()
        .p(12.)
        .gap(2.)
        .children((0..40).map(|i| {
            row()
                .h(34.)
                .px(10.)
                .rounded(8.)
                .items_center()
                .child(text(format!("Conversation {i}")).text_size(13.))
        }));
    let left =
        pane("Left", rows, images.clone(), &ui).reactive_style(move || Styles::new().w(a.get()));
    let right = pane("Right", rows, images, &ui)
        .reactive_style(move || Styles::new().w(1600. - 260. - 12. - b.get() - 12.));
    ui.mount(
        row()
            .w_full()
            .h_full()
            .gap(12.)
            .p(0.)
            .text_color(rgb(0xffffff))
            .child(sidebar)
            .child(left)
            .child(right),
    );
    let mut direction = 1.;
    let (mut flush, mut render, mut gpu_times) = (Vec::new(), Vec::new(), Vec::new());
    let (mut damaged, mut draws, mut instances, mut passes) = (0u64, 0usize, 0usize, 0usize);
    let (mut atlas, mut uploads) = (0usize, 0usize);
    let mut last = gpu.debug_cache_stats();
    let (warm, mut measured) = (30, 0usize);
    for tick in 0..ticks + warm {
        if tick > 0 {
            let now = split.get() + direction * step;
            if !(380. ..=880.).contains(&now) {
                direction = -direction;
            }
            split.set(split.get() + direction * step);
        }
        let t0 = std::time::Instant::now();
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        let t1 = std::time::Instant::now();
        let stats = gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
        #[cfg(target_os = "macos")]
        if layer {
            gpu.present().unwrap();
        }
        let t2 = std::time::Instant::now();
        #[cfg(target_os = "macos")]
        let time = gpu.take_gpu_time();
        #[cfg(not(target_os = "macos"))]
        let time: Option<std::time::Duration> = None;
        let now = gpu.debug_cache_stats();
        if tick >= warm {
            measured += 1;
            flush.push((t1 - t0).as_secs_f64() * 1e3);
            render.push((t2 - t1).as_secs_f64() * 1e3);
            gpu_times.extend(time.map(|t| t.as_secs_f64() * 1e3));
            damaged += stats.damaged_pixels;
            draws += stats.draw_calls;
            instances += stats.instances;
            passes += stats.render_passes;
            atlas += now.atlas_entries.saturating_sub(last.atlas_entries);
            uploads += now.image_textures.saturating_sub(last.image_textures);
        }
        last = now;
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v.get(v.len() / 2).copied().unwrap_or(0.)
    };
    let p90 = |v: &Vec<f64>| v.get((v.len() * 9) / 10).copied().unwrap_or(0.);
    let (fm, rm, gm) = (
        median(&mut flush),
        median(&mut render),
        median(&mut gpu_times),
    );
    let n = measured as f64;
    println!(
        "drag: layout+damage {fm:.2} ms median ({:.2} p90), render+present CPU {rm:.2} ms ({:.2} p90), GPU {gm:.2} ms ({:.2} p90) over {} frames; \
         damaged {:.2} of {:.2} Mpx, {:.1} draws, {:.0} instances, {:.1} passes a frame; {:.2} glyphs and {:.2} image textures added a frame",
        p90(&flush),
        p90(&render),
        p90(&gpu_times),
        measured,
        damaged as f64 / n / 1e6,
        (size.0 as f64 * size.1 as f64) / 1e6,
        draws as f64 / n,
        instances as f64 / n,
        passes as f64 / n,
        atlas as f64 / n,
        uploads as f64 / n,
    );
}
