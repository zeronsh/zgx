//! `ZGX_PAINT_CHECK`: each frame the window shows, checked against the same
//! scene drawn whole by a second renderer of its own, to catch a frame drawn
//! from its damage (a copy, a patch, a partial repaint) that differs. The
//! window's renderer is left as it is, so what goes wrong stays wrong for the
//! next frame to carry, as it would on screen.
//!
//! `ZGX_PAINT_CHECK=1` writes to a folder of its own in the temporary
//! directory; any other value names the folder. Each frame that differs logs
//! a line, and the first `ZGX_PAINT_CHECK_SAVES` (20) save, under its frame
//! number: what the window showed, the frame before it, the frame drawn whole
//! (PPM), and its damage, scroll moves, copies and notes. A pixel differs
//! when a channel does by more than `ZGX_PAINT_CHECK_LEVELS` (2), and a frame
//! when more than `ZGX_PAINT_CHECK_PIXELS` (8) pixels do. Loops a layer plays
//! are left out of the comparison: the reference draws them in its frames.
//! Costs a whole frame and two reads of it every frame; unset, nothing.
use zgx::scene::{Rect, Scene};
use zgx_gpu::GpuRenderer;

pub(crate) struct PaintCheck {
    dir: std::path::PathBuf,
    reference: Option<GpuRenderer>,
    /// The frame drawn whole, waiting for the window's frame to show.
    pending: Option<Frame>,
    /// What the window showed last, for a frame that differs.
    before: Option<Vec<u8>>,
    frame: u64,
    /// Frames compared, and those that differed.
    compared: u64,
    differed: u64,
    /// `ZGX_PAINT_CHECK_SELFTEST=n`: frame `n`'s reference has a box cut out,
    /// so a check that works reports it.
    selftest: Option<u64>,
    saved: usize,
    saves: usize,
    levels: u8,
    pixels: usize,
}

/// A frame drawn whole, and what the window's frame did.
struct Frame {
    whole: Vec<u8>,
    width: usize,
    report: String,
    /// Device-pixel boxes left out (loops a layer plays).
    masks: Vec<(usize, usize, usize, usize)>,
}

fn env_number<T: std::str::FromStr>(name: &str, default: T) -> T {
    zgx::env::var_os(name)
        .and_then(|value| value.to_str()?.parse().ok())
        .unwrap_or(default)
}

impl PaintCheck {
    pub(crate) fn from_env() -> Option<Self> {
        let value = zgx::env::var_os("ZGX_PAINT_CHECK")?;
        if value.is_empty() || value == "0" {
            return None;
        }
        let dir = match value.to_str() {
            Some("1") => {
                std::env::temp_dir().join(format!("zgx-paint-check-{}", std::process::id()))
            }
            _ => std::path::PathBuf::from(value),
        };
        if let Err(error) = std::fs::create_dir_all(&dir) {
            eprintln!("ZGX_PAINT_CHECK: can't make {}: {error}", dir.display());
            return None;
        }
        eprintln!(
            "ZGX_PAINT_CHECK: checking every frame, into {}",
            dir.display()
        );
        Some(Self {
            dir,
            reference: None,
            pending: None,
            before: None,
            frame: 0,
            compared: 0,
            differed: 0,
            selftest: zgx::env::var_os("ZGX_PAINT_CHECK_SELFTEST")
                .and_then(|value| value.to_str()?.parse().ok()),
            saved: 0,
            saves: env_number("ZGX_PAINT_CHECK_SAVES", 20),
            levels: env_number("ZGX_PAINT_CHECK_LEVELS", 2),
            pixels: env_number("ZGX_PAINT_CHECK_PIXELS", 8),
        })
    }

    /// After `renderer` drew `scene` from `damage`: the frame drawn whole by
    /// the reference, for `shown` to compare (on macOS once it presents).
    pub(crate) fn rendered(
        &mut self,
        renderer: &mut GpuRenderer,
        scene: &Scene,
        damage: &[Rect],
        stats: &zgx_gpu::GpuStats,
    ) {
        self.frame += 1;
        let notes = renderer.take_scroll_notes();
        let (width, height) = renderer.size();
        let scale = renderer.scale_factor();
        let reference = match &mut self.reference {
            Some(reference) if reference.size() == (width, height) => reference,
            _ => match Self::reference_for(width, height) {
                Ok(reference) => self.reference.insert(reference),
                Err(error) => {
                    eprintln!("ZGX_PAINT_CHECK: no reference renderer: {error}");
                    return;
                }
            },
        };
        reference.set_scale_factor(scale);
        reference.set_background(renderer.background());
        let all = [Rect::new(
            0.,
            0.,
            width as f32 / scale,
            height as f32 / scale,
        )];
        if reference.render(scene, &all).is_err() {
            return;
        }
        // Pictures still decoding draw later in one than the other.
        if reference.images_decoded() {
            self.pending = None;
            return;
        }
        let Some(mut whole) = Self::pixels(reference) else {
            return;
        };
        if self.selftest == Some(self.frame) {
            let row = width as usize * 4;
            for line in whole.chunks_mut(row).skip(10).take(20) {
                for byte in line.iter_mut().skip(40).take(80) {
                    *byte = byte.wrapping_add(128);
                }
            }
        }
        let serial = scene.flush_serial();
        let moves: Vec<String> = scene
            .scroll_moves()
            .iter()
            .filter(|scroll| scroll.serial == serial)
            .map(|scroll| {
                format!(
                    "@{} by ({}, {}) in {:?}",
                    scroll.node.number(),
                    scroll.dx,
                    scroll.dy,
                    scroll.clip
                )
            })
            .collect();
        #[cfg(target_os = "macos")]
        let masks = scene
            .baked_loops()
            .iter()
            .filter_map(|(node, _)| scene.visible_bounds(*node))
            .map(|r| {
                let px = |v: f32| (v * scale).max(0.) as usize;
                (
                    px(r.x - 1.),
                    px(r.y - 1.),
                    px(r.x + r.width + 1.),
                    px(r.y + r.height + 1.),
                )
            })
            .collect();
        #[cfg(not(target_os = "macos"))]
        let masks = Vec::new();
        let report = format!(
            "damage {damage:?}\nscroll moves {moves:?}\nscroll copies {}\nnotes {notes:#?}\n",
            stats.scroll_copies
        );
        self.pending = Some(Frame {
            whole,
            width: width as usize,
            report,
            masks,
        });
        // Elsewhere the window's frame is its target, drawn already.
        #[cfg(not(target_os = "macos"))]
        if let Ok(shown) = renderer.readback() {
            self.shown(shown);
        }
    }

