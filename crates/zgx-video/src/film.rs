//! A composition's frames on a [`Stage`]: how many the file has, each one's
//! place on the stage's clock, and motion blur's samples.
use crate::stage::Stage;

/// How a composition's frames become a file's.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Frames per second of the file; the composition's if `None`.
    pub fps: Option<f64>,
    /// Frames blended into each.
    pub samples: u32,
    /// How much of a frame's time they span.
    pub shutter: f64,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            fps: None,
            samples: 1,
            shutter: 0.5,
        }
    }
}
/// [`Timing`], in a stage's ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ticks {
    /// Frames per second of the file.
    pub output_fps: f64,
    /// The stage's ticks to a frame of the file.
    pub per_frame: u32,
    /// Those drawn and blended, the first of the frame on.
    pub samples: u32,
}
impl Ticks {
    /// `timing` of a composition running at `fps`.
    pub fn of(timing: Timing, fps: f64) -> Self {
        let samples = timing.samples.max(1);
        Self {
            output_fps: timing.fps.unwrap_or(fps),
            // Samples span the shutter's share of each frame's ticks.
            per_frame: match samples {
                1 => 1,
                _ => ((f64::from(samples) / timing.shutter.clamp(0.01, 1.)).round() as u32)
                    .max(samples),
            },
            samples,
        }
    }
    /// The rate its stage runs at.
    pub fn stage_fps(&self) -> f64 {
        self.output_fps * f64::from(self.per_frame)
    }
}

/// What a film asks of whatever plays on it.
pub(crate) struct Player {
    /// Move the composition's frame to this (fractions when the file has more
    /// frames than it).
    pub set_frame: Box<dyn Fn(f64)>,
    /// Whether it has failed.
    pub check: Box<dyn Fn() -> Result<(), String>>,
    /// What it holds the render for.
    pub holds: Box<dyn Fn() -> Vec<String>>,
    /// After a step: bring what Rust shows up to its frame (a clip's picture);
    /// whether anything changed, to lay out again.
    pub after_step: Box<dyn Fn() -> Result<bool, String>>,
}

/// A composition, standing on a stage, a frame of a file at a time.
pub struct Film {
    /// The stage it plays on.
    pub stage: Stage,
    ticks: Ticks,
    /// The composition's frames a tick moves on.
    frames_per_tick: f64,
    /// How many frames its file has.
    frames: u64,
    player: Player,
    /// A motion-blurred frame's sums, and the sample being drawn.
    blend: crate::blend::Blend,
    sample: Vec<u8>,
}
impl Film {
    pub(crate) fn new(
        stage: Stage,
        ticks: Ticks,
        fps: f64,
        duration: u64,
        player: Player,
    ) -> Result<Self, String> {
        let seconds = duration as f64 / fps;
        let film = Self {
            stage,
            ticks,
            frames_per_tick: fps / ticks.stage_fps(),
            frames: (seconds * ticks.output_fps).round().max(1.) as u64,
            player,
            blend: Default::default(),
            sample: Vec::new(),
        };
        let mut film = film;
        film.caught_up()?;
        Ok(film)
    }
    /// How its frames become a file's.
    pub fn ticks(&self) -> Ticks {
        self.ticks
    }
    /// How many frames its file has.
    pub fn output_frames(&self) -> u64 {
        self.frames
    }
    /// The frame of the file it stands at.
    pub fn output_frame(&self) -> u64 {
        self.stage.frame() / u64::from(self.ticks.per_frame)
    }
    /// Move to the file's `frame`, on from where it stands, drawing nothing.
    pub fn step_to(&mut self, frame: u64) -> Result<(), String> {
        self.step_to_tick(frame * u64::from(self.ticks.per_frame))
    }
    /// Whether its stage's ticks are longer than the composition's frames, so
    /// it may never stand inside some (a 30 fps composition at 24 fps, not
    /// blurred, stands at 8.75 and then 10): what mounts only in one of those
    /// it never mounts.
    pub fn skips_frames(&self) -> bool {
        self.frames_per_tick > 1. + 1e-9
    }
    /// Move on, drawing nothing, until it has stood at frame `frame` of the
    /// composition (a tick of the stage may fall between its frames).
    pub fn step_to_composition_frame(&mut self, frame: u64) -> Result<(), String> {
        let tick = (frame as f64 / self.frames_per_tick - 1e-9).ceil().max(0.) as u64;
        self.step_to_tick(tick)
    }
    fn step_to_tick(&mut self, tick: u64) -> Result<(), String> {
        while self.stage.frame() < tick {
            let next = self.stage.frame() + 1;
            (self.player.set_frame)(next as f64 * self.frames_per_tick);
            self.stage.step()?;
            self.caught_up()?;
        }
        Ok(())
    }
    fn caught_up(&mut self) -> Result<(), String> {
        (self.player.check)()?;
        if (self.player.after_step)()? {
            self.stage.lay_out_again()?;
            (self.player.check)()?;
        }
        Ok(())
    }
    /// Draw the frame it stands at (see [`Stage::draw`]).
    pub fn draw(&mut self, pixels: &mut Vec<u8>) -> Result<(), zgx_gpu::GpuError> {
        self.stage.draw(pixels)
    }
    /// The file's `frame`, drawn into `pixels` (RGBA, premultiplied): its
    /// samples blended, what the script holds it for waited for.
    pub fn draw_output(
        &mut self,
        frame: u64,
        pixels: &mut Vec<u8>,
        hold_timeout: std::time::Duration,
    ) -> Result<(), String> {
        let first = frame * u64::from(self.ticks.per_frame);
        let samples = self.ticks.samples;
        if samples <= 1 {
            self.step_to_tick(first)?;
            self.wait_for_holds(hold_timeout)?;
            return self.stage.draw(pixels).map_err(|error| error.to_string());
        }
        let mut sample = std::mem::take(&mut self.sample);
        for at in 0..u64::from(samples) {
            self.step_to_tick(first + at)?;
            self.wait_for_holds(hold_timeout)?;
            self.stage
                .draw(&mut sample)
                .map_err(|error| error.to_string())?;
            self.blend.add(&sample, at == 0, samples);
        }
        self.sample = sample;
        self.blend.average_into(samples, pixels);
        Ok(())
    }
    /// Wait, in real time, for what the composition holds the render for
    /// (`video.hold`), letting it run; an error past `timeout`.
    pub fn wait_for_holds(&mut self, timeout: std::time::Duration) -> Result<(), String> {
        let started = std::time::Instant::now();
        let mut held = false;
        loop {
            self.stage.settle();
            let holds = (self.player.holds)();
            if holds.is_empty() {
                return if held {
                    self.stage.lay_out_again()
                } else {
                    Ok(())
                };
            }
            held = true;
            if started.elapsed() > timeout {
                return Err(format!(
                    "frame {} waited {timeout:?} for {}",
                    self.output_frame(),
                    holds.join(", ")
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
