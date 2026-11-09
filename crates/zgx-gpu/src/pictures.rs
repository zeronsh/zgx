//! Scroll content kept in a texture of its own while it scrolls.
//!
//! A scroll frame that repaints what its view shows costs what the content
//! holds, and one that copies the pixels drawn is refused by much of what
//! content can hold. A picture costs neither: the content rasters once into a
//! texture, each row of device pixels at its row modulo the texture's height,
//! and a frame composites the rows its view shows, one quad or two where they
//! wrap. Moving rasters only the rows newly wanted, and what draws over the
//! view (a glass bar, whose blur reads the view as composited) draws as ever.
//!
//! The scene decides what can be a picture (`Scene::picture_fits`) and ends
//! them at rest, so a window that isn't scrolling holds none.
use crate::{GpuError, GpuRenderer, GpuStats, LayerCache, Quad, Region};
use rustc_hash::FxHashMap;
use std::ops::Range;
use zgx::scene::{Color, NodeId, NodeKind, PaintItem, Rect, Scene};

/// Rows rastered ahead of a scroll in a frame: what it moved, which holds its
/// lead at any steady speed, and this many more to build one, spread over
/// frames so that none takes the GPU long.
const AHEAD_ROWS: i64 = 64;

/// The one opaque colour under all of `clip` (`below`: what paints before a
/// picture there; `window`: the window's background), if there is one. Content
/// rastered over it blends just as painted over it does, where rastered over
/// nothing an edge's partial coverage rounds a level off.
///
/// None for a picture a fade or rounded clip cuts (`picture`): what's under
/// it is cut alike, and that colour in the picture would cover it twice over.
pub(crate) fn under(
    picture: &PaintItem<'_>,
    below: &[PaintItem<'_>],
    clip: Rect,
    window: Color,
) -> Color {
    let clear = Color(0, 0, 0, 0);
    if picture.mask().is_some() || picture.rounded().iter().any(Option::is_some) {
        return clear;
    }
    let covers = |rect: Rect| {
        rect.x <= clip.x
            && rect.y <= clip.y
            && rect.x + rect.width >= clip.x + clip.width
            && rect.y + rect.height >= clip.y + clip.height
    };
    for item in below.iter().rev() {
        let quad = match item.kind {
            NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } => quad,
            NodeKind::Container(_) if !item.isolated && item.effects.blur_radius <= 0. => continue,
            _ => return clear,
        };
        let shows = item.clip.map_or(item.bounds, |c| {
            item.bounds.intersection(c).unwrap_or_default()
        });
        if !shows.intersects(clip) {
            continue;
        }
        let plain = quad.radius <= 0.
            && quad.border_width <= 0.
            && quad.shadow.is_none()
            && quad.decoration.is_none()
            && item.effects.opacity >= 1.
            && item.effects.edge_fade <= 0.
            && item.effects.blur_radius <= 0.
            && item.mask().is_none()
            && item.rounded().iter().all(Option::is_none)
            && !item.isolated;
        return match plain && quad.fill.3 == 255 && covers(shows) {
            true => quad.fill,
            false if quad.fill.3 == 0 && plain => continue,
            false => clear,
        };
    }
    match window.3 {
        255 => window,
        _ => clear,
    }
}

pub(crate) struct Picture {
    /// The view, in the window's points.
    clip: Rect,
    /// What it rasters over (`under`).
    under: Color,
    scale: f32,
    /// The view's size in device pixels.
    columns: u32,
    rows: u32,
    /// The rows of content (device pixels down from its box's top) the
    /// texture holds, each at its row modulo the texture's height.
    valid: Range<i64>,
    /// The first of them the view shows.
    top: i64,
    /// The content's `Scene::layer_revision` when last brought up to date.
    revision: u64,
    /// Drawn without something on its way (`images_awaited`).
    incomplete: bool,
    /// Its children and where their ink lay in its rows, to see what moves.
    placed: FxHashMap<NodeId, Range<i64>>,
    /// The rows of content its children reach, once worked out.
    mounted: Option<Range<i64>>,
}