    /// After the window's renderer presented: its frame, compared.
    #[cfg(target_os = "macos")]
    pub(crate) fn presented(&mut self, renderer: &GpuRenderer) {
        if let Some((_, _, shown)) = renderer.debug_bare_pixels() {
            self.shown(shown);
        }
    }

    fn shown(&mut self, shown: Vec<u8>) {
        let Some(frame) = self.pending.take() else {
            self.before = Some(shown);
            return;
        };
        let masked = |index: usize| {
            let (x, y) = (index % frame.width, index / frame.width);
            frame
                .masks
                .iter()
                .any(|&(x0, y0, x1, y1)| x >= x0 && x < x1 && y >= y0 && y < y1)
        };
        self.compared += 1;
        let (mut count, mut most) = (0, 0);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for (index, (a, b)) in shown.chunks(4).zip(frame.whole.chunks(4)).enumerate() {
            let off = a
                .iter()
                .zip(b)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            if off > self.levels && !masked(index) {
                count += 1;
                most = most.max(off);
                let (x, y) = (index % frame.width, index / frame.width);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
        if count > self.pixels || shown.len() != frame.whole.len() {
            self.differed += 1;
            let n = self.frame;
            eprintln!(
                "ZGX_PAINT_CHECK frame {n}: {count} pixels differ from the frame drawn whole, in \
                 ({x0},{y0})..({x1},{y1}) device pixels, by up to {most}"
            );
            if self.saved < self.saves {
                self.saved += 1;
                let height = shown.len() / 4 / frame.width.max(1);
                let ppm = |name: &str, pixels: &[u8]| {
                    let mut out = format!("P6\n{} {height}\n255\n", frame.width).into_bytes();
                    for pixel in pixels.chunks(4) {
                        out.extend(Self::rgb(pixel));
                    }
                    let _ = std::fs::write(self.dir.join(format!("{n}-{name}.ppm")), out);
                };
                ppm("shown", &shown);
                ppm("whole", &frame.whole);
                if let Some(before) = &self.before {
                    ppm("before", before);
                }
                let _ = std::fs::write(
                    self.dir.join(format!("{n}.txt")),
                    format!(
                        "frame {n}: {count} pixels differ in ({x0},{y0})..({x1},{y1}), by up \
                         to {most}\n{}",
                        frame.report
                    ),
                );
            }
        }
        self.before = Some(shown);
    }

    /// A pixel's red, green and blue, as each read gives it.
    fn rgb(pixel: &[u8]) -> [u8; 3] {
        // (macOS reads its surfaces as BGRA.)
        if cfg!(target_os = "macos") {
            [pixel[2], pixel[1], pixel[0]]
        } else {
            [pixel[0], pixel[1], pixel[2]]
        }
    }

    #[cfg(target_os = "macos")]
    fn reference_for(width: u32, height: u32) -> Result<GpuRenderer, zgx_gpu::GpuError> {
        let mut reference =
            GpuRenderer::debug_for_layer(objc2_quartz_core::CALayer::new(), width, height)?;
        reference.set_paint_reference(true);
        Ok(reference)
    }
    #[cfg(not(target_os = "macos"))]
    fn reference_for(width: u32, height: u32) -> Result<GpuRenderer, zgx_gpu::GpuError> {
        let mut reference = GpuRenderer::new(width, height)?;
        reference.set_paint_reference(true);
        Ok(reference)
    }

    /// What the reference drew, in the window's read's order.
    fn pixels(reference: &mut GpuRenderer) -> Option<Vec<u8>> {
        #[cfg(target_os = "macos")]
        {
            reference.present().ok()?;
            reference.debug_bare_pixels().map(|(_, _, pixels)| pixels)
        }
        #[cfg(not(target_os = "macos"))]
        {
            reference.readback().ok()
        }
    }
}
impl Drop for PaintCheck {
    fn drop(&mut self) {
        eprintln!(
            "ZGX_PAINT_CHECK: {} of {} frames shown compared, {} differed",
            self.compared, self.frame, self.differed
        );
    }
}
