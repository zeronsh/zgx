//! Liquid Glass: the system's own material, where the platform has one.
//!
//! On iOS a [`glass`] box is backed by a `UIGlassEffect` view, which the
//! compositor draws from what lies under it. It is a native view between the
//! window's planes ([`crate::native_view`]), so a window with no glass keeps
//! one plane and pays nothing. Its children are ordinary zgx nodes on the
//! overlay plane, and it is one node in the accessibility tree: the system's
//! view is hidden from assistive technology, and takes no touches of its own.
//!
//! Elsewhere the box draws a backdrop blur under a tint, close enough to lay
//! out and try an interface on a desktop.
use crate::compose::prelude::*;
#[cfg(target_os = "ios")]
use crate::native_view::{NativeContent, NativeViews};
use crate::scene::Color;
#[cfg(target_os = "ios")]
use std::{cell::RefCell, rc::Rc};

/// Which of the system's glasses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlassVariant {
    /// Takes on what is behind it and stays legible over anything.
    #[default]
    Regular,
    /// More see-through, for rich content that dims behind it.
    Clear,
}

/// How a [`glass`] box looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassStyle {
    /// Which glass.
    pub variant: GlassVariant,
    /// A colour the glass takes on.
    pub tint: Option<Color>,
    /// It answers presses as the system's controls do.
    pub interactive: bool,
    /// Its corners' radius in points; `f32::INFINITY` for a capsule.
    pub radius: f32,
}
impl Default for GlassStyle {
    fn default() -> Self {
        Self {
            variant: GlassVariant::Regular,
            tint: None,
            interactive: false,
            radius: 24.,
        }
    }
}

/// What a glass node's system view is.
#[cfg(target_os = "ios")]
#[derive(Clone)]
pub enum GlassContent {
    /// A pane of glass, its style as last read.
    Pane(Rc<RefCell<GlassStyle>>),
    /// A group whose panes merge within `spacing` points of each other.
    Group(f32),
}

/// A box of glass: add children, size and place it like any `div`. `style`
/// runs again when state it reads changes.
pub fn glass(style: impl FnMut() -> GlassStyle + 'static) -> View {
    div().child(backing(style))
}

/// A group whose [`glass`] descendants merge and separate as they come within
/// `spacing` points of each other, as the system's do. On platforms without
/// the system's glass, an ordinary box.
pub fn glass_group(spacing: f32) -> View {
    #[cfg(target_os = "ios")]
    {
        div().child(native(GlassContent::Group(spacing)))
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = spacing;
        div()
    }
}

#[cfg(target_os = "ios")]
fn backing(mut style: impl FnMut() -> GlassStyle + 'static) -> View {
    component(move |cx| {
        let shared = Rc::new(RefCell::new(style()));
        let content = GlassContent::Pane(shared.clone());
        let effect = cx.runtime().effect(move || {
            let next = style();
            if *shared.borrow() != next {
                *shared.borrow_mut() = next;
                crate::native_view::wake();
            }
        });
        cx.retain(effect);
        native(content)
    })
}

/// The box filling its parent that the system's view is placed on.
#[cfg(target_os = "ios")]
fn native(content: GlassContent) -> View {
    component(move |cx| {
        let natives = cx.try_service::<NativeViews>();
        let content = NativeContent::Glass(content);
        div()
            .absolute()
            .left(0.)
            .top(0.)
            .w_full()
            .h_full()
            .on_mount(move |handle| {
                let Some(natives) = natives else { return };
                if natives.supported()
                    && let Some(scene) = handle.scene()
                    && let Ok(mut scene) = scene.try_borrow_mut()
                {
                    scene.set_platform_painted(handle.node(), true);
                }
                handle.retain(natives.register(handle.node(), content));
            })
    })
}

#[cfg(not(target_os = "ios"))]
fn backing(mut style: impl FnMut() -> GlassStyle + 'static) -> View {
    component(move |_| {
        let style = style();
        let bg = style.tint.unwrap_or(match style.variant {
            GlassVariant::Regular => Color(255, 255, 255, 96),
            GlassVariant::Clear => Color(255, 255, 255, 40),
        });
        let radius = if style.radius.is_finite() {
            style.radius
        } else {
            9999.
        };
        div()
            .absolute()
            .left(0.)
            .top(0.)
            .w_full()
            .h_full()
            .blur(20.)
            .bg(bg)
            .rounded(radius)
    })
}
