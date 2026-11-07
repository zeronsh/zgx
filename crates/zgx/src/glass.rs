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
    /// is: on Windows its Acrylic ([`GlassStyle::drawn_acrylic`]), elsewhere
    /// macOS's glass ([`GlassStyle::drawn_glass`]).
    pub fn drawn(&self, dark: bool) -> crate::shader_drive::ShaderDrive {
        match cfg!(windows) {
            true => self.drawn_acrylic(dark),
            false => self.drawn_glass(dark),
        }
    }
    /// The shadow the drawn glass casts: macOS's regular glass's (Acrylic
    /// casts none of its own).
    pub fn drawn_shadow(&self, dark: bool) -> Option<crate::scene::BoxShadow> {
        match cfg!(windows) {
            true => None,
            false => self.drawn_glass_shadow(dark),
        }
    }
    /// What it draws with Transparency effects or reduced transparency on,
    /// as the system's goes solid: its fill nearly opaque, or on Windows
    /// Acrylic's fallback colour.
    pub fn drawn_opaque(&self, dark: bool) -> crate::shader_drive::ShaderDrive {
        let mut drive = self.drawn(dark);
        match cfg!(windows) {
            true => drive.params[1][1] = 1.,
            false => {
                let gray = if dark { 44. / 255. } else { 246. / 255. };
                drive.params[1] = [gray, gray, gray, 0.97];
            }
        }
        drive
    }
    /// macOS's glass, as the system's is in a key window: its face from the
    /// numbers its own filter holds (the luma range, saturation, fill and
    /// rim colours), its blur, rim, shadow and tint fitted to its pixels
    /// (`scripts/glass_compare.py`).
    pub fn drawn_glass(&self, dark: bool) -> crate::shader_drive::ShaderDrive {
        thread_local! {
            static MATERIAL: std::sync::Arc<crate::decoration::FillShader> =
                crate::decoration::FillShader::new(include_str!("glass.wgsl"), None);
        }
        // [black, white, saturation, top], fill, blur (the system's has a
        // sharp core and a broad shoulder: one sigma fitted over text,
        // pictures and an edge together).
        let (face, fill, frost) = match (self.variant, dark) {
            (GlassVariant::Regular, false) => ([0.5, 1.03, 1., 0.97], [1., 1., 1., 0.4], 4.5),
            // Its filter's black is 0.2; on screen it reads 0.17.
            (GlassVariant::Regular, true) => ([0.17, 0.6, 1., 0.97], [0., 0., 0., 0.4], 4.5),
            (GlassVariant::Clear, _) => ([0.075, 1.15, 1.06, 1.], [1., 1., 1., 0.], 1.5),
        };
        // The rim's colour from the face: gain, luma taken off, lift; and its
        // shadow's alpha, for the material to see past it.
        let shadow = self
            .drawn_glass_shadow(dark)
            .map_or(0., |s| f32::from(s.color.3) / 255.);
        let rim = match dark {
            false => [1.5, 1.4, 0.9, shadow],
            true => [3., 1.65, 0.15, shadow],
        };
        let tint = self.tint.map_or([0.; 4], |tint| {
            [tint.0, tint.1, tint.2, tint.3].map(|v| f32::from(v) / 255.)
        });
        crate::shader_drive::ShaderDrive {
            shader: Some(MATERIAL.with(std::sync::Arc::clone)),
            params: [face, fill, rim, tint],
            frost,
            ..Default::default()
        }
    }
    /// The shadow the drawn glass casts, as the system's regular glass does
    /// in a key window (clear casts none): fitted to its pixels. Its offset
    /// and blur are the material's too (`glass.wgsl`), which sees past it.
    pub fn drawn_glass_shadow(&self, dark: bool) -> Option<crate::scene::BoxShadow> {
        (self.variant == GlassVariant::Regular).then_some(crate::scene::BoxShadow {
            color: Color(0, 0, 0, if dark { 64 } else { 30 }),
            offset: crate::scene::Transform { x: 0., y: 8. },
            blur_radius: 20.,
            spread: 0.,
        })
    }
    /// Windows' Acrylic, as WinUI's `AcrylicBrush` makes it, with its
    /// theme's numbers: regular glass the base acrylic
    /// (`AcrylicBackgroundFillColorBaseBrush`), clear the default, more
    /// see-through (`AcrylicBackgroundFillColorDefaultBrush`). A tint is the
    /// brush's own: its luminosity and opacity derived as the brush derives
    /// them.
    pub fn drawn_acrylic(&self, dark: bool) -> crate::shader_drive::ShaderDrive {
        thread_local! {
            static MATERIAL: std::sync::Arc<crate::decoration::FillShader> =
                crate::decoration::FillShader::new(include_str!("glass_acrylic.wgsl"), None);
        }
        let hex = |v: u32| [(v >> 16) & 255, (v >> 8) & 255, v & 255].map(|c| c as f32 / 255.);
        // Tint, tint opacity, luminosity opacity, fallback.
        let (tint, tint_opacity, luminosity, fallback) = match (self.variant, dark) {
            (GlassVariant::Regular, false) => (hex(0xF3F3F3), 0., 0.9, hex(0xEEEEEE)),
            (GlassVariant::Regular, true) => (hex(0x202020), 0.5, 0.96, hex(0x1C1C1C)),
            (GlassVariant::Clear, false) => (hex(0xFCFCFC), 0., 0.85, hex(0xF9F9F9)),
            (GlassVariant::Clear, true) => (hex(0x2C2C2C), 0.15, 0.96, hex(0x2C2C2C)),
        };
        let (tint, tint_opacity, luminosity_color, luminosity) = match self.tint {
            None => (tint, tint_opacity, tint, luminosity),
            Some(color) => {
                let rgb = [color.0, color.1, color.2].map(|v| f32::from(v) / 255.);
                let alpha = f32::from(color.3) / 255.;
                let (lum_rgb, lum_alpha) = acrylic_luminosity(rgb, alpha);
                (
                    rgb,
                    alpha * acrylic_tint_opacity_modifier(rgb),
                    lum_rgb,
                    lum_alpha,
                )
            }
        };
        crate::shader_drive::ShaderDrive {
            shader: Some(MATERIAL.with(std::sync::Arc::clone)),
            params: [
                [tint[0], tint[1], tint[2], tint_opacity],
                [luminosity, 0., 0., 0.],
                [fallback[0], fallback[1], fallback[2], 1.],
                [
                    luminosity_color[0],
                    luminosity_color[1],
                    luminosity_color[2],
                    0.,
                ],
            ],
            frost: 30.,
            ..Default::default()
        }
    }
    /// The drawn materials' own sources, for tests of every shader.
    pub fn material_source() -> &'static str {
        include_str!("glass.wgsl")
    }
    /// Acrylic's (`GlassStyle::drawn_acrylic`).
    pub fn acrylic_source() -> &'static str {
        include_str!("glass_acrylic.wgsl")
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
            let style = Rc::new(RefCell::new(style));
            let shading = style.clone();
            let shading_theme = theme.clone();
            let filled = match rounded.corners {
                Some(corners) => filling().rounded_corners(corners),
                None => filling().rounded(radius_of(&rounded)),
            };
            return filled
                .reactive_style(move || {
                    let dark = shading_theme.theme_kind().is_dark();
                    match (shading.borrow_mut())().drawn_shadow(dark) {
                        Some(shadow) => Styles::new().shadow(shadow),
                        None => Styles::new().shadow_none(),
                    }
                })
                .shader_drive(move || {
                    let dark = theme.theme_kind().is_dark();
                    let style = (style.borrow_mut())();
                    // Asked for less transparency, as the system's goes
                    // solid.
                    match themes.system_preferences().reduced_transparency {
                        true => style.drawn_opaque(dark),
                        false => style.drawn(dark),
                    }
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

/// RGB to HSV, each 0 to 1.
fn hsv(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta == 0. {
        0.
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.) / 6.
    } else if max == g {
        ((b - r) / delta + 2.) / 6.
    } else {
        ((r - g) / delta + 4.) / 6.
    };
    let s = if max == 0. { 0. } else { delta / max };
    [h, s, max]
}
fn rgb_of(hsv: [f32; 3]) -> [f32; 3] {
    let [h, s, v] = hsv;
    let c = v * s;
    let h = h * 6.;
    let x = c * (1. - (h.rem_euclid(2.) - 1.).abs());
    let [r, g, b] = match h as u32 {
        0 => [c, x, 0.],
        1 => [x, c, 0.],
        2 => [0., c, x],
        3 => [0., x, c],
        4 => [x, 0., c],
        _ => [c, 0., x],
    };
    [r, g, b].map(|channel| channel + v - c)
}
/// The luminosity colour and opacity WinUI's `AcrylicBrush` takes from a
/// tint given no luminosity opacity (`GetLuminosityColor`): its brightness
/// held between 0.125 and 0.965, its opacity mapped from 0.15 to 1.03.
fn acrylic_luminosity(rgb: [f32; 3], alpha: f32) -> ([f32; 3], f32) {
    let [h, s, v] = hsv(rgb);
    (
        rgb_of([h, s, v.clamp(0.125, 0.965)]),
        (alpha * 0.88 + 0.15).min(1.),
    )
}
/// How far `AcrylicBrush` lets a tint's opacity reach
/// (`GetTintOpacityModifier`): 0.45 for white, 0.85 for black, 0.9 for
/// mid grey, the suppression fading as saturation rises.
fn acrylic_tint_opacity_modifier(rgb: [f32; 3]) -> f32 {
    let [_, s, v] = hsv(rgb);
    let (lowest, deviation) = match v > 0.5 {
        true => (0.45, 0.5),
        false => (0.85, 0.5),
    };
    let suppression = (0.9 - lowest) * (1. - s * 2.).max(0.);
    0.9 - suppression * (v - 0.5).abs() / deviation
}
