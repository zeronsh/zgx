//! A Ghostty terminal's colours following the theme, live (macOS). A
//! program prints a row of full blocks in the default colour and one in
//! ANSI red, once, and sleeps; then, three times over:
//!
//! - the user's own colours (zgx's dark theme sets no terminal colours);
//! - a theme setting `terminal.foreground` and `terminal.ansi.1`;
//! - a scope over the terminal setting `terminal.foreground` alone, the
//!   theme's red kept;
//! - the dark theme again: the user's colours, as at first.
//!
//! Each is read from Ghostty's own last frame (`Terminal::snapshot`): the
//! colour down the first column of blocks, and the colour below it. Ghostty
//! draws in Display P3 on macOS, the sRGB colours it is given converted
//! (`window-colorspace = srgb`), so a colour matches as given or as P3. The
//! blocks are printed once, so each change repaints what was printed, in
//! the same surface (the terminal's backend kept). Prints `USER`, `STEP <name>
//! <ok|FAIL> …` and `GHOSTTY_THEME_DONE ok=<bool>`, and exits 1 on a
//! failure. Opens one window.
//! Run: cargo run -p zgx-desktop --features ghostty --example ghostty_theme_probe
use std::sync::atomic::{AtomicBool, Ordering};
use zgx::{
    compose::prelude::*,
    scene::Color,
    terminal::{Terminal, TerminalOptions, terminal},
    theme::{DARK, Theme, ThemeOverride, tok},
};
use zgx_desktop::{Application, WindowOptions};

static FAILED: AtomicBool = AtomicBool::new(false);

type Rgb = (u8, u8, u8);

const PAPER_TEXT: Rgb = (0x20, 0x21, 0x24);
const PAPER_RED: Rgb = (0xb0, 0x00, 0x20);
const SCOPED_TEXT: Rgb = (0x00, 0x00, 0xff);

/// The colour of the first column of blocks, and the next colour below it.
fn bands(term: &Terminal) -> Option<(Rgb, Rgb)> {
    let image = term.snapshot(4096)?;
    let (width, height) = (image.width() as usize, image.height() as usize);
    let pixels = image.pixels();
    let at = |x: usize, y: usize| {
        let i = (y * width + x) * 4;
        (pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3])
    };
    // The first opaque pixel, row by row: the first block's top left.
    let (x0, y0) = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .find(|&(x, y)| at(x, y).3 > 240)?;
    let x = (x0 + 4).min(width - 1);
    let first = at(x, (y0 + 3).min(height - 1));
    let differs = |a: (u8, u8, u8, u8), b: (u8, u8, u8, u8)| {
        a.0.abs_diff(b.0) as u32 + a.1.abs_diff(b.1) as u32 + a.2.abs_diff(b.2) as u32 > 30
    };
    let below = (y0 + 3..height)
        .find(|&y| at(x, y).3 > 240 && differs(at(x, y), first))
        .map(|y| at(x, (y + 3).min(height - 1)))?;
    Some(((first.0, first.1, first.2), (below.0, below.1, below.2)))
}

