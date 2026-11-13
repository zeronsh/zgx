//! A canvas drawn in tiles, once it pans past its raster or zooms: 256-px
//! tiles of its drawing under a zoom (a level), made off the frame on a few
//! threads, the visible ones first from the view's middle out. Where the
//! level shown has none yet, a cell shows another level's tiles, scaled, or
//! the canvas's backdrop: a pan or a zoom never rasterizes on the frame.
//! A level is drawn under the view it was made for and panned in whole
//! device pixels, as a box's raster is: at rest, what shows is the raster's
//! pixels.
use super::*;
use std::collections::VecDeque;
use std::sync::Condvar;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Whether a zoomed canvas goes to tiles (a pan past its raster always does).
pub(super) const ENABLED: bool = true;
/// A tile's side, in device pixels.
const TILE: i32 = 256;
/// A tile's size in device pixels: its texture's (kept to be written
/// again by the renderer).
pub const TILE_SIZE: (u32, u32) = (TILE as u32, TILE as u32);
/// What a tile is drawn with beyond its edges, and cut away.
const MARGIN: i32 = 16;
pub(super) const TILE_BYTES: usize = (TILE * TILE * 4) as usize;
/// The most tiles' textures a frame uploads, whatever the time left.
const UPLOADS_MOST: usize = 12;
/// What a canvas's tiles take at most, unless three views' worth is more.
const TILE_BUDGET: usize = 40 * 1024 * 1024;
/// The levels a canvas keeps, the least used going first.
const MAX_LEVELS: usize = 6;
/// How much finer a level may be and still stand in for the zoom shown.
const FINEST: f32 = 4.;
/// How long a frame spends uploading tiles' textures, about: a burst of
/// tiles landed shows over a few frames (what stands in for them meanwhile).
const UPLOAD_BUDGET: Duration = Duration::from_millis(4);
/// How many frames a tile's texture is kept unshown.
const TEXTURE_FRAMES: u8 = 8;
/// How long a frame waits, at most, for the tiles a change to the drawing
/// shows (made on the tile threads): a small change shows at once, a large
/// one over the next frames, its old tiles shown meanwhile.
const CHANGE_WAIT: Duration = Duration::from_millis(8);
type View = zgx::scene::CanvasView;
/// A tile: its level's zoom (bits), column and row.
type Key = (u32, i32, i32);

