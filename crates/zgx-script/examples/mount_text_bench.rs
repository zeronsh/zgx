//! Mounting text no one has measured yet, headless, with a window's text
//! services: `ROWS` rows of two new texts, then a document of about 40 KB in
//! paragraphs. CI's instruction gate counts it, for what a new word costs as a
//! list or a document first shows. Text services are new for each, so nothing
//! is shaped already.
//! Run: cargo run --release -p zgx-script --example mount_text_bench
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};

fn next(seed: &mut u64, of: u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 33) % of
}
fn word(seed: &mut u64) -> String {
    (0..2 + next(seed, 9))
        .map(|_| (b'a' + next(seed, 26) as u8) as char)
        .collect()
}
fn rows(n: usize) -> View {
    let row = |i: usize| {
        row()
            .gap(8.)
            .px(12.)
            .py(6.)
            .bg(Color(0x1b, 0x22, 0x30, 255))
            .rounded(6.)
            .child(text(format!("Row {i}")).text_size(13.))
            .child(
                text(format!("{} MB", i * 3))
                    .text_size(12.)
                    .text_color(Color(0x88, 0x99, 0xaa, 255)),
            )
    };
    column().w(1200.).gap(4.).children((1..=n).map(row))
}
fn document(bytes: usize) -> View {
    let (mut seed, mut made) = (7_u64, 0);
    // A vocabulary, as prose has: most words seen before, some new.
    let words: Vec<String> = (0..1500).map(|_| word(&mut seed)).collect();
    let mut paragraphs = Vec::new();
    while made < bytes {
        let mut paragraph = String::new();
        for _ in 0..20 + next(&mut seed, 80) {
            let at = next(&mut seed, 1500);
            paragraph += &words[(at * at / 1500) as usize];
            paragraph.push(' ');
        }
        made += paragraph.len();
        paragraphs.push(text(paragraph).text_size(14.));
    }
    column().w(700.).gap(8.).children(paragraphs)
}
fn mount(view: View) {
    let mut ui = Ui::new(1200., 900.);
    let services = zgx_gpu::TextServices::new();
    services.install(&mut ui.scene.borrow_mut());
    ui.scene.borrow_mut().fonts_replaced();
    let _handle = ui.mount(view);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
}
fn main() {
    let n: usize = std::env::var("ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let reps: usize = std::env::var("REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    for _ in 0..reps {
        mount(rows(n));
        mount(document(40_000));
    }
}