/// An sRGB colour in Display P3's coordinates.
fn display_p3((r, g, b): Rgb) -> Rgb {
    let linear = |c: u8| {
        let c = f64::from(c) / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let encode = |c: f64| {
        let c = c.clamp(0., 1.);
        let v = if c <= 0.0031308 {
            12.92 * c
        } else {
            1.055 * c.powf(1. / 2.4) - 0.055
        };
        (v * 255.).round() as u8
    };
    let (r, g, b) = (linear(r), linear(g), linear(b));
    (
        encode(0.8225 * r + 0.1774 * g),
        encode(0.0332 * r + 0.9669 * g),
        encode(0.0171 * r + 0.0724 * g + 0.9108 * b),
    )
}

fn close(a: Rgb, b: Rgb) -> bool {
    a.0.abs_diff(b.0) <= 6 && a.1.abs_diff(b.1) <= 6 && a.2.abs_diff(b.2) <= 6
}
/// `seen` is `want`, as given or in Display P3.
fn near(seen: Rgb, want: Rgb) -> bool {
    close(seen, want) || close(seen, display_p3(want))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let script = std::env::temp_dir().join(format!("zgx-theme-probe-{}.sh", std::process::id()));
    std::fs::write(
        &script,
        "printf '\\033[?25l\\033[2J\\033[H'\n\
         printf '████████████████████\\r\\n\\033[31m████████████████████\\033[0m\\r\\n'\n\
         sleep 600\n",
    )?;
    let command = format!("/bin/sh {}", script.display());
    Application::new()
        .window(WindowOptions {
            title: "Ghostty theme probe".into(),
            width: 700.,
            height: 400.,
            ..Default::default()
        })
        .run(move |cx| {
            cx.ui.theme().register(
                Theme::from_json(
                    r##"{ "name": "probe-paper", "type": "light", "tokens": {
                        "terminal.foreground": "#202124", "terminal.ansi.1": "#b00020" } }"##,
                )
                .unwrap(),
            );
            let term = Terminal::new(
                &cx.ui.runtime,
                TerminalOptions {
                    command: Some(command.clone()),
                    ..Default::default()
                },
            );
            let scoped = cx.ui.runtime.signal(false);
            let over = scoped.clone();
            cx.render(
                column()
                    .w_full()
                    .h_full()
                    .bg(rgba(0xffffffff))
                    .child(theme_scope_with(
                        move || match over.get() {
                            true => ThemeOverride::new().with(
                                tok::TERMINAL_FOREGROUND,
                                Color(SCOPED_TEXT.0, SCOPED_TEXT.1, SCOPED_TEXT.2, 255),
                            ),
                            false => ThemeOverride::new(),
                        },
                        terminal(&term).grow().w_full(),
                    )),
            );
            let themes = cx.ui.theme().clone();
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                let pause = |ms| zgx::timer::sleep(std::time::Duration::from_millis(ms));
                // The program's output, read until it shows.
                let mut user = None;
                for _ in 0..50 {
                    pause(200).await;
                    user = bands(&term);
                    if user.is_some() {
                        break;
                    }
                }
                let Some(user) = user else {
                    println!("STEP output FAIL no blocks in Ghostty's frame");
                    FAILED.store(true, Ordering::Relaxed);
                    window.close();
                    return;
                };
                println!("USER text={:?} red={:?}", user.0, user.1);
                // The surface: its backend, made once.
                let backend = |term: &Terminal| {
                    term.backend()
                        .map(|backend| std::rc::Rc::as_ptr(&backend) as *const () as usize)
                };
                let surface = backend(&term);
                let expect = |name: &'static str, want: (Rgb, Rgb)| {
                    let user_step = want == user;
                    let term = term.clone();
                    async move {
                        let mut seen = None;
                        // Up to 3 s for Ghostty's renderer to draw it.
                        for _ in 0..15 {
                            pause(200).await;
                            seen = bands(&term);
                            // The user's colours, read from the frame, match as read.
                            let matches = |(a, b): (Rgb, Rgb)| match user_step {
                                true => close(a, want.0) && close(b, want.1),
                                false => near(a, want.0) && near(b, want.1),
                            };
                            if seen.is_some_and(matches) {
                                println!("STEP {name} ok {seen:?}");
                                return;
                            }
                        }
                        println!("STEP {name} FAIL saw {seen:?} want {want:?}");
                        FAILED.store(true, Ordering::Relaxed);
                    }
                };
                for round in 0..3 {
                    println!("ROUND {round}");
                    themes.set("probe-paper").unwrap();
                    expect("theme", (PAPER_TEXT, PAPER_RED)).await;
                    scoped.set(true);
                    expect("scope", (SCOPED_TEXT, PAPER_RED)).await;
                    scoped.set(false);
                    expect("unscoped", (PAPER_TEXT, PAPER_RED)).await;
                    themes.set(DARK).unwrap();
                    expect("user-again", user).await;
                }
                let same = surface.is_some() && backend(&term) == surface;
                println!("SURFACE same={same}");
                if !same {
                    FAILED.store(true, Ordering::Relaxed);
                }
                window.close();
            });
        })?;
    let _ = std::fs::remove_file(&script);
    let ok = !FAILED.load(Ordering::Relaxed);
    println!("GHOSTTY_THEME_DONE ok={ok}");
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}