impl GpuRenderer {
    /// Draw scroll content from pictures while it scrolls, or not.
    pub fn set_scroll_pictures(&mut self, on: bool) {
        self.scroll_pictures = on;
    }
    /// How many pictures it holds (tests).
    pub fn debug_pictures(&self) -> usize {
        self.pictures.len()
    }
    /// Tell the scene whether pictures may start, and let go of those over.
    pub(crate) fn sync_pictures(&mut self, scene: &Scene) {
        let planes = self.views.as_ref().is_some_and(|views| views.has_planes());
        let allowed = self.scroll_pictures
            && zgx::switches::SCROLL_PICTURE.on()
            && self.plane.is_none()
            && !planes;
        scene.allow_pictures(allowed);
        if !allowed && self.scroll_pictures && !scene.scroll_moves().is_empty() {
            zgx::profile::note(|| {
                format!(
                    "scroll: no picture in this window (switched on: {}, native view plane: {}, \
                     GPU view planes: {planes})",
                    zgx::switches::SCROLL_PICTURE.on(),
                    self.plane.is_some(),
                )
            });
        }
        if !self.pictures.is_empty() {
            let layers = &mut self.layers;
            self.pictures.retain(|id, _| {
                let kept = scene.contains(*id) && scene.is_picture(*id);
                if !kept {
                    layers.remove(id);
                }
                kept
            });
        }
        if scene.has_pictures() {
            self.kept = None;
        }
    }
    /// A window at rest holds no picture (`Scene::rest` ends them).
    pub(crate) fn drop_pictures(&mut self) {
        for (id, _) in self.pictures.drain() {
            self.layers.remove(&id);
        }
    }
    /// Bring picture `id` up to date for a view showing it in `clip`.
    pub(crate) fn prepare_picture(
        &mut self,
        scene: &Scene,
        id: NodeId,
        clip: Rect,
        under: Color,
        damage: &[Rect],
        stats: &mut GpuStats,
    ) -> Result<(), GpuError> {
        let scale = self.scale;
        let top_of_content = scene.bounds(id).y;
        let columns = ((clip.width * scale).round() as u32).max(1);
        let rows = ((clip.height * scale).round() as u32).max(1);
        let stale = self.pictures.get(&id).is_none_or(|picture| {
            picture.clip != clip
                || picture.scale != scale
                || picture.columns != columns
                || picture.rows != rows
        });
        if stale {
            self.new_picture(scene, id, clip, columns, rows);
        }
        let height = i64::from(self.layers[&id].texture.height());
        let revision = scene.layer_revision(id);
        let serial = scene.flush_serial();
        let missed = self.rendered_serial.is_none_or(|drawn| drawn + 1 < serial);
        let top = ((clip.y - top_of_content) * scale).round() as i64;
        let view = top..top + i64::from(rows);
        let fresh = self.fresh;
        let picture = self.pictures.get_mut(&id).expect("made above");
        let moved = top - picture.top;
        picture.top = top;
        if moved != 0 {
            zgx::profile::note(|| {
                format!(
                    "scroll @{} composited its picture, moved {moved} px",
                    id.number()
                )
            });
        }
        let mut wanted: Vec<Range<i64>> = Vec::new();
        let new_under = std::mem::replace(&mut picture.under, under) != under;
        if fresh || missed || new_under || std::mem::take(&mut picture.incomplete) {
            picture.valid = 0..0;
        }
        let dirty = scene.take_picture_rows();
        // What changed in it: what it holds of the view again from the
        // changes, and past the view the rows that changed or moved go, with
        // whatever lies beyond them (it holds one run of rows).
        if picture.revision != revision {
            picture.revision = revision;
            let row = |y: f32| (y - top_of_content) * scale;
            let rows_of =
                |rect: Rect| row(rect.y).floor() as i64..row(rect.y + rect.height).ceil() as i64;
            let mut placed = FxHashMap::default();
            for child in scene.children(id) {
                if let Some(ink) = scene.picture_row_ink(id, *child) {
                    placed.insert(*child, rows_of(ink));
                }
            }
            let lose = |rows: &Range<i64>, valid: &mut Range<i64>| {
                if rows.start < view.start && rows.end > valid.start {
                    valid.start = valid.start.max(rows.end.min(view.start));
                }
                if rows.end > view.end && rows.start < valid.end {
                    valid.end = valid.end.min(rows.start.max(view.end));
                }
            };
            for (child, rows) in &placed {
                if picture.placed.get(child) != Some(rows) {
                    lose(rows, &mut picture.valid);
                }
            }
            for (child, rows) in &picture.placed {
                if placed.get(child) != Some(rows) {
                    lose(rows, &mut picture.valid);
                }
            }
            for child in &dirty {
                if let Some(rows) = placed.get(child) {
                    lose(rows, &mut picture.valid);
                }
            }
            picture.placed = placed;
            picture.mounted = None;
            let changed = |rect: Rect| {
                rect.intersection(clip)
                    .map(|rect| row(rect.y).floor() as i64..row(rect.y + rect.height).ceil() as i64)
            };
            match moved {
                0 => wanted.extend(damage.iter().copied().filter_map(changed)),
                _ => wanted.extend(scene.picture_changes(id).filter_map(changed)),
            }
        }
        // It may have lost touch with what the view shows now.
        if picture.valid.start >= view.end
            || picture.valid.end <= view.start
            || picture.valid.is_empty()
        {
            picture.valid = view.start..view.start;
            wanted.clear();
        }
        // Nothing is rastered ahead past what's built: those rows would only
        // go again as they are built.
        let mounted = match &picture.mounted {
            Some(mounted) => mounted.clone(),
            None => {
                let row = |y: f32| (y - top_of_content) * scale;
                let mounted = scene
                    .children(id)
                    .iter()
                    .filter_map(|child| scene.picture_row_ink(id, *child))
                    .fold(None::<Range<i64>>, |all, ink| {
                        let start = row(ink.y).floor() as i64;
                        let end = row(ink.y + ink.height).ceil() as i64;
                        Some(all.map_or(start..end, |all| all.start.min(start)..all.end.max(end)))
                    })
                    .unwrap_or(0..0);
                picture.mounted = Some(mounted.clone());
                mounted
            }
        };
        let valid = &mut picture.valid;
        if view.start < valid.start {
            wanted.push(view.start..valid.start);
            valid.start = view.start;
        }
        if view.end > valid.end {
            wanted.push(valid.end..view.end);
            valid.end = view.end;
        }
        // Rows behind the scroll give way to those ahead of it.
        let ahead = moved.abs() + AHEAD_ROWS;
        let spare = (height - i64::from(rows)) * 3 / 4;
        if moved >= 0 {
            let end = (valid.end + ahead).min(view.end + spare).min(mounted.end);
            if end > valid.end {
                wanted.push(valid.end..end);
                valid.end = end;
            }
        }
        if moved <= 0 {
            let start = (valid.start - ahead)
                .max(view.start - spare)
                .max(mounted.start);
            if start < valid.start {
                wanted.push(start..valid.start);
                valid.start = start;
            }
        }
        if valid.end - valid.start > height {
            match moved >= 0 {
                true => valid.start = valid.end - height,
                false => valid.end = valid.start + height,
            }
        }
        let valid = valid.clone();
        for band in wanted {
            let band = band.start.max(valid.start)..band.end.min(valid.end);
            self.raster_picture(scene, id, clip, top_of_content, band, stats)?;
        }
        let frame = self.layer_frame;
        if let Some(cache) = self.layers.get_mut(&id) {
            cache.used = frame;
        }
        Ok(())
    }
    /// A texture for a view of `columns` by `rows`, holding nothing yet:
    /// twice the view, or the view alone past the budget for layers.
    fn new_picture(&mut self, scene: &Scene, id: NodeId, clip: Rect, columns: u32, rows: u32) {
        self.layers.remove(&id);
        let limit = self.device.limits().max_texture_dimension_2d;
        let width = columns.next_multiple_of(64).min(limit);
        let used: usize = self.layers.values().map(|layer| layer.bytes).sum();
        let room = self.layer_budget().saturating_sub(used);
        let fits = |height: u32| width as usize * height as usize * 4 <= room;
        let height = match fits(rows * 2) {
            true => rows * 2,
            false => rows,
        }
        .next_multiple_of(64)
        .min(limit);
        let texture = crate::texture(&self.device, width, height, "scroll picture");
        let bind = self.texture_bind(&texture.create_view(&Default::default()), "scroll picture");
        let bytes = width as usize * height as usize * 4;
        let root = scene.bounds(id);
        self.layers.insert(
            id,
            LayerCache {
                texture,
                bind,
                bounds: clip,
                content: clip,
                revision: scene.layer_revision(id),
                root_origin: (root.x, root.y),
                bytes,
                uv: [1., 1.],
                step: 0,
                factor: 1f32.to_bits(),
                exact: false,
                transformed: false,
                used: self.layer_frame,
                parked: false,
                incomplete: false,
                watched: None,
            },
        );
        self.pictures.insert(
            id,
            Picture {
                clip,
                under: Color(0, 0, 0, 0),
                scale: self.scale,
                columns,
                rows,
                valid: 0..0,
                top: 0,
                // (Not yet its own: what it holds gets placed in the first go.)
                revision: scene.layer_revision(id).wrapping_sub(1),
                incomplete: false,
                placed: FxHashMap::default(),
                mounted: None,
            },
        );
    }
    /// Raster rows `band` of content into picture `id`'s texture, in a pass
    /// for each run between its wraps.
    fn raster_picture(
        &mut self,
        scene: &Scene,
        id: NodeId,
        clip: Rect,
        top_of_content: f32,
        band: Range<i64>,
        stats: &mut GpuStats,
    ) -> Result<(), GpuError> {
        let scale = self.scale;
        let (width, height) = {
            let texture = &self.layers[&id].texture;
            (texture.width() as f32, i64::from(texture.height()))
        };
        let mut at = band.start;
        while at < band.end {
            let ring = at.rem_euclid(height);
            let length = (band.end - at).min(height - ring);
            let top = top_of_content + at as f32 / scale;
            let run = Rect::new(clip.x, top, clip.width, length as f32 / scale);
            let local = Rect::new(0., ring as f32 / scale, clip.width, run.height);
            let bounds = Rect::new(
                clip.x,
                top - ring as f32 / scale,
                width / scale,
                height as f32 / scale,
            );
            let items = scene.picture_items(id, clip, &[run]);
            let banding = self.band_capable() && scene.take_painted_fades();
            for item in &items {
                if item.isolated && item.effects.opacity > 0. {
                    self.layer_shifted = false;
                    self.prepare_layer(scene, item.id, 1., true, None, stats, crate::LAYER_ROOM)?;
                }
            }
            self.prepare_effects(scene, &items)?;
            let cache = self.layers.remove(&id).expect("made by prepare_picture");
            self.banding = banding;
            self.layer_under = self.pictures[&id].under;
            let (texture, result) =
                self.paint_layer(scene, items, bounds, cache.texture, scale, &[local], false);
            self.layers.insert(id, LayerCache { texture, ..cache });
            let incomplete = std::mem::take(&mut self.layer_incomplete);
            if let Some(picture) = self.pictures.get_mut(&id) {
                picture.incomplete |= incomplete;
            }
            crate::add_layer_stats(stats, &result?);
            at += length;
        }
        Ok(())
    }
    /// The quads that composite what picture `item` shows: one, or two where
    /// its rows wrap in the texture.
    /// Whether they draw plain: nothing but the view's scissor cuts them.
    pub(crate) fn picture_quads(
        &self,
        item: &PaintItem<'_>,
        regions: &mut Vec<Region>,
        quads: &mut Vec<Quad>,
    ) -> bool {
        let picture = &self.pictures[&item.id];
        let texture = &self.layers[&item.id].texture;
        let (width, height) = (texture.width() as f32, i64::from(texture.height()));
        let scale = self.scale;
        let mask = crate::item_region(regions, item);
        let mut at = picture.top;
        let end = at + i64::from(picture.rows);
        while at < end {
            let ring = at.rem_euclid(height);
            let length = (end - at).min(height - ring);
            quads.push(Quad {
                rect: [
                    picture.clip.x,
                    picture.clip.y + (at - picture.top) as f32 / scale,
                    picture.clip.width,
                    length as f32 / scale,
                ],
                uv: [
                    0.,
                    ring as f32 / height as f32,
                    picture.columns as f32 / width,
                    length as f32 / height as f32,
                ],
                color: [1., 1., 1., item.effects.opacity],
                fade: [
                    item.bounds.x,
                    item.bounds.y,
                    item.bounds.width,
                    item.bounds.height,
                ],
                options: [0., 2., 0., 0.],
                shape: [0.; 4],
                border: [0.; 4],
                mask: [mask, 0., 0., 0.],
            });
            at += length;
        }
        mask == 0.
    }
}
