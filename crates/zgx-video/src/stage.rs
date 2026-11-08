//! A view on a virtual clock, drawn offscreen a frame at a time.
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use zgx::clock::VirtualClock;
use zgx::compose::{TaskRunner, View, ViewHandle, provide};
use zgx::frame::{Frame, FrameClock};
use zgx::reactive::Runtime;
use zgx::scene::Color;
use zgx::task::LocalExecutor;
use zgx::widgets::Ui;
use zgx_gpu::{GpuError, GpuRenderer};

/// A [`Stage`]'s size and pace.
#[derive(Clone, Debug)]
pub struct StageOptions {
    /// Its width, in points.
    pub width: f32,
    /// Its height, in points.
    pub height: f32,
    /// Frames per second.
    pub fps: f64,
    /// Pixels per point.
    pub scale: f32,
    /// What's behind the view. Transparent keeps the view's own alpha.
    pub background: Color,
    /// Fonts its text may name, beside the system's.
    pub fonts: Vec<zgx_gpu::text::FontData>,
}
impl Default for StageOptions {
    fn default() -> Self {
        Self {
            width: 1920.,
            height: 1080.,
            fps: 30.,
            scale: 1.,
            background: Color(0, 0, 0, 255),
            fonts: Vec::new(),
        }
    }
}

/// What a [`Stage`] builds its view with.
pub struct Setting<'a> {
    /// The runtime its state lives in.
    pub runtime: &'a Runtime,
}

/// A view on a virtual clock of its own: each [`Stage::step`] is one frame's
/// time, for animations, timers and shaders alike, and [`Stage::draw`] draws
/// it. The thread's clock is its own while it lives, so a thread has one
/// stage at a time.
pub struct Stage {
    gpu: GpuRenderer,
    /// What frames are read back through, kept from one to the next.
    readback: zgx_gpu::Readback,
    handle: ViewHandle,
    ui: Ui,
    executor: Rc<RefCell<LocalExecutor>>,
    frames: FrameClock,
    options: StageOptions,
    /// The frame it stands at.
    frame: u64,
    /// Fields drop in order: what waits on the clock goes before it.
    clock: VirtualClock,
    /// Its thread decodes every picture a frame draws, until it goes.
    _whole_frames: WholeFrames,
}

/// This thread's frames decode every picture they draw while it lives (see
/// `zgx::image::set_whole_frames_on_this_thread`), as it was put back after:
/// a stage made on an app's own thread (`render_still` from a window's) leaves
/// that thread's pictures decoding off it, as a window's should.
struct WholeFrames(bool);
impl WholeFrames {
    fn on() -> Self {
        Self(zgx::image::set_whole_frames_on_this_thread(true))
    }
}
impl Drop for WholeFrames {
    fn drop(&mut self) {
        zgx::image::set_whole_frames_on_this_thread(self.0);
    }
}

