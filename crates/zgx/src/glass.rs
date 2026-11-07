//! Glass: the system's own material where the platform has one, the same box
//! drawn elsewhere.
//!
//! On macOS 26 a [`glass`] box is backed by an `NSGlassEffectView`, which the
//! window server draws from what lies under it. It's a native view between the
//! window's planes ([`crate::native_view`]), so a window with no glass keeps
//! one plane and pays nothing. Its children are ordinary nodes on the plane
//! above it, and it's one node for input and assistive technology: the system's
//! view takes no events of its own.
//!
//! Elsewhere (Windows, Linux, older macOS, a renderer with no native views) the
//! box draws a material measured against the system's, so an app says the
//! same thing everywhere.
use crate::compose::prelude::*;
use crate::native_view::{NativeContent, NativeViews};
use crate::scene::Color;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// Which of the system's glasses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlassVariant {
    /// Takes on what's behind it and stays legible over anything.
    #[default]
    Regular,
    /// More see-through, for rich content.
    Clear,
}

/// How a [`glass`] box looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassStyle {
    /// Which glass.
    pub variant: GlassVariant,
    /// A colour it takes on.
    pub tint: Option<Color>,
    /// Its corners' radius in points; `f32::INFINITY` for a capsule.
    pub radius: f32,
    /// Each corner's own radius, over `radius`: a sheet rounder on top than
    /// at its bottom. The drawn glass rounds each; macOS's system glass takes
    /// one, `radius`.
    pub corners: Option<crate::decoration::Corners>,
}
impl Default for GlassStyle {
    fn default() -> Self {
        Self {
            variant: GlassVariant::Regular,
            tint: None,
            radius: 24.,
            corners: None,
        }
    }
}
impl GlassStyle {
    /// What it draws where the system has none, `dark` or light as the theme
    /// is: numbers measured against the system's own
    /// (`scripts/glass_compare.py`), its frost a backdrop blur.
    pub fn drawn(&self, dark: bool) -> crate::shader_drive::ShaderDrive {
        thread_local! {
            static MATERIAL: std::sync::Arc<crate::decoration::FillShader> =
                crate::decoration::FillShader::new(include_str!("glass.wgsl"), None);
        }
        let (frost, saturation, gray, amount, depth, bevel, rim) = match (self.variant, dark) {
            (GlassVariant::Clear, _) => (20., 1.4, 114., 0.36, 6., 10., 0.4),
            (GlassVariant::Regular, true) => (9., 2.2, 44., 0.7, 4., 8., 0.3),
            (GlassVariant::Regular, false) => (9., 2.2, 246., 0.7, 4., 8., 0.3),
        };
        let (tint, amount) = match self.tint {
            Some(tint) => (
                [f32::from(tint.0), f32::from(tint.1), f32::from(tint.2)].map(|v| v / 255.),
                f32::from(tint.3) / 255.,
            ),
            None => ([gray / 255.; 3], amount),
        };
        crate::shader_drive::ShaderDrive {
            shader: Some(MATERIAL.with(std::sync::Arc::clone)),
            params: [
                [bevel, depth, saturation, amount],
                [tint[0], tint[1], tint[2], rim],
                [0.; 4],
                [0.; 4],
            ],
            reach: depth * 1.2,
            frost,
            ..Default::default()
        }
    }
    /// The drawn material's own source, for tests of every shader.
    pub fn material_source() -> &'static str {
        include_str!("glass.wgsl")
    }
}

/// A pane of the system's glass, for its host to show.
#[derive(Clone)]
pub struct GlassPane(pub(crate) Rc<RefCell<PaneState>>);
pub(crate) struct PaneState {
    style: GlassStyle,
    view: Option<Rc<dyn Any>>,
}
impl GlassPane {
    /// How it should look.
    pub fn style(&self) -> GlassStyle {
        self.0.borrow().style
    }
    /// The platform's view its host made for it, kept here.
    pub fn view(&self) -> Option<Rc<dyn Any>> {
        self.0.borrow().view.clone()
    }
    /// Keep the view its host made.
    pub fn set_view(&self, view: Rc<dyn Any>) {
        self.0.borrow_mut().view = Some(view);
    }
}

/// A box of glass: add children, size and place it like any `div`. `style`
/// runs again when state it reads changes.
pub fn glass(mut style: impl FnMut() -> GlassStyle + 'static) -> View {
    let backing = component(move |cx| {
        let natives = cx.try_service::<NativeViews>();
        let shared = Rc::new(RefCell::new(PaneState {
            style: style(),
            view: None,
        }));
        let filling = || div().absolute().left(0.).top(0.).w_full().h_full();
        let natives = natives.filter(|natives| natives.supported() && natives.glass_supported());
        let Some(natives) = natives else {
            // No system glass: drawn. It's a box at the same place, following
            // what `style` reads.
            let rounded = style();
            let theme = cx.theme();
            let themes = crate::theme::Themes::of(&cx.runtime());
            let filled = match rounded.corners {
                Some(corners) => filling().rounded_corners(corners),
                None => filling().rounded(radius_of(&rounded)),
            };
            return filled.shader_drive(move || {
                let mut drive = style().drawn(theme.theme_kind().is_dark());
                // Asked for less transparency, as the system's goes solid.
                if themes.system_preferences().reduced_transparency {
                    drive.params[0][3] = 0.97;
                }
                drive
            });
        };
        let pane = GlassPane(shared.clone());
        let effect = cx.runtime().effect(move || {
            let next = style();
            if shared.borrow().style != next {
                shared.borrow_mut().style = next;
                crate::native_view::wake();
            }
        });
        cx.retain(effect);
        filling().pointer_events(false).on_mount(move |handle| {
            if let Some(scene) = handle.scene()
                && let Ok(mut scene) = scene.try_borrow_mut()
            {
                scene.set_platform_painted(handle.node(), true);
            }
            handle.retain(natives.register(handle.node(), NativeContent::Glass(pane)));
        })
    });
    div().relative().child(backing)
}
fn radius_of(style: &GlassStyle) -> f32 {
    if style.radius.is_finite() {
        style.radius
    } else {
        9999.
    }
}
