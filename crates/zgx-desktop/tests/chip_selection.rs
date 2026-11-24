//! A chip in a text area shows the selection over it, as a browser's
//! `::selection` paints over a span's background: an atom's wash, and a
//! span styled as a chip (`SpanStyle::chip`), selected, take the
//! selection's colour where they're selected. Drawn by the GPU renderer
//! and read back.
use zgx::{
    compose::prelude::*,
    input::{InputEvent, PointerButton},
    scene::Color,
    text_edit::Atom,
    widgets::{EditorHandle, SpanStyle, Ui},
};
use zgx_gpu::GpuRenderer;

const W: u32 = 360;
const H: u32 = 120;
const CHIP: Color = Color(0, 200, 0, 255);

/// "see @file.pdf now" in a text area, "@file.pdf" (4..13) a chip: an
/// atom washed green (`atom`), or a green chip span from a highlighter.
fn field(atom: bool) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(W as f32, H as f32);
    let value = ui.signal(String::from("see @file.pdf now"));
    let mut area = text_area("Notes", value)
        .id("notes")
        .w(300.)
        .h(80.)
        .text_size(20.);
    area = if atom {
        area.atoms_when(|editor| vec![Atom::new(editor.new_atom_id(), 4..13)])
            .atom_wash(CHIP)
    } else {
        area.span_highlighter(|line, state| {
            let spans = line
                .find("@file.pdf")
                .map(|at| {
                    (
                        at..at + 9,
                        SpanStyle::default().background(CHIP).chip([2., 1.], 4.),
                    )
                })
                .into_iter()
                .collect();
            (spans, state)
        })
    };
    let view = ui.mount(
        column()
            .size(W as f32, H as f32)
            .bg(rgb(0xffffff))
            .child(area),
    );
    ui.prepare_frame();
    let node = view.find("notes").expect("the text area");
    // Focused as a person focuses it, by a click: the field shows its
    // selection only while it has the focus.
    let at = ui.scene.borrow().bounds(node);
    let (x, y) = (at.x + at.width - 4., at.y + at.height - 4.);
    for event in [
        InputEvent::PointerDown {
            x,
            y,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            x,
            y,
            button: PointerButton::Primary,
        },
    ] {
        ui.dispatch(event);
    }
    ui.prepare_frame();
    let editor = ui.editor(node).expect("an editor");
    (ui, editor)
}

/// How many pixels inside `area` (points, 1x) of `ui`, as drawn, are the
/// chip's colour (each channel within 8 of it).
fn chip_pixels(ui: &Ui, gpu: &mut GpuRenderer, area: zgx::scene::Rect) -> usize {
    ui.prepare_frame();
    // The frame's changes taken, as a window takes them before drawing; then
    // all of it drawn.
    ui.scene.borrow_mut().flush();
    let damage = [ui.scene.borrow().bounds(ui.scene.borrow().root())];
    gpu.render(&ui.scene.borrow(), &damage).expect("render");
    let rgba = gpu.readback().expect("readback");
    let mut count = 0;
    for y in area.y.ceil() as usize..(area.y + area.height).floor() as usize {
        for x in area.x.ceil() as usize..(area.x + area.width).floor() as usize {
            let at = (y * W as usize + x) * 4;
            let near = |c: u8, want: u8| c.abs_diff(want) <= 8;
            if near(rgba[at], CHIP.0) && near(rgba[at + 1], CHIP.1) && near(rgba[at + 2], CHIP.2) {
                count += 1;
            }
        }
    }
    count
}

/// The chip's pixels unselected, then with all the text selected.
fn selection_shows(atom: bool) -> Option<(usize, usize)> {
    let Ok(mut gpu) = GpuRenderer::new(W, H) else {
        eprintln!("skipped: no GPU adapter");
        return None;
    };
    let (ui, editor) = field(atom);
    let area = editor.range_boxes(4..13)[0];
    editor.set_selection(17, 17);
    let unselected = chip_pixels(&ui, &mut gpu, area);
    editor.set_selection(0, 17);
    let selected = chip_pixels(&ui, &mut gpu, area);
    Some((unselected, selected))
}

#[test]
fn a_selected_atom_shows_the_selection_over_its_wash() {
    let Some((unselected, selected)) = selection_shows(true) else {
        return;
    };
    assert!(
        unselected > 200,
        "unselected, the wash shows: {unselected} of its pixels"
    );
    assert!(
        selected * 4 < unselected,
        "selected, the selection over the wash: {selected} of {unselected} still the wash's"
    );
}

#[test]
fn a_selected_chip_span_shows_the_selection_over_its_background() {
    let Some((unselected, selected)) = selection_shows(false) else {
        return;
    };
    assert!(
        unselected > 200,
        "unselected, the chip shows: {unselected} of its pixels"
    );
    assert!(
        selected * 4 < unselected,
        "selected, the selection over the chip, not under its background: \
         {selected} of {unselected} still the chip's"
    );
}
