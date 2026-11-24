//! A text area's selection shows over its line marks (`set_line_marks`):
//! a code block's wash and a quote's rule are drawn under the selection,
//! as an atom's wash is.
use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    scene::{Color, Rect},
    text_layout::{FontStyle, TextLayout},
    widgets::{LineMark, Ui},
};
use zgx_gpu::{GpuRenderer, text::ShapedText};

const W: u32 = 320;
const H: u32 = 160;
const WASH: Color = Color(255, 240, 200, 255);

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * W + x) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}

/// The frame drawn, as pixels.
fn render(gpu: &mut GpuRenderer, ui: &mut Ui) -> Vec<u8> {
    ui.prepare_frame();
    let mut scene = ui.scene.borrow_mut();
    scene.flush();
    gpu.render(&scene, &[Rect::new(0., 0., W as f32, H as f32)])
        .unwrap();
    gpu.readback().unwrap()
}

#[test]
fn a_selection_shows_over_a_code_blocks_wash() {
    let fonts = Rc::new(RefCell::new(cosmic_text::FontSystem::new()));
    let mut ui = Ui::new(W as f32, H as f32);
    ui.scene.borrow_mut().set_font_text_shaper(
        move |text: &str, size, width, font: &FontStyle| -> Box<dyn TextLayout> {
            Box::new(ShapedText::with_font(
                &mut fonts.borrow_mut(),
                text,
                size,
                width,
                font,
            ))
        },
    );
    // The second line is code, washed to the area's edge; its middle is
    // spaces, so a pixel there is the wash's, or the selection's.
    let text = "plain\nx                    y\nend";
    let view = ui.mount(
        text_area("Note", ui.signal(text.to_owned()))
            .size(300., 120.)
            .text_size(14.)
            .line_height(20.)
            .text_wrap(true)
            // Its selection shows focused or not.
            .read_only(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_marks(Some(Rc::new(|i, _, _| {
        if i == 1 {
            vec![LineMark::new(0., 0., f32::INFINITY, f32::INFINITY, WASH)]
        } else {
            Vec::new()
        }
    })));
    ui.input.focus(&ui.scene, Some(view.node()));
    editor.editor.borrow_mut().set_selection(0, 0);
    editor.refresh();
    let mut gpu = GpuRenderer::new(W, H).unwrap();
    let unselected = render(&mut gpu, &mut ui);
    // Where the wash shows: the line's middle, in the second row.
    let wash = [WASH.0, WASH.1, WASH.2, 255];
    let row = (0..H)
        .find(|y| (100..200).all(|x| pixel(&unselected, x, *y) == wash))
        .expect("the code line's wash shows");
    // Within the spaces the selection covers (20 of them after the `x`).
    let at = (40, row + 2);
    assert_eq!(pixel(&unselected, at.0, at.1), wash, "the wash, unselected");

    // The code line's spaces selected: the selection shows over its wash.
    let start = text.find('x').unwrap() + 1;
    let end = text.find('y').unwrap();
    editor.editor.borrow_mut().set_selection(start, end);
    editor.refresh();
    let selected = render(&mut gpu, &mut ui);
    assert_ne!(
        pixel(&selected, at.0, at.1),
        wash,
        "the selection shows over the wash"
    );
    // Off the selection the wash is unchanged.
    assert_eq!(pixel(&selected, 296, at.1), pixel(&unselected, 296, at.1));
}
