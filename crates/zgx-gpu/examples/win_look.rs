//! zgx's menus, popovers, tooltips, focus rings and scrollbars, offscreen,
//! in the dark and light themes, as one sheet: what a platform's look
//! changes (`system_colors`'s tokens) is seen at a glance, and compared
//! byte for byte between commits (a look change on one platform must leave
//! the others' sheets as they were).
//!
//! ```sh
//! cargo run -p zgx-gpu --example win_look --release -- look.png
//! ```
//!
//! `SCALE` pixels a point (2 by default). Each platform's look, as zgx is
//! built there (and on Linux the colours zgx-desktop sets). Each cell is its own window, so
//! each holds its own focus: an open menu with the keyboard on its second
//! item, an open popover, a tooltip under the pointer, a button focused
//! from the keyboard, and a list with its scrollbar.
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers},
    scene::Rect,
    widgets::Ui,
};
use zgx_gpu::GpuRenderer;

const CELL: (f32, f32) = (300., 260.);

fn key(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
}

/// One cell's pixels: `build` mounted in `theme`, `then` run on it.
fn cell(
    scale: f32,
    theme: &str,
    build: impl FnOnce(&Ui) -> View,
    then: impl FnOnce(&mut Ui, &zgx::compose::ViewHandle),
) -> Vec<u8> {
    let (width, height) = ((CELL.0 * scale) as u32, (CELL.1 * scale) as u32);
    let mut gpu = GpuRenderer::new(width, height).expect("a renderer");
    gpu.set_scale_factor(scale);
    let mut ui = Ui::new(CELL.0, CELL.1);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let themes = zgx::theme::Themes::of(&ui.runtime);
    themes.set(theme).expect("a built-in theme");
    // The system's colours, as zgx-desktop sets them there.
    if cfg!(target_os = "linux") {
        themes.set_system_tokens(zgx::theme::linux_look());
    }
    let view = build(&ui);
    let view = ui.mount(
        column()
            .size(CELL.0, CELL.1)
            .p(16.)
            .bg(zgx::theme::tok::BACKGROUND)
            .text_color(zgx::theme::tok::TEXT_PRIMARY)
            .child(view),
    );
    ui.prepare_frame();
    then(&mut ui, &view);
    for _ in 0..3 {
        ui.prepare_frame();
        let _ = ui.scene.borrow_mut().flush();
    }
    gpu.render(&ui.scene.borrow(), &[Rect::new(0., 0., CELL.0, CELL.1)])
        .expect("a frame");
    gpu.readback().expect("its pixels")
}

fn cells(scale: f32, theme: &str) -> Vec<Vec<u8>> {
    let menu_cell = cell(
        scale,
        theme,
        |ui| {
            let open = ui.signal(false);
            let more = ui.signal(false);
            menu("Edit", open, button().id("anchor").child(text("Edit")))
                .child(menu_item("Cut").shortcut("Ctrl+X"))
                .child(menu_item("Copy").shortcut("Ctrl+C"))
                .child(menu_item("Word wrap").checked_when(|| true))
                .child(menu_item("Unavailable").disabled(true))
                .child(submenu("More", more).child(menu_item("Inside")))
        },
        |ui, view| {
            ui.input.focus(&ui.scene, view.find("anchor"));
            // Opened from the keyboard: its first item focused, then the next.
            key(ui, Key::Enter);
            ui.prepare_frame();
            key(ui, Key::ArrowDown);
        },
    );
    let popover_cell = cell(
        scale,
        theme,
        |ui| {
            popover("Details", ui.signal(true), button().child(text("Details"))).child(
                text("A popover's panel, holding text that wraps across it.").text_wrap(true),
            )
        },
        |_, _| {},
    );
    let tooltip_cell = cell(
        scale,
        theme,
        |_| {
            column().pt(80.).child(
                button()
                    .id("tipped")
                    .child(text("Hover"))
                    .tooltip("A tooltip"),
            )
        },
        |ui, view| {
            let bounds = ui.scene.borrow().bounds(view.find("tipped").unwrap());
            ui.dispatch(InputEvent::PointerMove {
                x: bounds.x + bounds.width / 2.,
                y: bounds.y + bounds.height / 2.,
            });
        },
    );
    let focus_cell = cell(
        scale,
        theme,
        |_| {
            row()
                .gap(12.)
                .child(button().id("one").child(text("Focused")))
                .child(button().child(text("Other")))
        },
        |ui, view| {
            // From the keyboard, so the ring shows.
            ui.input.focus(&ui.scene, view.find("one"));
            key(ui, Key::Tab);
            key(ui, Key::Tab);
            ui.input.focus(&ui.scene, view.find("one"));
        },
    );
    let scroll_cell = cell(
        scale,
        theme,
        |ui| {
            scroll(ui.signal(40.))
                .scrollbar(true)
                .size(220., 200.)
                .children((0..30).map(|i| text(format!("Row {i}")).h(24.)))
        },
        |_, _| {},
    );
    vec![
        menu_cell,
        popover_cell,
        tooltip_cell,
        focus_cell,
        scroll_cell,
    ]
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "look.png".into());
    let scale: f32 = std::env::var("SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2.);
    let (cw, ch) = ((CELL.0 * scale) as usize, (CELL.1 * scale) as usize);
    let rows = [cells(scale, "zgx-dark"), cells(scale, "zgx-light")];
    let columns = rows[0].len();
    let (width, height) = (cw * columns, ch * rows.len());
    let mut sheet = vec![0u8; width * height * 4];
    for (r, row) in rows.iter().enumerate() {
        for (c, pixels) in row.iter().enumerate() {
            for y in 0..ch {
                let from = &pixels[y * cw * 4..(y + 1) * cw * 4];
                let at = ((r * ch + y) * width + c * cw) * 4;
                sheet[at..at + cw * 4].copy_from_slice(from);
            }
        }
    }
    image::save_buffer(
        &out,
        &sheet,
        width as u32,
        height as u32,
        image::ColorType::Rgba8,
    )
    .expect("the sheet written");
    println!("{out}: {columns} cells by 2 themes");
}
