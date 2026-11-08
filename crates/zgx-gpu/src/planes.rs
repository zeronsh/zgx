//! Rendering one presentation plane around native views (`zgx::native_view`).
//!
//! A window with native views presents two planes: the base (its renderer
//! as ever, drawing all paint not above a native view) and, when anything
//! paints above one, an overlay renderer drawing just that paint into
//! surfaces of its own, shown by a layer above the native views. The overlay's
//! surfaces cover only the overlay paint's box (`set_plane_rect`), not the
//! window: its items draw relative to that box, as an isolated layer's do.
//! Both show their frames as plain layer contents, so they change with the
//! native views' geometry in one Core Animation transaction.
//!
//! A popup window's renderer is an overlay renderer drawing one popup root's
//! subtree (`Scene::set_popup`) alone, unclipped by the window, in the popup's
//! box (`set_popup`).
#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
use crate::{GpuError, GpuRenderer};
use std::{borrow::Cow, rc::Rc};
use zgx::{
    native_view::{Plane, PlaneSplit},
    scene::{NodeId, PaintItem, Rect, Scene},
};

pub(crate) struct PlaneState {
    split: Rc<PlaneSplit>,
    plane: Plane,
    /// A popup window's root: its subtree is all this draws (no split).
    popup: Option<NodeId>,
    /// The overlay surface's box in the window, in logical pixels on whole
    /// device pixels; `None` for the base plane (the whole window).
    rect: Option<Rect>,
}

/// `rect`'s edges (left, top, right, bottom) on the whole device pixels round
/// it, at `scale` pixels a point: what a plane in `rect` draws into.
fn plane_pixels(rect: Rect, scale: f32) -> (f32, f32, f32, f32) {
    let left = (rect.x * scale).floor();
    let top = (rect.y * scale).floor();
    let right = ((rect.x + rect.width) * scale).ceil().max(left + 1.);
    let bottom = ((rect.y + rect.height) * scale).ceil().max(top + 1.);
    (left, top, right, bottom)
}

/// The pixels a plane in `rect` draws into, at `scale` pixels a point
/// (`GpuRenderer::set_plane_rect`, `set_popup`): the size to make its surface.
pub fn plane_size(rect: Rect, scale: f32) -> (u32, u32) {
    let (left, top, right, bottom) = plane_pixels(rect, scale);
    ((right - left) as u32, (bottom - top) as u32)
}

