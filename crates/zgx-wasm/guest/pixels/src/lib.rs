//! A CPU-drawn app: a gradient with a ring where the pointer is and a bar
//! per kind of input heard, drawn only when something changes (it doesn't
//! animate). Its title counts what it heard, for tests and for a look.
use zgx_app::{App, Button, InputEvent, host};

#[derive(Default)]
struct Pixels {
    width: u32,
    height: u32,
    scale: f32,
    pointer: (f32, f32),
    down: bool,
    events: u32,
    keys: u32,
    text: u32,
    buttons: u32,
    wheel: u32,
    resizes: u32,
    focused: bool,
    last: String,
}

impl App for Pixels {
    fn init(info: zgx_app::SurfaceInfo) -> Result<Self, String> {
        host::set_animating(false);
        let app = Pixels {
            width: info.width,
            height: info.height,
            scale: info.scale,
            pointer: (info.width as f32 / 2., info.height as f32 / 2.),
            last: "init".into(),
            ..Default::default()
        };
        app.title();
        Ok(app)
    }
    fn frame(&mut self, _time: f64) {
        let (w, h) = (self.width.max(1), self.height.max(1));
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let ring = 24. * self.scale.max(1.);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
                let mut c = [(40. + 120. * fx) as u8, (60. + 80. * fy) as u8, 160, 255];
                let (dx, dy) = (x as f32 - self.pointer.0, y as f32 - self.pointer.1);
                let d = (dx * dx + dy * dy).sqrt();
                if (d - ring).abs() < 3. * self.scale.max(1.) {
                    c = if self.down {
                        [255, 210, 60, 255]
                    } else {
                        [255, 255, 255, 255]
                    };
                }
                pixels[i..i + 4].copy_from_slice(&c);
            }
        }
        // A bar per kind heard, along the bottom.
        let bars = [self.keys, self.text, self.buttons, self.wheel, self.resizes];
        let bar_w = (w / 12).max(2);
        for (n, count) in bars.iter().enumerate() {
            let height = (*count * 4).min(h / 2);
            for y in h - height..h {
                for x in
                    (n as u32 * bar_w * 2 + 4).min(w)..(n as u32 * bar_w * 2 + 4 + bar_w).min(w)
                {
                    let i = ((y * w + x) * 4) as usize;
                    pixels[i..i + 4].copy_from_slice(&[240, 240, 240, 255]);
                }
            }
        }
        if let Err(why) = host::present_pixels(&pixels, w, h) {
            zgx_app::log(why);
        }
    }
    fn event(&mut self, event: InputEvent) {
        self.events += 1;
        self.last = match &event {
            InputEvent::Key(key) => {
                if key.down {
                    self.keys += 1;
                }
                format!("key:{}:{}", key.code, if key.down { "down" } else { "up" })
            }
            InputEvent::Text(text) => {
                self.text += text.chars().count() as u32;
                format!("text:{text}")
            }
            InputEvent::PointerMove(p) => {
                self.pointer = (p.x, p.y);
                format!("move:{}:{}", p.x, p.y)
            }
            InputEvent::Button(b) => {
                self.buttons += 1;
                self.down = b.down && matches!(b.button, Button::Left);
                self.pointer = (b.x, b.y);
                format!(
                    "button:{}:{}:{}",
                    if b.down { "down" } else { "up" },
                    b.x,
                    b.y
                )
            }
            InputEvent::Wheel(w) => {
                self.wheel += 1;
                format!("wheel:{}", w.dy)
            }
            InputEvent::PointerMotion(m) => format!("motion:{}:{}", m.dx, m.dy),
            InputEvent::PointerLeave => "leave".into(),
            InputEvent::PointerLock(on) => format!("lock:{on}"),
            InputEvent::Focus(on) => {
                self.focused = *on;
                format!("focus:{on}")
            }
            InputEvent::Visible(on) => format!("visible:{on}"),
            InputEvent::Resize(info) => {
                self.resizes += 1;
                self.width = info.width;
                self.height = info.height;
                self.scale = info.scale;
                format!("resize:{}x{}", info.width, info.height)
            }
            InputEvent::Refresh(hz) => format!("refresh:{hz}"),
            InputEvent::GpuReset => "gpu-reset".into(),
            InputEvent::PipelineReady(id) => format!("pipeline:{id}"),
        };
        self.title();
        host::request_frame();
    }
}

impl Pixels {
    fn title(&self) {
        host::set_title(&format!(
            "size={}x{} events={} keys={} text={} buttons={} wheel={} resizes={} focused={} last={}",
            self.width,
            self.height,
            self.events,
            self.keys,
            self.text,
            self.buttons,
            self.wheel,
            self.resizes,
            self.focused,
            self.last
        ));
    }
}

zgx_app::app!(Pixels);
