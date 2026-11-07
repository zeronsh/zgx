//! A system symbol: the host's drawing at its size, or an empty box of it.
use std::sync::Arc;
use zgx::compose::prelude::*;
use zgx::symbol::{SymbolStyle, symbol};
use zgx::widgets::Ui;

#[test]
fn a_symbol_with_no_renderer_is_an_empty_box_of_its_size_hidden_from_assistive_technology() {
    let mut ui = Ui::new(200., 100.);
    let view = ui.mount(div().id("holder").child(symbol("wifi", || SymbolStyle {
        size: 20.,
        ..Default::default()
    })));
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let scene = ui.scene.borrow();
    let node = scene.children(holder)[0];
    let bounds = scene.bounds(node);
    assert_eq!((bounds.width, bounds.height), (20., 20.));
    assert!(ui.semantics.borrow().get(node).is_none_or(|n| n.hidden));
}

#[test]
fn a_renderers_drawing_sets_the_symbols_size() {
    fn render(name: &str, style: SymbolStyle) -> Option<zgx::symbol::DrawnSymbol> {
        (name == "chevron.forward").then(|| {
            let pixels = zgx::image::ImageData::new(48, 36, vec![255; 48 * 36 * 4]).unwrap();
            (Arc::new(pixels), style.size * 1.2, style.size * 0.9)
        })
    }
    zgx::symbol::set_renderer(render);
    zgx::symbol::clear_cache();
    let mut ui = Ui::new(200., 100.);
    let view = ui.mount(
        div()
            .id("holder")
            .child(symbol("chevron.forward", || SymbolStyle {
                size: 20.,
                ..Default::default()
            })),
    );
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let scene = ui.scene.borrow();
    let bounds = scene.bounds(scene.children(holder)[0]);
    assert_eq!((bounds.width, bounds.height), (24., 18.));
}