impl GpuRenderer {
    /// A renderer presenting into `host`, a layer of the caller's above the
    /// window's native views (the overlay plane), sharing `context` (its
    /// device, fonts and text caches) with the window's renderer. It draws
    /// nothing until given a plane (`set_plane`) and its box (`set_plane_rect`).
    #[cfg(target_os = "macos")]
    pub fn for_overlay_layer(
        host: objc2::rc::Retained<objc2_quartz_core::CALayer>,
        context: &crate::GpuContext,
        scale: f32,
    ) -> Result<Self, GpuError> {
        let mut renderer = Self::create(None, 1, 1, context)?;
        if renderer.info.backend != wgpu::Backend::Metal {
            return Err(GpuError("overlay planes need the Metal backend".into()));
        }
        renderer.set_scale_factor(scale);
        renderer.layer_present = Some(crate::LayerPresent {
            host: Some(host),
            plain_only: true,
            ..Default::default()
        });
        // A surface of the layer's is the target (`begin_layer_frame`).
        renderer.target = crate::texture(&renderer.device, 1, 1, "retained output placeholder");
        renderer.view = renderer.target.create_view(&Default::default());
        renderer.blit = None;
        renderer.blit_bind = None;
        Ok(renderer)
    }
    /// Draw only `plane`'s paint of `split` from now on. Give every split
    /// computed to each plane's renderer: layers whose division changed lose
    /// their cached textures here.
    pub fn set_plane(&mut self, split: Rc<PlaneSplit>, plane: Plane) {
        for id in split.changed_layers() {
            self.layers.remove(id);
        }
        let rect = self.plane.as_ref().and_then(|state| state.rect);
        self.plane = Some(PlaneState {
            split,
            plane,
            popup: None,
            rect,
        });
    }
    /// Draw popup root `root`'s subtree alone from now on (a popup window's
    /// renderer), in `rect`, its box in window coordinates (see
    /// `set_plane_rect`). A new root repaints everything.
    pub fn set_popup(&mut self, root: NodeId, rect: Rect) {
        let previous = self.plane.as_ref().and_then(|state| state.popup);
        if previous != Some(root) {
            let rect = self.plane.as_ref().and_then(|state| state.rect);
            self.plane = Some(PlaneState {
                split: Rc::default(),
                plane: Plane::Overlay,
                popup: Some(root),
                rect,
            });
            self.fresh = true;
        }
        self.set_plane_rect(rect);
    }
    /// Draw none of `panel`'s shadows (`None`: every node's again), for a
    /// popup whose window the system shadows (Windows' acrylic popups,
    /// whose window is their panel's box). The window cuts its shadow past
    /// the panel, and under it, through a translucent fill (zgx draws a
    /// box's shadow whole under it, not cut to what lies past it), the
    /// shadow darkened what the system blurs behind the window: by half,
    /// under a menu's shadow of alpha 0.5.
    pub fn set_unshadowed(&mut self, panel: Option<NodeId>) {
        if self.unshadowed != panel {
            self.unshadowed = panel;
            self.fresh = true;
        }
    }
    /// What this popup's backdrop filters blur: `window`'s last frame under
    /// its box, or (`None`) its own target, which holds nothing of the app's.
    /// A popup in a window of its own can't see the window under it; what
    /// that window last drew can be copied, its renderer on this one's
    /// context and at its scale. Taken now, before this renderer's next
    /// frame: call it again when the window repaints under the popup. Past
    /// the window's edge its edge's pixels go on, so the whole of the glass is
    /// the window's, blurred, with nothing sharp through it. (macOS's popups
    /// have AppKit's material; its frames encoded with Metal directly don't
    /// read this.)
    pub fn set_backdrop(&mut self, window: Option<&GpuRenderer>) {
        let (Some(window), Some(rect)) = (window, self.plane_rect()) else {
            self.backdrop = None;
            return;
        };
        window.flush_pending();
        let (width, height) = (self.width, self.height);
        let texture = |device: &wgpu::Device| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("popup backdrop"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: crate::FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let fits = self
            .backdrop
            .as_ref()
            .is_some_and(|(texture, _)| (texture.width(), texture.height()) == (width, height));
        if !fits {
            let made = texture(&self.device);
            let view = made.create_view(&Default::default());
            self.backdrop = Some((made, view));
        }
        // Whole pixels of the window, as `set_plane_rect` snapped its box.
        let left = (rect.x * self.scale).round() as i64;
        let top = (rect.y * self.scale).round() as i64;
        let across = window_span(left, width, window.width);
        let down = window_span(top, height, window.height);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let copy = |encoder: &mut wgpu::CommandEncoder,
                    source: &wgpu::Texture,
                    from: (u32, u32),
                    target: &wgpu::Texture,
                    to: (u32, u32),
                    size: (u32, u32)| {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: source,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: from.0,
                        y: from.1,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: target,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: to.0,
                        y: to.1,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
            );
        };
        let (backdrop, _) = self.backdrop.as_ref().expect("made above");
        // What lies over the window, and the window's first and last rows
        // again past them.
        let inner = (across.1 - across.0, down.1 - down.0);
        copy(
            &mut encoder,
            &window.target,
            (across.2, down.2),
            backdrop,
            (across.0, down.0),
            inner,
        );
        let last_row = down.2 + inner.1 - 1;
        for y in (0..down.0).chain(down.1..height) {
            let row = if y < down.0 { down.2 } else { last_row };
            copy(
                &mut encoder,
                &window.target,
                (across.2, row),
                backdrop,
                (across.0, y),
                (inner.0, 1),
            );
        }
        // The first and last columns of all that again past them: from a
        // copy, a texture not being copied within itself.
        if inner.0 < width {
            let extended = texture(&self.device);
            copy(
                &mut encoder,
                backdrop,
                (0, 0),
                &extended,
                (0, 0),
                (width, height),
            );
            let last_column = across.1 - 1;
            for x in (0..across.0).chain(across.1..width) {
                let column = if x < across.0 { across.0 } else { last_column };
                copy(
                    &mut encoder,
                    backdrop,
                    (column, 0),
                    &extended,
                    (x, 0),
                    (1, height),
                );
            }
            let view = extended.create_view(&Default::default());
            self.backdrop = Some((extended, view));
        }
        self.queue.submit([encoder.finish()]);
    }
    /// The popup root this draws (`set_popup`).
    pub(crate) fn plane_popup(&self) -> Option<NodeId> {
        self.plane.as_ref().and_then(|state| state.popup)
    }
    /// Draw everything again (no native views).
    pub fn clear_plane(&mut self) {
        if let Some(state) = self.plane.take()
            && state.rect.is_some()
        {
            self.layer_origin = (0., 0.);
            self.fresh = true;
        }
    }
    /// The overlay plane's box in window coordinates: its surfaces are this
    /// size, its items draw relative to its origin. Snapped out to whole
    /// device pixels. A new origin repaints the plane; a new size makes new
    /// surfaces (the layer shows the last frame until the next).
    pub fn set_plane_rect(&mut self, rect: Rect) {
        let scale = self.scale;
        let (left, top, right, bottom) = plane_pixels(rect, scale);
        let snapped = Rect::new(
            left / scale,
            top / scale,
            (right - left) / scale,
            (bottom - top) / scale,
        );
        let Some(state) = &mut self.plane else {
            return;
        };
        if state.rect == Some(snapped) {
            return;
        }
        let moved = state
            .rect
            .is_none_or(|old| (old.x, old.y) != (snapped.x, snapped.y));
        state.rect = Some(snapped);
        self.layer_origin = (snapped.x, snapped.y);
        self.resize((right - left) as u32, (bottom - top) as u32);
        if moved {
            self.fresh = true;
        }
    }
    /// The plane's box (see `set_plane_rect`), in window coordinates.
    pub fn plane_rect(&self) -> Option<Rect> {
        self.plane.as_ref().and_then(|state| state.rect)
    }
    /// Repaint the whole target next frame: its surfaces went unshown (an
    /// overlay plane hidden while nothing painted on it).
    pub fn repaint_all(&mut self) {
        self.fresh = true;
    }
    /// Whether native views share the window. While they do, every frame
    /// shows on the plain layer, in the caller's Core Animation transaction
    /// with the views' moves, and never on the video layer: handing frames
    /// from one to the other left the window empty for a refresh (it
    /// flashed when a tab switch moved the views). The video layer's
    /// steadier pacing is for windows of zgx alone.
    pub fn present_with_native_views(&mut self, #[allow(unused)] natives: bool) {
        #[cfg(target_os = "macos")]
        if let Some(present) = &mut self.layer_present {
            present.with_natives = natives;
        }
    }
    /// Show frames on the plain layer for `duration` from now: they then
    /// change in the caller's Core Animation transaction, with native views
    /// moved in it (the video layer shows frames on its own thread, later).
    pub fn present_plain_for(&mut self, #[allow(unused)] duration: std::time::Duration) {
        #[cfg(target_os = "macos")]
        if let Some(present) = &mut self.layer_present {
            let until = std::time::Instant::now() + duration;
            present.plain_until = Some(present.plain_until.map_or(until, |at| at.max(until)));
        }
    }
    /// An overlay plane's origin, when drawing one.
    pub(crate) fn plane_offset(&self) -> Option<(f32, f32)> {
        self.plane
            .as_ref()
            .and_then(|state| state.rect)
            .map(|rect| (rect.x, rect.y))
    }
    /// What scroll moves this flush touch, repainted on an overlay plane.
    pub(crate) fn plane_scrolled(&self, scene: &Scene) -> Vec<Rect> {
        let serial = scene.flush_serial();
        scene
            .scroll_moves()
            .iter()
            .filter(|scroll| scroll.serial == serial)
            .map(|scroll| scroll.clip)
            .collect()
    }
    /// `damage` (window coordinates) within an overlay plane's box, in its
    /// space; `None` drawing the base plane.
    pub(crate) fn plane_damage(&self, damage: &[Rect]) -> Option<Vec<Rect>> {
        let rect = self.plane.as_ref()?.rect?;
        Some(
            damage
                .iter()
                .filter_map(|damage| damage.intersection(rect))
                .map(|within| {
                    Rect::new(
                        within.x - rect.x,
                        within.y - rect.y,
                        within.width,
                        within.height,
                    )
                })
                .collect(),
        )
    }
    /// `regions` in the plane's space, back in the window's.
    pub(crate) fn plane_world<'r>(&self, regions: &'r [Rect]) -> Cow<'r, [Rect]> {
        match self.plane_offset() {
            None => Cow::Borrowed(regions),
            Some((x, y)) => Cow::Owned(
                regions
                    .iter()
                    .map(|r| Rect::new(r.x + x, r.y + y, r.width, r.height))
                    .collect(),
            ),
        }
    }
    /// Keep `items` (any layer's, window coordinates) this plane draws.
    pub(crate) fn plane_filter(&self, items: &mut Vec<PaintItem<'_>>) {
        if let Some(state) = &self.plane
            && state.popup.is_none()
        {
            state.split.filter(items, state.plane);
        }
    }
    /// The window's top-level items this plane draws, in its space.
    pub(crate) fn plane_items<'a>(&self, mut items: Vec<PaintItem<'a>>) -> Vec<PaintItem<'a>> {
        self.plane_filter(&mut items);
        let Some((x, y)) = self.plane_offset() else {
            return items;
        };
        zgx::scene::translate_items(&mut items, -x, -y);
        items
    }
}

/// Along one axis, a popup from `start` (the window's pixels) and `size` long
/// over a window `whole` long: the part of the popup that lies over it
/// (`.0` to `.1`, the popup's pixels), and where that starts in the window. A
/// popup wholly past the window takes the window's nearest pixel, so the part
/// is never empty.
fn window_span(start: i64, size: u32, whole: u32) -> (u32, u32, u32) {
    let (size, whole) = (i64::from(size), i64::from(whole));
    let from = (-start).clamp(0, size - 1);
    let to = (whole - start).clamp(from + 1, size);
    let at = (start + from).clamp(0, whole - 1);
    (from as u32, (to.min(from + whole - at)) as u32, at as u32)
}
