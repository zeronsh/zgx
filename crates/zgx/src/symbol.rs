//! SF Symbols, as the system draws them.
//!
//! A [`symbol`] is an image the host asks the platform for: on iOS `UIImage`'s
//! system symbols, at the screen's pixels. Where there is no renderer (the
//! desktop today), it is an empty box of its size, so layouts hold.
use crate::compose::prelude::*;
use crate::image::ImageData;
use crate::scene::Color;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::Arc,
};

/// How a symbol looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SymbolStyle {
    /// Its point size, as a font's.
    pub size: f32,
    /// A font weight, 100 to 900.
    pub weight: u16,
    /// Its colour.
    pub color: Color,
}
impl Default for SymbolStyle {
    fn default() -> Self {
        Self {
            size: 17.,
            weight: 400,
            color: Color(0, 0, 0, 255),
        }
    }
}

/// A drawn symbol: its pixels, and its size in points.
pub type DrawnSymbol = (Arc<ImageData>, f32, f32);
/// What draws a symbol.
pub type SymbolRenderer = fn(&str, SymbolStyle) -> Option<DrawnSymbol>;

type Key = (String, u32, u16, [u8; 4]);
thread_local! {
    static RENDERER: Cell<Option<SymbolRenderer>> = const { Cell::new(None) };
    static DRAWN: RefCell<HashMap<Key, Option<DrawnSymbol>>> = RefCell::default();
}

/// The host's renderer (the host calls this).
pub fn set_renderer(renderer: SymbolRenderer) {
    RENDERER.with(|slot| slot.set(Some(renderer)));
}

/// Forget drawn symbols: the display's scale or the system's symbols changed.
pub fn clear_cache() {
    DRAWN.with(|drawn| drawn.borrow_mut().clear());
}

fn drawn(name: &str, style: SymbolStyle) -> Option<DrawnSymbol> {
    let key = (
        name.to_owned(),
        style.size.to_bits(),
        style.weight,
        [style.color.0, style.color.1, style.color.2, style.color.3],
    );
    DRAWN.with(|drawn| {
        if let Some(found) = drawn.borrow().get(&key) {
            return found.clone();
        }
        let made = RENDERER
            .with(Cell::get)
            .and_then(|render| render(name, style));
        drawn.borrow_mut().insert(key, made.clone());
        made
    })
}

/// The system's symbol `name` (`"wifi"`, `"chevron.forward"`); `style` runs
/// again when state it reads changes. Decorative unless given a label: what
/// holds it speaks for it.
pub fn symbol(name: impl Into<String>, mut style: impl FnMut() -> SymbolStyle + 'static) -> View {
    let name = name.into();
    component(move |_| {
        let first = style();
        let blank = Arc::new(ImageData::new(1, 1, vec![0u8; 4]).expect("one pixel"));
        let (width, height) = drawn(&name, first).map_or((first.size, first.size), |d| (d.1, d.2));
        image_signal(String::new(), move || {
            let style = style();
            drawn(&name, style).map_or_else(|| blank.clone(), |drawn| drawn.0)
        })
        .w(width)
        .h(height)
        .flex_shrink(0.)
        .accessible_hidden()
    })
}
