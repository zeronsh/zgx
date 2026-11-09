//! An app that goes wrong on purpose, for the host's tests: a key picks how.
//! `KeyT` panics (a trap), `KeyL` loops forever, `KeyM` takes memory until
//! it can't, `KeyE` exits with 7 (WASI 0.2 says only that it failed), `KeyF` reads a file it has no folder for,
//! and its title counts the events heard until then.
use zgx_app::{App, InputEvent, host};

struct Misbehave {
    events: u32,
    hoard: Vec<Vec<u8>>,
    /// What its last try found (`KeyF`).
    found: String,
}

impl App for Misbehave {
    fn init(_info: zgx_app::SurfaceInfo) -> Result<Self, String> {
        host::set_animating(false);
        host::set_title("events=0");
        Ok(Misbehave {
            events: 0,
            hoard: Vec::new(),
            found: String::new(),
        })
    }
    fn frame(&mut self, _time: f64) {}
    fn event(&mut self, event: InputEvent) {
        self.events += 1;
        host::set_title(&format!("events={}{}", self.events, self.found));
        let InputEvent::Key(key) = event else { return };
        if !key.down {
            return;
        }
        match key.code.as_str() {
            "KeyT" => panic!("a trap on purpose"),
            "KeyL" => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            "KeyM" => loop {
                let mut block = vec![0u8; 1 << 20];
                block[0] = 1;
                self.hoard.push(std::hint::black_box(block));
            },
            "KeyE" => std::process::exit(7),
            "KeyF" => {
                let read = std::fs::read_to_string("/etc/passwd").map(|s| s.len());
                let listed = std::fs::read_dir("/").map(|d| d.count());
                self.found = format!(" file={read:?} dir={listed:?}");
                host::set_title(&format!("events={}{}", self.events, self.found));
            }
            _ => {}
        }
    }
}

zgx_app::app!(Misbehave);
