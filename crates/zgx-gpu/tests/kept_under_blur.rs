//! A window with a backdrop blur always on screen (a chat composer's frosted
//! glass over the bottom of a streamed reply): each frame is pixel for pixel
//! what drawing the scene afresh draws, whether its damage lies under the
//! glass, in reach of it or far from it.
//!
//! Today such a window keeps no frame to patch (`render_once` keeps none
//! where the scene has a backdrop blur), so every frame here is drawn whole
//! within the blur's reach. Kept frames under a blur (docs/ROADMAP.md) patch
//! them: `streamed_frames_under_the_glass_patch` says so once they do.
use zgx::{compose::prelude::*, reactive::Signal, scene::Color, widgets::Ui};
use zgx_gpu::{GpuRenderer, text::FontData};

const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");
const SIZE: (f32, f32) = (420., 300.);
/// The reply's text, streamed a chunk at a time.
const REPLY: &str = "The reply arrives a few words at a time, each chunk an append, \
    and the page shows it as it comes, line after line under the frosted composer \
    until it reaches the end of what was sent.";
const CHUNK: usize = 24;

/// The reply as it stands after `step` chunks.
fn streamed(step: usize) -> String {
    REPLY[..(step * CHUNK).min(REPLY.len())].to_owned()
}

/// Earlier paragraphs, the streamed one under the composer's glass, and a
/// status pill at the top whose colour changes each step (damage far from
/// the glass).
fn view(reply: Signal<String>, status: Signal<usize>) -> View {
    let earlier = (0..3).map(|index| {
        div().px(12.).py(4.).child(
            text(format!(
                "Paragraph {index}: earlier text, drawn once and left as it is."
            ))
            .text_size(13.),
        )
    });
    let pill = div()
        .absolute()
        .ml(300.)
        .mt(8.)
        .size(100., 18.)
        .rounded(9.)
        .reactive_style(move || {
            let shade = (status.get() * 37 % 200) as u8;
            Styles::new().bg(Color(40 + shade / 2, 90, 200 - shade / 3, 255))
        });
    let transcript = column().children(earlier).child(div().h(120.)).child(
        div()
            .px(12.)
            .py(4.)
            .child(text_signal(move || reply.get()).text_size(13.)),
    );
    let composer = div()
        .absolute()
        .ml(20.)
        .mt(SIZE.1 - 96.)
        .size(SIZE.0 - 40., 84.)
        .rounded(16.)
        .bg(Color(240, 240, 245, 150))
        .backdrop_blur(9.);
    div()
        .size(SIZE.0, SIZE.1)
        .bg(Color(20, 24, 32, 255))
        .text_color(Color(232, 236, 242, 255))
        .font_family("Geist")
        .child(transcript)
        .child(pill)
        .child(composer)
}

struct Window {
    ui: Ui,
    gpu: GpuRenderer,
    reply: Signal<String>,
    status: Signal<usize>,
}
impl Window {
    /// The window as it stands after `step` chunks and status changes.
    fn new(step: usize) -> Self {
        let mut gpu = GpuRenderer::new((SIZE.0 * 2.) as u32, (SIZE.1 * 2.) as u32).unwrap();
        gpu.set_scale_factor(2.);
        FontData::new(GEIST.to_vec())
            .unwrap()
            .install(&mut gpu.text_system().borrow_mut());
        let mut ui = Ui::new(SIZE.0, SIZE.1);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let reply = ui.signal(streamed(step));
        let status = ui.signal(step);
        ui.mount(view(reply.clone(), status.clone()));
        Window {
            ui,
            gpu,
            reply,
            status,
        }
    }
    fn draw(&mut self) -> zgx_gpu::GpuStats {
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap()
    }
    /// The next step: a chunk of the reply under the glass, the status pill
    /// far from it, or both.
    fn step(&mut self, step: usize, what: Change) -> zgx_gpu::GpuStats {
        if what != Change::Status {
            self.reply.set(streamed(step));
        }
        if what != Change::Reply {
            self.status.set(step);
        }
        self.draw()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Change {
    Reply,
    Status,
    Both,
}

/// Streams the reply under the glass, changing the status pill far from it
/// between chunks, each frame compared with the scene drawn afresh. The
/// frames that patched, and those that blurred.
fn streamed_frames() -> (usize, usize) {
    let mut window = Window::new(1);
    window.draw();
    let (mut patched, mut blurred) = (0, 0);
    // Where the window's reply and pill stand, for the window drawn afresh.
    let (mut reply, mut status) = (1, 1);
    let steps = REPLY.len().div_ceil(CHUNK);
    for step in 2..=steps {
        let what = [Change::Reply, Change::Status, Change::Both][step % 3];
        if what != Change::Status {
            reply = step;
        }
        if what != Change::Reply {
            status = step;
        }
        let stats = window.step(step, what);
        patched += usize::from(stats.patched_items > 0);
        blurred += usize::from(stats.blur_passes > 0);
        let mut fresh = Window::new(reply);
        fresh.status.set(status);
        fresh.draw();
        assert!(
            window.gpu.readback().unwrap() == fresh.gpu.readback().unwrap(),
            "step {step} ({what:?}) differs from the scene drawn afresh (patched {})",
            stats.patched_items
        );
    }
    (patched, blurred)
}

#[test]
fn streamed_frames_under_the_glass_are_the_frames_drawn_afresh() {
    let (_, blurred) = streamed_frames();
    // The reply's chunks lie under the glass: their frames blur it again.
    assert!(blurred >= 4, "{blurred} frames blurred");
}

/// Kept frames under a backdrop blur (docs/ROADMAP.md): the pill's frames,
/// far from the glass, patch; so do the reply's, re-blurring what they reach.
#[test]
#[ignore = "kept frames under a backdrop blur: docs/ROADMAP.md"]
fn streamed_frames_under_the_glass_patch() {
    let (patched, _) = streamed_frames();
    assert!(patched >= 4, "{patched} frames patched");
}
