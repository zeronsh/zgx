//! What a token streamed into a bound editor costs as the document grows,
//! shaped as a window shapes it: 1,000 tokens appended to the text a
//! `text_area` is bound to, at documents of 1 KB to 1 MB, in the bundled
//! headless font, with the instructions each token took (macOS; 0
//! elsewhere) and its time. zgx's example of the same name measures the
//! headless estimate, without fonts.
//!   cargo run --release -p zgx-desktop --example editor_stream_bench
//! (`SIZES=16,1024` runs those sizes, in KB, alone.)
#[path = "support/headless_font.rs"]
mod headless_font;
use std::time::Instant;
use zgx::{compose::prelude::*, reactive::Signal, text::TextValue, widgets::Ui};

/// Instructions this thread has retired (macOS; 0 elsewhere).
fn instructions() -> u64 {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn thread_selfcounts(kind: i32, buffer: *mut u64, size: usize) -> i32;
        }
        let mut counts = [0u64; 2];
        // SAFETY: THSC_CPI (1) fills two u64s: instructions, cycles.
        unsafe { thread_selfcounts(1, counts.as_mut_ptr(), 16) };
        counts[0]
    }
    #[cfg(not(target_os = "macos"))]
    0
}

fn append(value: &Signal<TextValue>, token: &str) {
    value.mutate(|text| {
        match text {
            TextValue::Long(text) => {
                let end = text.len();
                text.replace_range(end..end, token);
            }
            TextValue::Flat(text) => text.push_str(token),
        }
        true
    });
}

fn main() {
    const TOKENS: usize = 1_000;
    let sizes: Vec<usize> = std::env::var("SIZES").map_or(vec![1, 16, 128, 1024], |s| {
        s.split(',')
            .filter_map(|kb| kb.trim().parse().ok())
            .collect()
    });
    println!("document   per token (instructions)   per token (time)");
    for size in sizes.into_iter().map(|kb| kb << 10) {
        let line = "a line of the document so far, as a model wrote it\n";
        let document = line.repeat(size / line.len() + 1);
        let mut ui = Ui::new(800., 600.);
        // The shaper a window has, in the font every machine has.
        let text = zgx_gpu::TextServices::new();
        headless_font::font().install(&mut text.text_system().borrow_mut());
        text.install(&mut ui.scene.borrow_mut());
        let value = ui.signal(TextValue::from(document.as_str()));
        let mounted = ui.mount(
            text_area_value("Reply", value.clone())
                .id("reply")
                .font_family(zgx_workload::FONT),
        );
        ui.prepare_frame();
        // Warm up: the first tokens settle what the binding holds.
        for i in 0..20 {
            append(&value, &format!("warm{i} "));
            ui.prepare_frame();
        }
        let (start, began) = (instructions(), Instant::now());
        for i in 0..TOKENS {
            append(&value, if i % 30 == 29 { "\n" } else { "token " });
            // A frame a token, as a window lays it out to draw it.
            ui.prepare_frame();
        }
        let (spent, took) = (instructions() - start, began.elapsed());
        println!(
            "{:>7} KB   {:>24}   {:>13.2} us",
            size >> 10,
            spent / TOKENS as u64,
            took.as_secs_f64() * 1e6 / TOKENS as f64
        );
        mounted.unmount();
        drop(text);
    }
}