/// A canvas's tiles, by level, and those being made.
pub(super) struct Pyramid {
    /// The drawing its tiles are of (held: what a change changed is told
    /// from it, `changed_region`).
    drawing: Arc<Canvas>,
    /// The box's scale its tiles are at.
    scale: f32,
    /// What the drawing paints within, its text too (in its points).
    bounds: Option<[f32; 4]>,
    /// The first, the level shown last.
    levels: Vec<Level>,
    /// The zoom last seen, and since when.
    zoom: (f32, Instant),
    /// A wake asked for, for when the zoom has held still.
    wake: Option<Instant>,
    tasks: HashMap<Key, Arc<Task>>,
    /// Tiles failed, not tried again (for this drawing).
    failed: std::collections::HashSet<Key>,
    landed: Arc<Landed>,
    bytes: usize,
    /// The images it showed last.
    shown: Vec<u64>,
    /// Its tiles' textures, by image, and how many frames since shown: kept
    /// only a few frames unshown (the tiles keep their pixels), as images'
    /// textures past their budget are drawn smaller.
    textured: HashMap<u64, u8>,
    /// The raster it was promoted from, whose texture the renderer has:
    /// shown where its level's tiles aren't yet, until unshown a while
    /// (`textured`). Nothing is uploaded for it.
    stand_in: Option<StandIn>,
    /// Its backdrop's pieces uploaded (`pieces`): kept while it is the same.
    pinned: std::collections::HashSet<u64>,
}
/// A promoted raster standing in for its level's tiles: drawn under `view`,
/// `region` of its device pixels, its texture keyed by `id`.
#[derive(Clone, Copy)]
struct StandIn {
    view: View,
    region: [i32; 4],
    id: u64,
}
struct Level {
    /// The view it is drawn under.
    view: View,
    /// What the drawing paints within, in its device pixels.
    painted: [i32; 4],
    tiles: HashMap<(i32, i32), Tile>,
    used: u64,
}
struct Tile {
    image: Arc<ImageData>,
    used: u64,
    /// Of a drawing before, where it changed since: shown until its
    /// cell's tile of this drawing lands.
    stale: bool,
}
type MadeTile = Result<ImageData, GpuError>;
/// Tiles made off the frame, for the frame to take.
#[derive(Default)]
struct Landed {
    made: Mutex<Vec<(Arc<Task>, MadeTile)>>,
    cond: Condvar,
}
/// A tile to make: `region` of `canvas` under `view` at `scale`.
struct Task {
    key: Key,
    canvas: Arc<Canvas>,
    view: View,
    scale: f32,
    region: [i32; 4],
    fonts: CanvasFonts,
    claimed: AtomicBool,
    done: AtomicBool,
    cancel: Arc<AtomicBool>,
    landed: Arc<Landed>,
}
impl Task {
    /// Whether this thread is the one to make it.
    fn claim(&self) -> bool {
        !self.cancel.load(Ordering::Relaxed) && !self.claimed.swap(true, Ordering::AcqRel)
    }
    fn make(self: &Arc<Self>, text: &SharedText) {
        SUPERSEDED_BY.with(|by| *by.borrow_mut() = Some(self.cancel.clone()));
        // Drawn with a margin, cut away: what is drawn near a pixmap's edge
        // is antialiased a little otherwise, and would show as a seam.
        let [x0, y0, x1, y1] = self.region;
        let grown = [x0 - MARGIN, y0 - MARGIN, x1 + MARGIN, y1 + MARGIN];
        let made = rasterize_region(&self.canvas, self.view, self.scale, grown, None, text)
            .and_then(|image| {
                let (w, stride) = ((x1 - x0) as usize, (grown[2] - grown[0]) as usize);
                let mut out = vec![0u8; w * (y1 - y0) as usize * 4];
                for (row, line) in out.chunks_exact_mut(w * 4).enumerate() {
                    let src = ((row + MARGIN as usize) * stride + MARGIN as usize) * 4;
                    line.copy_from_slice(&image.pixels()[src..src + w * 4]);
                }
                ImageData::new(w as u32, (y1 - y0) as u32, out).map_err(|e| GpuError(e.into()))
            });
        SUPERSEDED_BY.with(|by| *by.borrow_mut() = None);
        let mut held = self.landed.made.lock().unwrap_or_else(|p| p.into_inner());
        self.done.store(true, Ordering::Release);
        if !self.cancel.load(Ordering::Relaxed) {
            held.push((self.clone(), made));
        }
        drop(held);
        self.landed.cond.notify_all();
    }
}
/// The threads making tiles, and what they have to make, first first.
struct Pool {
    queue: Mutex<VecDeque<Arc<Task>>>,
    ready: Condvar,
}
fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    static STARTED: std::sync::Once = std::sync::Once::new();
    let pool = POOL.get_or_init(|| Pool {
        queue: Mutex::default(),
        ready: Condvar::new(),
    });
    STARTED.call_once(|| {
        let threads = std::thread::available_parallelism()
            .map_or(2, |n| n.get() / 2)
            .clamp(2, 3);
        for at in 0..threads {
            std::thread::Builder::new()
                .name(format!("zgx canvas tiles {at}"))
                .spawn(work)
                .expect("a canvas tile thread");
        }
    });
    pool
}
fn work() {
    thread_local! {
        static TEXT: SharedText = Default::default();
    }
    let pool = pool();
    loop {
        let task = {
            let mut queue = pool.queue.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                match queue.pop_front() {
                    Some(task) => break task,
                    None => queue = pool.ready.wait(queue).unwrap_or_else(|p| p.into_inner()),
                }
            }
        };
        if !task.claim() {
            continue;
        }
        TEXT.with(|text| {
            {
                let mut held = text.lock().unwrap_or_else(|p| p.into_inner());
                if !Arc::ptr_eq(&held.fonts.0, &task.fonts.0) {
                    held.fonts = task.fonts.clone();
                }
            }
            task.make(text);
        });
        zgx::image::wake_waiting();
    }
}
/// Wake the host at `due` (a zoom held still: its level is made then).
fn wake_at(due: Instant) {
    static TIMER: OnceLock<Mutex<std::sync::mpsc::Sender<Instant>>> = OnceLock::new();
    let sender = TIMER.get_or_init(|| {
        let (send, receive) = std::sync::mpsc::channel::<Instant>();
        std::thread::Builder::new()
            .name("zgx canvas tiles timer".into())
            .spawn(move || {
                for due in receive {
                    let left = due.saturating_duration_since(Instant::now());
                    std::thread::sleep(left);
                    zgx::image::wake_waiting();
                }
            })
            .expect("a canvas timer thread");
        Mutex::new(send)
    });
    let _ = sender.lock().unwrap_or_else(|p| p.into_inner()).send(due);
}
fn div_floor(a: i32, b: i32) -> i32 {
    a.div_euclid(b)
}
fn div_ceil(a: i32, b: i32) -> i32 {
    -(-a).div_euclid(b)
}
/// `image` (a backdrop) cut into tiles, each with the region of it it holds;
/// those at its edges padded with nothing.
pub(super) fn pieces(image: &ImageData) -> Vec<([i32; 4], Arc<ImageData>)> {
    let (w, h) = (image.width() as i32, image.height() as i32);
    let mut out = Vec::new();
    for j in 0..div_ceil(h, TILE) {
        for i in 0..div_ceil(w, TILE) {
            let at = region((i, j));
            let both = [at[0], at[1], at[2].min(w), at[3].min(h)];
            let mut pixels = vec![0u8; TILE_BYTES];
            let len = (both[2] - both[0]) as usize * 4;
            for y in both[1]..both[3] {
                let src = (y as usize * w as usize + both[0] as usize) * 4;
                let dst = (y - at[1]) as usize * TILE as usize * 4;
                pixels[dst..dst + len].copy_from_slice(&image.pixels()[src..src + len]);
            }
            if let Ok(piece) = ImageData::new(TILE as u32, TILE as u32, pixels) {
                out.push((at, Arc::new(piece)));
            }
        }
    }
    out
}
/// A tile's region in its level's device pixels.
fn region((i, j): (i32, i32)) -> [i32; 4] {
    [i * TILE, j * TILE, (i + 1) * TILE, (j + 1) * TILE]
}
/// Where `region` of an image drawn under `drawn` shows under `view`, in a
/// box at `scale`, cut to `cell` (in the box's device pixels).
fn scaled_part(
    region: [i32; 4],
    drawn: View,
    view: View,
    scale: f32,
    cell: [f32; 4],
) -> Option<Placement> {
    let k = view.scale / drawn.scale;
    let (ox, oy) = (
        (view.x - drawn.x * k) * scale,
        (view.y - drawn.y * k) * scale,
    );
    let [x0, y0, x1, y1] = region.map(|v| v as f32);
    let at = [x0 * k + ox, y0 * k + oy, x1 * k + ox, y1 * k + oy];
    let q = [
        at[0].max(cell[0]),
        at[1].max(cell[1]),
        at[2].min(cell[2]),
        at[3].min(cell[3]),
    ];
    if q[0] >= q[2] || q[1] >= q[3] {
        return None;
    }
    let (w, h) = (at[2] - at[0], at[3] - at[1]);
    Some(Placement {
        rect: [
            q[0] / scale,
            q[1] / scale,
            (q[2] - q[0]) / scale,
            (q[3] - q[1]) / scale,
        ],
        uv: [
            (q[0] - at[0]) / w,
            (q[1] - at[1]) / h,
            (q[2] - q[0]) / w,
            (q[3] - q[1]) / h,
        ],
        scaled: true,
    })
}
/// Where `new`, drawn in place of `old`, paints otherwise (in their points),
/// told by their parts (`Canvas::parts`: each `each` item's, its clips
/// balanced): what the parts between the same first ones and the same last
/// ones paint, in either. Elsewhere the same parts paint, in the same order
/// under the same clips (but for texts in layers joined past their most,
/// `Canvas::layers`, which hang on all of the drawing). `Some(None)`:
/// nowhere. `None` where it can't be told: a drawing not all parts. The
/// same parts' text boxes are kept for `new` (its words measured alone).
fn changed_region(
    old: &Arc<Canvas>,
    new: &Arc<Canvas>,
    text: &SharedText,
) -> Option<Option<[f32; 4]>> {
    if old.rasters_text() != new.rasters_text() || !all_parts(old) || !all_parts(new) {
        return None;
    }
    let (a, b) = (old.parts(), new.parts());
    let same = |i: usize, j: usize| {
        let (p, q) = (&a[i], &b[j]);
        old.commands()[p.commands.clone()] == new.commands()[q.commands.clone()]
            && old.texts()[p.texts.clone()] == new.texts()[q.texts.clone()]
            && p.texts.clone().zip(q.texts.clone()).all(|(s, t)| {
                old.text_transform(s) == new.text_transform(t)
                    && old.text_after(s).checked_sub(p.commands.start)
                        == new.text_after(t).checked_sub(q.commands.start)
            })
    };
    let most = a.len().min(b.len());
    let first = (0..most).take_while(|&k| same(k, k)).count();
    let last = (0..most - first)
        .take_while(|&k| same(a.len() - 1 - k, b.len() - 1 - k))
        .count();
    let mut held = text.lock().unwrap_or_else(|p| p.into_inner());
    if new.rasters_text() && !new.texts().is_empty() {
        let texts_first = first.checked_sub(1).map_or(0, |at| a[at].texts.end);
        let texts_last = match last {
            0 => 0,
            _ => old.texts().len() - a[a.len() - last].texts.start,
        };
        held.boxes_after(new, old, texts_first, texts_last);
    }
    let mut out: Option<[f32; 4]> = None;
    for (canvas, parts) in [(old, first..a.len() - last), (new, first..b.len() - last)] {
        let bounds = held.part_bounds(canvas);
        for [x0, y0, x1, y1] in parts.filter_map(|at| bounds.get(at).copied().flatten()) {
            let o = out.get_or_insert([x0, y0, x1, y1]);
            *o = [o[0].min(x0), o[1].min(y0), o[2].max(x1), o[3].max(y1)];
        }
    }
    Some(out)
}
/// Whether all of `canvas` is in its parts, one after another.
fn all_parts(canvas: &Canvas) -> bool {
    let (mut commands, mut texts) = (0, 0);
    for part in canvas.parts() {
        if part.commands.start != commands || part.texts.start != texts {
            return false;
        }
        (commands, texts) = (part.commands.end, part.texts.end);
    }
    !canvas.parts().is_empty()
        && commands == canvas.commands().len()
        && texts == canvas.texts().len()
}
impl Pyramid {
    /// The images it showed last, which must keep their textures.
    pub(super) fn shown(&self) -> impl Iterator<Item = u64> + '_ {
        self.shown.iter().copied()
    }
    fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        (self.levels.iter())
            .flat_map(|level| level.tiles.values().map(|tile| tile.image.id()))
            .chain(self.stand_in.map(|s| s.id))
            .chain(self.pinned.iter().copied())
    }
    /// Where its stand-in shows under `view` in a box at `scale`, if it
    /// covers all of `cell` (in the box's device pixels).
    fn stand_in_over(&self, view: View, scale: f32, cell: [f32; 4]) -> Option<(u64, Placement)> {
        let s = self
            .stand_in
            .filter(|s| self.textured.contains_key(&s.id))?;
        let k = view.scale / s.view.scale;
        let (ox, oy) = (
            (view.x - s.view.x * k) * scale,
            (view.y - s.view.y * k) * scale,
        );
        let [x0, y0, x1, y1] = s.region.map(|v| v as f32);
        // (Half a pixel's slack: a level's own cells are whole pixels.)
        let covers = x0 * k + ox <= cell[0] + 0.5
            && y0 * k + oy <= cell[1] + 0.5
            && x1 * k + ox >= cell[2] - 0.5
            && y1 * k + oy >= cell[3] - 0.5;
        if !covers {
            return None;
        }
        scaled_part(s.region, s.view, view, scale, cell).map(|p| (s.id, p))
    }
    fn cancel_all(&mut self) {
        for task in self.tasks.values() {
            task.cancel.store(true, Ordering::Relaxed);
        }
        self.tasks.clear();
    }
    /// What lands, taken into its level (if still of this drawing).
    fn take_landed(&mut self, clock: u64) {
        let made = std::mem::take(&mut *self.landed.made.lock().unwrap_or_else(|p| p.into_inner()));
        for (task, made) in made {
            if self
                .tasks
                .get(&task.key)
                .is_some_and(|t| Arc::ptr_eq(t, &task))
            {
                self.tasks.remove(&task.key);
            }
            if !Arc::ptr_eq(&self.drawing, &task.canvas) {
                continue;
            }
            let Some(level) = self.levels.iter_mut().find(|level| level.view == task.view) else {
                continue;
            };
            match made {
                Ok(image) => {
                    let tile = Tile {
                        image: Arc::new(image),
                        used: clock,
                        stale: false,
                    };
                    self.bytes += TILE_BYTES;
                    if let Some(old) = level.tiles.insert((task.key.1, task.key.2), tile) {
                        self.bytes -= TILE_BYTES;
                        drop(old);
                    }
                }
                Err(e) if e.0 == SUPERSEDED => {}
                Err(_) => {
                    self.failed.insert(task.key);
                }
            }
        }
    }
}
impl Drop for Pyramid {
    fn drop(&mut self) {
        self.cancel_all();
    }
}
impl CanvasCache {
    /// A tile's texture took `took` to upload in frame `frame` (the
    /// renderer's count): what the frame's budget has left, and what the
    /// next count on (an average of the last few).
    pub fn tile_uploaded(&mut self, frame: u64, took: Duration) {
        let ms = took.as_secs_f32() * 1e3;
        self.uploads.0 = match self.uploads.0 {
            0. => ms,
            was => was * 0.8 + ms * 0.2,
        };
        match self.uploads.1 == frame {
            true => (self.uploads.2, self.uploads.3) = (self.uploads.2 + ms, self.uploads.3 + 1),
            false => (self.uploads.1, self.uploads.2, self.uploads.3) = (frame, ms, 1),
        }
    }
    /// How many new tiles' textures frame `frame` uploads still: those that
    /// fit in what its budget has left (a canvas may be drawn more than
    /// once a frame), at least one if it has spent none.
    fn tile_uploads(&self, frame: u64) -> usize {
        let (each, at, spent, count) = self.uploads;
        let each = if each > 0. { each } else { 1. };
        let (spent, count) = if at == frame { (spent, count) } else { (0., 0) };
        let left = UPLOAD_BUDGET.as_secs_f32() * 1e3 - spent;
        let fit = match count > 0 {
            true => (left.max(0.) / each) as usize,
            false => ((left / each) as usize).max(1),
        };
        fit.min(UPLOADS_MOST - count.min(UPLOADS_MOST))
    }
    /// Whether image `id` is `node`'s raster standing in for its tiles: its
    /// texture is kept (`get_tiles` retires it once unshown a while).
    pub fn stands_in(&self, node: NodeId, id: u64) -> bool {
        (self.pyramids.get(&node))
            .and_then(|p| p.stand_in)
            .is_some_and(|s| s.id == id)
    }
    /// `node`'s stand-in has no texture after all (the renderer let it go):
    /// it shows no more.
    pub fn stand_in_lost(&mut self, node: NodeId) {
        if let Some(p) = self.pyramids.get_mut(&node)
            && let Some(s) = p.stand_in.take()
        {
            p.textured.remove(&s.id);
        }
    }
    /// Whether `node` is drawn in tiles (`get_tiles`).
    #[inline]
    pub fn tiled(&self, node: NodeId) -> bool {
        !self.pyramids.is_empty() && self.pyramids.contains_key(&node)
    }
    /// The images it drew and draws no more: their textures can go.
    pub fn take_retired(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.retired)
    }
    pub(super) fn retain_pyramids(&mut self, keep: impl Fn(NodeId) -> bool) {
        let gone: Vec<NodeId> = self
            .pyramids
            .keys()
            .copied()
            .filter(|id| !keep(*id))
            .collect();
        for id in gone {
            let pyramid = self.pyramids.remove(&id).expect("listed");
            self.retired.extend(pyramid.ids());
        }
    }
    /// `node`'s raster, panned past or zoomed from: to be drawn in tiles
    /// from now on (`get_tiles`), the raster's texture standing in for its
    /// level's until they are made (no texture uploaded for it on the
    /// frame). What it gives back shows nothing.
    pub(super) fn promote(&mut self, node: NodeId, canvas: &Arc<Canvas>) -> (Raster, Placement) {
        let entry = self
            .entries
            .remove(&node)
            .expect("promoted from its raster");
        self.bytes -= entry.bytes;
        self.zooms.remove(&node);
        let none = Placement {
            rect: [0.; 4],
            uv: [0.; 4],
            scaled: false,
        };
        let raster = Raster {
            id: entry.id,
            pixels: None,
        };
        let mut level = Level {
            view: entry.view,
            painted: [0; 4],
            tiles: HashMap::new(),
            used: self.clock,
        };
        let mut stand_in = None;
        if let Some(panned) = entry.panned {
            level.painted = panned.painted;
            stand_in = Some(StandIn {
                view: entry.view,
                region: panned.region,
                id: entry.id,
            });
        }
        let shapes = painted_bounds(canvas);
        let words = text_bounds(canvas, &self.text);
        let pyramid = Pyramid {
            drawing: canvas.clone(),
            scale: entry.scale,
            bounds: match (shapes, words) {
                (Some(a), Some(b)) => Some([
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]),
                (a, b) => a.or(b),
            },
            levels: vec![level],
            zoom: (entry.view.scale, Instant::now()),
            wake: None,
            tasks: HashMap::new(),
            failed: Default::default(),
            landed: Arc::default(),
            bytes: 0,
            shown: Vec::new(),
            // (Its texture kept while it is shown, and a while after.)
            textured: stand_in.iter().map(|s| (s.id, 0)).collect(),
            stand_in,
            pinned: Default::default(),
        };
        self.backdrops.entry(node).or_default();
        self.pyramids.insert(node, pyramid);
        (raster, none)
    }
    /// `canvas` for `node` under `view`, drawn in tiles (`tiled`): what
    /// shows in its `width` by `height` point box at `scale`, image by
    /// image, into `out`. `true`: some shows scaled or not at all while
    /// tiles are made off the frame (the host is woken as they land).
    /// `wait`, or a change to the drawing, makes those that show at once
    /// (on the tile threads and this one), as a box's raster is. `frame`:
    /// the renderer's frame count, for its upload budget (`tile_uploaded`).
    #[allow(clippy::too_many_arguments)]
    pub fn get_tiles(
        &mut self,
        node: NodeId,
        canvas: &Arc<Canvas>,
        view: View,
        width: f32,
        height: f32,
        scale: f32,
        wait: bool,
        frame: u64,
        out: &mut Vec<(Raster, Placement)>,
    ) -> Result<bool, GpuError> {
        if ![width, height, scale, view.x, view.y, view.scale]
            .iter()
            .all(|v| v.is_finite())
            || width <= 0.
            || height <= 0.
            || scale <= 0.
            || view.scale <= 0.
            || canvas.commands().len() > 65536
        {
            return Err(GpuError(
                "invalid canvas dimensions or command count".into(),
            ));
        }
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        let size = [
            (width * scale).ceil() as i32,
            (height * scale).ceil() as i32,
        ];
        if !wait {
            self.keep_backdrop(node, canvas, scale);
        }
        let Some(mut p) = self.pyramids.remove(&node) else {
            return Ok(false);
        };
        let now = Instant::now();
        p.take_landed(clock);
        let same = Arc::ptr_eq(&p.drawing, canvas) || *p.drawing == **canvas;
        if same {
            p.drawing = canvas.clone();
        }
        // Another scale: none of its tiles stand. Another drawing: its tiles
        // are stale, shown until made again (off the frame), those where it
        // changed (`changed_region`) first.
        let rescaled = p.scale != scale;
        let redrawn = !same && !rescaled;
        let changed = !same || rescaled;
        let mut differs = Some(None);
        if rescaled {
            p.cancel_all();
            self.retired.extend(p.ids());
            if let Some(s) = p.stand_in.take() {
                p.textured.remove(&s.id);
            }
            p.pinned.clear();
            p.levels.clear();
            p.bytes = 0;
        } else if redrawn {
            p.cancel_all();
            differs = changed_region(&p.drawing, canvas, &self.text);
            // (Where it paints the same, only texts in layers joined past
            // their most may differ: remade too, but not waited for.)
            if differs != Some(None) {
                for level in &mut p.levels {
                    for tile in level.tiles.values_mut() {
                        tile.stale = true;
                    }
                }
            }
        }
        if changed {
            p.failed.clear();
            p.scale = scale;
            p.drawing = canvas.clone();
            let shapes = painted_bounds(canvas);
            let words = text_bounds(canvas, &self.text);
            p.bounds = match (shapes, words) {
                (Some(a), Some(b)) => Some([
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]),
                (a, b) => a.or(b),
            };
            for level in &mut p.levels {
                level.painted = device_bounds(p.bounds, level.view, scale);
            }
        }
        if p.zoom.0 != view.scale {
            p.zoom = (view.scale, now);
        }
        // The level of this zoom: made once the zoom holds still (or at
        // once ×2 from every level, or to wait, or at another scale).
        let mut current = p.levels.iter().position(|l| l.view.scale == view.scale);
        if current.is_none() {
            let ratio = |a: f32, b: f32| (a / b).max(b / a);
            let nearest = (p.levels.iter())
                .map(|l| ratio(l.view.scale, view.scale))
                .fold(f32::INFINITY, f32::min);
            let due = p.zoom.1 + REFINE_AFTER;
            if wait || rescaled || nearest >= 2. || now >= due {
                p.levels.insert(
                    0,
                    Level {
                        view,
                        painted: device_bounds(p.bounds, view, scale),
                        tiles: HashMap::new(),
                        used: clock,
                    },
                );
                current = Some(0);
                // The least used past the most go.
                while p.levels.len() > MAX_LEVELS {
                    let (at, _) = (p.levels.iter().enumerate().skip(1))
                        .min_by_key(|(_, l)| l.used)
                        .expect("more than one");
                    let level = p.levels.remove(at);
                    p.bytes -= level.tiles.len() * TILE_BYTES;
                    self.retired
                        .extend(level.tiles.values().map(|t| t.image.id()));
                }
            } else if p.wake.is_none_or(|w| w != due) {
                p.wake = Some(due);
                wake_at(due);
            }
        }
        // The cells to show: the level's tiles, or a grid like them.
        let (painted, shift) = match current {
            Some(at) => {
                let level = &mut p.levels[at];
                level.used = clock;
                let shift = [
                    ((view.x - level.view.x) * scale).round() as i32,
                    ((view.y - level.view.y) * scale).round() as i32,
                ];
                (level.painted, shift)
            }
            None => (device_bounds(p.bounds, view, scale), [0, 0]),
        };
        let seen = [-shift[0], -shift[1], size[0] - shift[0], size[1] - shift[1]];
        let shows = intersect(seen, painted);
        let mut cells = Vec::new();
        if !is_empty(shows) {
            for j in div_floor(shows[1], TILE)..div_ceil(shows[3], TILE) {
                for i in div_floor(shows[0], TILE)..div_ceil(shows[2], TILE) {
                    cells.push((i, j));
                }
            }
        }
        // From the middle out.
        let (cx, cy) = (
            (seen[0] + seen[2]) as f32 / 2.,
            (seen[1] + seen[3]) as f32 / 2.,
        );
        let distance = |&(i, j): &(i32, i32)| {
            let (x, y) = (
                (i * TILE + TILE / 2) as f32 - cx,
                (j * TILE + TILE / 2) as f32 - cy,
            );
            x * x + y * y
        };
        cells.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
        let fonts = self
            .text
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .fonts
            .clone();
        // Tiles to make: those that show, then a ring about them.
        let mut wanted: Vec<Arc<Task>> = Vec::new();
        if let Some(at) = current {
            let bits = view.scale.to_bits();
            let level = &p.levels[at];
            let mut ring = Vec::new();
            if !is_empty(shows) {
                let [i0, j0, i1, j1] = [
                    div_floor(shows[0], TILE) - 1,
                    div_floor(shows[1], TILE) - 1,
                    div_ceil(shows[2], TILE) + 1,
                    div_ceil(shows[3], TILE) + 1,
                ];
                for j in j0..j1 {
                    for i in i0..i1 {
                        if (i == i0 || i == i1 - 1 || j == j0 || j == j1 - 1)
                            && !is_empty(intersect(region((i, j)), level.painted))
                        {
                            ring.push((i, j));
                        }
                    }
                }
                ring.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
            }
            let made = |cell: &(i32, i32)| level.tiles.get(cell).is_some_and(|t| !t.stale);
            for cell in cells.iter().chain(&ring) {
                let key = (bits, cell.0, cell.1);
                if made(cell) || p.failed.contains(&key) {
                    continue;
                }
                let task = p.tasks.entry(key).or_insert_with(|| {
                    Arc::new(Task {
                        key,
                        canvas: canvas.clone(),
                        view: level.view,
                        scale,
                        region: region(*cell),
                        fonts: fonts.clone(),
                        claimed: AtomicBool::new(false),
                        done: AtomicBool::new(false),
                        cancel: Arc::default(),
                        landed: p.landed.clone(),
                    })
                });
                wanted.push(task.clone());
            }
            let visible = wanted.len().min(cells.iter().filter(|c| !made(c)).count());
            // A change's: those it reaches first, the frame waiting for them.
            let mut hot = 0;
            if redrawn {
                let hit = differs.map(|r| r.map(|r| device_bounds(Some(r), level.view, scale)));
                let reaches = |t: &Arc<Task>| match hit {
                    None => true,
                    Some(None) => false,
                    Some(Some(hit)) => !is_empty(intersect(t.region, hit)),
                };
                wanted[..visible].sort_by_key(|t| !reaches(t));
                hot = wanted[..visible].iter().filter(|t| reaches(t)).count();
            }
            // Those no longer wanted, if not begun, go.
            let keep: std::collections::HashSet<Key> = wanted.iter().map(|t| t.key).collect();
            p.tasks.retain(|key, task| {
                let kept = keep.contains(key) || task.claimed.load(Ordering::Acquire);
                if !kept {
                    task.cancel.store(true, Ordering::Relaxed);
                }
                kept
            });
            if !wanted.is_empty() || !p.tasks.is_empty() {
                let pool = pool();
                let mut queue = pool.queue.lock().unwrap_or_else(|p| p.into_inner());
                queue.retain(|t| !Arc::ptr_eq(&t.landed, &p.landed));
                // Before others' (the canvas shown last is the one in use).
                for task in wanted.iter().rev() {
                    if !task.claimed.load(Ordering::Acquire) {
                        queue.push_front(task.clone());
                    }
                }
                drop(queue);
                pool.ready.notify_all();
            }
            // A change: those that show where it is, made on the tile
            // threads, the frame waiting a little for them (those not made
            // by then land later, the old tiles shown meanwhile).
            if redrawn && !wait && hot > 0 {
                let deadline = Instant::now() + CHANGE_WAIT;
                let now_wanted = &wanted[..hot];
                let mut held = p.landed.made.lock().unwrap_or_else(|p| p.into_inner());
                while !now_wanted.iter().all(|t| t.done.load(Ordering::Acquire)) {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    held = (p.landed.cond)
                        .wait_timeout(held, left)
                        .unwrap_or_else(|p| p.into_inner())
                        .0;
                }
                drop(held);
                p.take_landed(clock);
            }
            // Made now: those that show, this thread helping.
            if (wait || rescaled) && visible > 0 {
                let now_wanted = &wanted[..visible];
                for task in now_wanted.iter().rev() {
                    if task.claim() {
                        task.make(&self.text);
                    }
                }
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut held = p.landed.made.lock().unwrap_or_else(|p| p.into_inner());
                while !now_wanted.iter().all(|t| t.done.load(Ordering::Acquire))
                    && Instant::now() < deadline
                {
                    held = (p.landed.cond)
                        .wait_timeout(held, Duration::from_millis(20))
                        .unwrap_or_else(|p| p.into_inner())
                        .0;
                }
                drop(held);
                p.take_landed(clock);
                self.rasterizations += 1;
            }
        }
        // What shows, cell by cell: the level's tile, else another's, else
        // the backdrop.
        let backdrop = (self.backdrops.get(&node))
            .filter(|b| b.made.as_ref().is_some_and(|m| m.3 == scale))
            .map(|b| (b.made.as_ref().expect("made").0, b.pieces.clone()));
        if backdrop.is_some()
            && let Some(b) = self.backdrops.get_mut(&node)
        {
            b.used = clock;
        }
        // (Pieces of a backdrop before were let go when it was replaced.)
        if !p.pinned.is_empty() {
            let current: std::collections::HashSet<u64> = (backdrop.iter())
                .flat_map(|(_, pieces)| pieces.iter().map(|p| p.1.id()))
                .collect();
            p.pinned.retain(|id| current.contains(id));
        }
        let mut others: Vec<usize> = (0..p.levels.len())
            .filter(|at| Some(*at) != current)
            .filter(|at| p.levels[*at].view.scale / view.scale <= FINEST)
            .collect();
        let off = |at: &usize| (p.levels[*at].view.scale / view.scale).ln().abs();
        others.sort_by(|a, b| off(a).total_cmp(&off(b)));
        let mut awaiting = false;
        // Tiles shown with a texture, or to be uploaded this frame: few,
        // but all to wait for or at another scale (it shows at once, as a
        // raster's does).
        let mut fresh = 0;
        let most = match wait || rescaled {
            true => usize::MAX,
            false => self.tile_uploads(frame),
        };
        let ready = |textured: &HashMap<u64, u8>, fresh: usize, id: u64| {
            textured.contains_key(&id) || fresh < most
        };
        p.shown.clear();
        let full = [0., 0., width * scale, height * scale];
        for cell in &cells {
            let at = region(*cell);
            let box_at = [
                at[0] + shift[0],
                at[1] + shift[1],
                at[2] + shift[0],
                at[3] + shift[1],
            ];
            let c = intersect(box_at, [0, 0, size[0], size[1]]);
            if is_empty(c) {
                continue;
            }
            if let Some(cur) = current
                && let Some(tile) = p.levels[cur].tiles.get_mut(cell)
                && ready(&p.textured, fresh, tile.image.id())
            {
                fresh += usize::from(!p.textured.contains_key(&tile.image.id()));
                tile.used = clock;
                awaiting |= tile.stale;
                let (x, y) = (c[0] as f32 / scale, c[1] as f32 / scale);
                let w = ((c[2] - c[0]) as f32 / scale).min(width - x);
                let h = ((c[3] - c[1]) as f32 / scale).min(height - y);
                if w > 0. && h > 0. {
                    p.shown.push(tile.image.id());
                    out.push((
                        Raster {
                            id: tile.image.id(),
                            pixels: Some(tile.image.clone()),
                        },
                        Placement {
                            rect: [x, y, w, h],
                            uv: [
                                (c[0] - box_at[0]) as f32 / TILE as f32,
                                (c[1] - box_at[1]) as f32 / TILE as f32,
                                w * scale / TILE as f32,
                                h * scale / TILE as f32,
                            ],
                            scaled: false,
                        },
                    ));
                }
                continue;
            }
            awaiting = true;
            let c = c.map(|v| v as f32);
            let c = [c[0], c[1], c[2].min(full[2]), c[3].min(full[3])];
            let mut covered = false;
            // The stand-in, among the others by how far its zoom is.
            let stand_off = (p.stand_in)
                .filter(|s| s.view.scale / view.scale <= FINEST)
                .map(|s| (s.view.scale / view.scale).ln().abs());
            let mut stand_tried = stand_off.is_none();
            for at in others.iter().map(Some).chain([None]) {
                if !stand_tried
                    && at.is_none_or(|at| {
                        stand_off.is_some_and(|o| {
                            o <= (p.levels[*at].view.scale / view.scale).ln().abs()
                        })
                    })
                {
                    stand_tried = true;
                    if let Some((id, placement)) = p.stand_in_over(view, scale, c) {
                        p.shown.push(id);
                        out.push((Raster { id, pixels: None }, placement));
                        covered = true;
                        break;
                    }
                }
                let Some(at) = at else { break };
                let level = &p.levels[*at];
                let k = view.scale / level.view.scale;
                let (ox, oy) = (
                    (view.x - level.view.x * k) * scale,
                    (view.y - level.view.y * k) * scale,
                );
                let f = [
                    (c[0] - ox) / k,
                    (c[1] - oy) / k,
                    (c[2] - ox) / k,
                    (c[3] - oy) / k,
                ];
                let f = [
                    f[0].floor() as i32,
                    f[1].floor() as i32,
                    f[2].ceil() as i32,
                    f[3].ceil() as i32,
                ];
                let f = intersect(f, level.painted);
                if is_empty(f) {
                    continue;
                }
                let range = (div_floor(f[1], TILE)..div_ceil(f[3], TILE)).flat_map(|j| {
                    (div_floor(f[0], TILE)..div_ceil(f[2], TILE)).map(move |i| (i, j))
                });
                let all = range.clone().all(|t| match level.tiles.get(&t) {
                    Some(tile) => ready(&p.textured, fresh, tile.image.id()),
                    None => is_empty(intersect(region(t), level.painted)),
                });
                if !all {
                    continue;
                }
                for t in range {
                    if let Some(tile) = level.tiles.get(&t)
                        && let Some(placement) = scaled_part(region(t), level.view, view, scale, c)
                    {
                        fresh += usize::from(!p.textured.contains_key(&tile.image.id()));
                        p.shown.push(tile.image.id());
                        out.push((
                            Raster {
                                id: tile.image.id(),
                                pixels: Some(tile.image.clone()),
                            },
                            placement,
                        ));
                    }
                }
                covered = true;
                break;
            }
            // Else the backdrop's pieces, if uploaded (or all can be now).
            if !covered && let Some((drawn, pieces)) = &backdrop {
                let parts: Vec<(&Arc<ImageData>, Placement)> = (pieces.iter())
                    .filter_map(|(r, image)| {
                        scaled_part(*r, *drawn, view, scale, c).map(|placed| (image, placed))
                    })
                    .collect();
                let new = (parts.iter())
                    .filter(|(image, _)| !p.pinned.contains(&image.id()))
                    .count();
                if new == 0 || fresh + new <= most {
                    for (image, placement) in parts {
                        fresh += usize::from(p.pinned.insert(image.id()));
                        p.shown.push(image.id());
                        out.push((
                            Raster {
                                id: image.id(),
                                pixels: Some(image.clone()),
                            },
                            placement,
                        ));
                    }
                }
            }
        }
        // Those used least go past the budget, but none shown now.
        let budget = TILE_BUDGET.max(cells.len() * 3 * TILE_BYTES);
        if p.bytes > budget {
            let mut all: Vec<(u64, usize, (i32, i32))> = (p.levels.iter().enumerate())
                .flat_map(|(at, l)| l.tiles.iter().map(move |(cell, t)| (t.used, at, *cell)))
                .filter(|(used, ..)| *used != clock)
                .collect();
            all.sort_unstable_by_key(|(used, ..)| *used);
            for (_, at, cell) in all {
                if p.bytes <= budget {
                    break;
                }
                if let Some(tile) = p.levels[at].tiles.remove(&cell) {
                    self.retired.push(tile.image.id());
                    p.textured.remove(&tile.image.id());
                    p.bytes -= TILE_BYTES;
                }
            }
            // Levels left empty go, unless shown.
            let mut at = 0;
            p.levels.retain(|l| {
                let keep = Some(at) == current || !l.tiles.is_empty();
                at += 1;
                keep
            });
        }
        // The backdrop's pieces not uploaded yet, with what the frame has
        // left (not drawn: `rect` empty), so a zoom out finds them there.
        let mut unloaded = false;
        if let Some((_, pieces)) = &backdrop {
            for (_, image) in pieces {
                if p.pinned.contains(&image.id()) {
                    continue;
                }
                if fresh >= most {
                    unloaded = true;
                    break;
                }
                fresh += 1;
                p.pinned.insert(image.id());
                out.push((
                    Raster {
                        id: image.id(),
                        pixels: Some(image.clone()),
                    },
                    Placement {
                        rect: [0.; 4],
                        uv: [0.; 4],
                        scaled: false,
                    },
                ));
            }
        }
        // Tiles left for the next frame's uploads: it comes at once.
        if fresh >= most && (awaiting || unloaded) {
            wake_at(now);
        }
        // Textures of tiles unshown a while go (their pixels stay; a
        // backdrop's pieces stay uploaded).
        for id in &p.shown {
            if !p.pinned.contains(id) {
                p.textured.insert(*id, 0);
            }
        }
        let retired = &mut self.retired;
        p.textured.retain(|id, age| {
            *age += 1;
            let kept = *age <= TEXTURE_FRAMES;
            if !kept {
                retired.push(*id);
            }
            kept
        });
        if p.stand_in.is_some_and(|s| !p.textured.contains_key(&s.id)) {
            p.stand_in = None;
        }
        self.pyramids.insert(node, p);
        Ok(awaiting)
    }
}
#[cfg(test)]
mod tests;