impl Stage {
    /// `build`'s view, mounted and laid out, standing at frame 0.
    pub fn new(
        options: StageOptions,
        build: impl FnOnce(Setting<'_>) -> Result<View, String>,
    ) -> Result<Self, String> {
        let clock = VirtualClock::new();
        // A frame drawn without a picture's pixels would be written out so: a
        // large one decodes as it's first drawn, as a small one does, rather
        // than off the thread for a later frame. Put back when it goes, or if
        // it fails to be made.
        let whole_frames = WholeFrames::on();
        let (width, height) = pixels(&options);
        let mut gpu = GpuRenderer::new(width, height).map_err(|error| error.to_string())?;
        gpu.set_scale_factor(options.scale);
        gpu.set_background(options.background);
        {
            let text = gpu.text_system();
            let mut text = text.borrow_mut();
            for font in &options.fonts {
                font.install(&mut text);
            }
        }
        let mut ui = Ui::new(options.width, options.height);
        ui.contain_layout_feedback();
        {
            let mut scene = ui.scene.borrow_mut();
            scene.set_pixel_scale(options.scale);
            gpu.install_text(&mut scene);
        }
        let view = build(Setting {
            runtime: &ui.runtime,
        })?;
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let frames = FrameClock::new();
        let handle = ui.mount(provide(
            zgx::native_view::NativeViews::new(),
            provide(
                TaskRunner::from_executor(executor.clone()),
                provide(frames.clone(), view),
            ),
        ));
        let mut stage = Self {
            gpu,
            readback: zgx_gpu::Readback::default(),
            handle,
            ui,
            executor,
            frames,
            options,
            frame: 0,
            clock,
            _whole_frames: whole_frames,
        };
        stage.settle();
        stage.lay_out()?;
        stage.deliver()?;
        Ok(stage)
    }
    /// The frame it stands at.
    pub fn frame(&self) -> u64 {
        self.frame
    }
    /// The view's document.
    pub fn ui(&self) -> &Ui {
        &self.ui
    }
    /// The mounted view.
    pub fn handle(&self) -> &ViewHandle {
        &self.handle
    }
    /// Its options.
    pub fn options(&self) -> &StageOptions {
        &self.options
    }
    /// The time at `frame`, from frame 0.
    pub fn time_of(&self, frame: u64) -> Duration {
        Duration::from_secs_f64(frame as f64 / self.options.fps)
    }
    /// Run what is ready, until nothing is.
    pub fn settle(&self) {
        while self.executor.borrow_mut().tick() > 0 {}
    }
    /// Move on one frame: timers due by then fire at their own times, then
    /// what waits on the frame clock runs, and it lays out. Nothing is drawn.
    pub fn step(&mut self) -> Result<(), String> {
        self.frame += 1;
        let to = self.time_of(self.frame);
        let executor = self.executor.clone();
        self.clock
            .advance(to, || while executor.borrow_mut().tick() > 0 {});
        self.deliver()
    }
    /// Move on to `frame`, drawing nothing; a frame already past stays where it is.
    pub fn step_to(&mut self, frame: u64) -> Result<(), String> {
        while self.frame < frame {
            self.step()?;
        }
        Ok(())
    }
    fn deliver(&mut self) -> Result<(), String> {
        self.frames.deliver(Frame {
            // Counts from 1, as a display's refreshes do.
            index: self.frame + 1,
            time: self.clock.now(),
            interval: Duration::from_secs_f64(1. / self.options.fps),
        });
        self.settle();
        self.lay_out()
    }
    fn lay_out(&mut self) -> Result<(), String> {
        let now = self.clock.now();
        self.ui
            .advance_interactions(now)
            .map_err(|error| error.to_string())?;
        // Layout may start what is due now (`ui.after(0)` from `on_resize`),
        // which may lay out again.
        for _ in 0..8 {
            self.ui
                .try_prepare_frame()
                .map_err(|error| error.to_string())?;
            self.settle();
            if !self.executor.borrow().has_ready() && !self.ui.reclaiming() {
                break;
            }
        }
        Ok(())
    }
    /// Lay out what has happened since it last did.
    pub fn lay_out_again(&mut self) -> Result<(), String> {
        self.lay_out()
    }
    /// Draw the frame it stands at, and read it back into `pixels`, RGBA with
    /// premultiplied alpha, a row at a time.
    pub fn draw(&mut self, pixels: &mut Vec<u8>) -> Result<(), GpuError> {
        let mut scene = self.ui.scene.borrow_mut();
        scene.flush();
        // Whole, each time: a frame owes nothing to the frames drawn before it,
        // so a frame reached by seeking is the frame reached by playing.
        let whole = [scene.bounds(scene.root())];
        self.gpu.render(&scene, &whole)?;
        drop(scene);
        self.gpu.readback_into(pixels, &mut self.readback)?;
        Ok(())
    }
    /// Draw the frame it stands at, whole, reading nothing back: what a frame
    /// costs to make, for measuring it.
    #[doc(hidden)]
    pub fn render(&mut self) -> Result<(), GpuError> {
        let mut scene = self.ui.scene.borrow_mut();
        scene.flush();
        let whole = [scene.bounds(scene.root())];
        self.gpu.render(&scene, &whole).map(drop)
    }
    /// [`Stage::draw`], repainting only what changed since it last drew, as a
    /// window does: for comparing the two.
    #[doc(hidden)]
    pub fn draw_damaged(&mut self, pixels: &mut Vec<u8>) -> Result<(), GpuError> {
        let mut scene = self.ui.scene.borrow_mut();
        let report = scene.flush();
        self.gpu.render(&scene, &report.damage)?;
        drop(scene);
        self.gpu.readback_into(pixels, &mut self.readback)?;
        Ok(())
    }
    /// The size of what [`Stage::draw`] reads, in pixels.
    pub fn pixel_size(&self) -> (u32, u32) {
        pixels(&self.options)
    }
}
fn pixels(options: &StageOptions) -> (u32, u32) {
    (
        ((options.width * options.scale).round() as u32).max(1),
        ((options.height * options.scale).round() as u32).max(1),
    )
}
