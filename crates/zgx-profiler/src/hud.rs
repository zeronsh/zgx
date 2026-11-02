use std::sync::Arc;

use zgx::{
    scene::{Color, Layout, NodeId, NodeKind, QuadStyle, Rect, Scene, Style, Transform},
    text_layout::{FontFamily, FontStyle},
    widgets::Ui,
};

use crate::record::{Bar, PhaseCost, Summary, WAIT};

pub struct Model<'a> {
    pub summary: &'a Summary,
    pub graph: &'a [Bar],
    pub thrashing: &'a [(usize, u32)],
    pub damage: &'a [Rect],
    /// Where the app is repainting, with each rect's age in milliseconds:
    /// the flash overlay draws them pulsing as they fade.
    pub flashes: &'a [(Rect, f32)],
    pub damage_overlay: bool,
    pub visible: bool,
}

/// How long a flashing rect stays lit, milliseconds.
pub(super) const FLASH_MS: f32 = 600.;
/// Flashing rects kept on screen.
const FLASH_QUADS: usize = 32;

/// A flashing rect's fill and border at `age` milliseconds: pink, fading out,
/// pulsing at ~18 Hz so a repaint reads as a flash rather than a stain.
/// `None` once it has faded out.
pub(super) fn flash_fill(age_ms: f32) -> Option<(Color, Color)> {
    if !(0. ..FLASH_MS).contains(&age_ms) {
        return None;
    }
    let fade = 1. - age_ms / FLASH_MS;
    let pulse = 0.55 + 0.45 * (age_ms * 0.11).sin();
    let alpha = fade * pulse;
    Some((
        Color(255, 60, 100, (alpha * 130.) as u8),
        Color(255, 90, 130, (alpha * 210.) as u8),
    ))
}

const PAD: f32 = 10.;
const WIDTH: f32 = 376.;
const ROW_H: f32 = 15.;
const GRAPH_H: f32 = 62.;
const BARS: usize = 86;
const SLOT: f32 = 4.;
const BAR_W: f32 = 3.1;
const STACK_H: f32 = 8.;
const OUTLINES: usize = 16;

const BG: Color = Color(20, 17, 24, 255);
const BORDER: Color = Color(255, 255, 255, 24);
const TEXT: Color = Color(214, 222, 235, 255);
const DIM: Color = Color(136, 148, 168, 255);
const GOOD: Color = Color(150, 214, 170, 255);
const WARN: Color = Color(233, 190, 120, 255);
const BAD: Color = Color(237, 130, 150, 255);

fn phase_color(name: &str) -> Color {
    match name {
        "input" => Color(129, 161, 246, 255),
        "script" => Color(219, 52, 242, 255),
        "update" => Color(160, 208, 136, 255),
        "style" => Color(122, 206, 160, 255),
        "layout" => Color(232, 184, 112, 255),
        "text" => Color(214, 196, 140, 255),
        "damage" => Color(247, 128, 160, 255),
        "paint" => Color(150, 196, 255, 255),
        "encode" => Color(96, 190, 200, 255),
        "submit" => Color(176, 172, 226, 255),
        "present" => Color(214, 186, 250, 255),
        "wait" => Color(70, 64, 88, 255),
        _ => Color(120, 130, 150, 255),
    }
}

struct Row {
    text: String,
    y: f32,
    color: Color,
    size: f32,
}

struct Content {
    rows: Vec<Row>,
    graph_y: f32,
    stack_y: f32,
    height: f32,
}

#[derive(Default)]
pub struct Hud {
    root: Option<NodeId>,
    panel: Option<NodeId>,
    panel_state: Option<(Rect, QuadStyle)>,
    rows: Vec<NodeId>,
    row_state: Vec<(String, Color, f32)>,
    bars: Vec<NodeId>,
    bar_state: Vec<(Rect, Color)>,
    segments: Vec<NodeId>,
    segment_state: Vec<(Rect, Color)>,
    outlines: Vec<NodeId>,
    outline_state: Vec<Rect>,
    flashes: Vec<NodeId>,
    flash_state: Vec<(Rect, Color)>,
    origin: Option<(f32, f32)>,
    pub bounds: Option<Rect>,
}

impl Hud {
    /// The HUD's root node, above the app (the window's dev overlays order
    /// themselves once, after both are drawn).
    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    pub fn update(&mut self, ui: &Ui, model: &Model) -> Vec<Rect> {
        let mut scene = ui.scene.borrow_mut();
        if !model.visible {
            return self.clear(&mut scene);
        }
        let viewport = scene.bounds(scene.root());
        let root = self.ensure_root(&mut scene, viewport);
        let content = build(model);
        let x = (viewport.width - WIDTH - 12.).max(0.);
        let y = (viewport.height - content.height - 76.).max(52.);
        let panel = Rect::new(x, y, WIDTH, content.height);
        let before = self.bounds.replace(panel);
        let mut panel_changed = self.draw_panel(&mut scene, root, panel);
        panel_changed |= self.draw_rows(&mut scene, root, panel, &content.rows);
        panel_changed |= self.draw_graph(&mut scene, root, panel, &content, model);
        panel_changed |= self.draw_stack(&mut scene, root, panel, &content, model);
        let mut changed = self.draw_outlines(&mut scene, root, model);
        changed.extend(self.draw_flashes(&mut scene, root, model));
        self.origin = Some((panel.x, panel.y));
        if panel_changed {
            changed.push(panel);
            changed.extend(before.filter(|before| *before != panel));
        }
        changed
    }

    fn clear(&mut self, scene: &mut Scene) -> Vec<Rect> {
        let Some(root) = self.root.take() else {
            return Vec::new();
        };
        let shown: Vec<Rect> = self
            .bounds
            .into_iter()
            .chain(self.outline_state.iter().copied())
            .chain(self.flash_state.iter().map(|(rect, _)| *rect))
            .filter(|rect| rect.width > 0.)
            .collect();
        self.panel = None;
        self.panel_state = None;
        self.rows.clear();
        self.row_state.clear();
        self.bars.clear();
        self.bar_state.clear();
        self.segments.clear();
        self.segment_state.clear();
        self.outlines.clear();
        self.outline_state.clear();
        self.flashes.clear();
        self.flash_state.clear();
        self.origin = None;
        self.bounds = None;
        if scene.contains(root) {
            scene.remove(root);
        }
        shown
    }

    fn ensure_root(&mut self, scene: &mut Scene, viewport: Rect) -> NodeId {
        let scene_root = scene.root();
        let root = match self.root.filter(|root| scene.contains(*root)) {
            Some(root) => root,
            None => {
                let root = scene.append(
                    scene_root,
                    NodeKind::Container(Layout::Overlay),
                    Style {
                        absolute: true,
                        width: Some(viewport.width),
                        height: Some(viewport.height),
                        ..Style::default()
                    },
                );
                scene.set_inert(root, true);
                self.root = Some(root);
                self.panel = None;
                self.panel_state = None;
                self.rows.clear();
                self.row_state.clear();
                self.origin = None;
                root
            }
        };
        scene.set_style(
            root,
            Style {
                absolute: true,
                width: Some(viewport.width),
                height: Some(viewport.height),
                ..Style::default()
            },
        );
        root
    }

    fn draw_panel(&mut self, scene: &mut Scene, root: NodeId, panel: Rect) -> bool {
        let node = match self.panel.filter(|panel| scene.contains(*panel)) {
            Some(node) => node,
            None => {
                let node = scene.append(
                    root,
                    NodeKind::Quad(QuadStyle::default()),
                    Style {
                        absolute: true,
                        ..Style::default()
                    },
                );
                self.panel = Some(node);
                self.panel_state = None;
                node
            }
        };
        let quad = QuadStyle {
            fill: BG,
            radius: 8.,
            border_color: BORDER,
            border_width: 1.,
            ..QuadStyle::default()
        };
        if self.panel_state.as_ref() == Some(&(panel, quad.clone())) {
            return false;
        }
        self.panel_state = Some((panel, quad.clone()));
        place(scene, node, panel, quad);
        true
    }

    fn draw_rows(&mut self, scene: &mut Scene, root: NodeId, panel: Rect, rows: &[Row]) -> bool {
        while self.rows.len() > rows.len() {
            let node = self.rows.pop().expect("a row");
            self.row_state.pop();
            if scene.contains(node) {
                scene.remove(node);
            }
        }
        let moved = self.origin != Some((panel.x, panel.y));
        let existing = self.rows.len();
        let mut changed = false;
        while self.rows.len() < rows.len() {
            let node = scene.append(
                root,
                NodeKind::Text {
                    text: Arc::from(""),
                    color: TEXT,
                    font_size: 11.,
                },
                Style {
                    absolute: true,
                    width: Some(WIDTH - PAD * 2.),
                    ..Style::default()
                },
            );
            scene.set_font(
                node,
                FontStyle {
                    family: FontFamily::Monospace,
                    weight: 400,
                    ..FontStyle::default()
                },
            );
            self.rows.push(node);
            self.row_state.push((String::new(), Color(0, 0, 0, 0), 0.));
            changed = true;
        }
        for (i, row) in rows.iter().enumerate() {
            let node = self.rows[i];
            let state = &mut self.row_state[i];
            if state.0 != row.text || state.1 != row.color || state.2 != row.size {
                *state = (row.text.clone(), row.color, row.size);
                scene.set_kind(
                    node,
                    NodeKind::Text {
                        text: Arc::from(row.text.as_str()),
                        color: row.color,
                        font_size: row.size,
                    },
                );
                changed = true;
            }
            if moved || i >= existing {
                scene.set_transform(
                    node,
                    Transform {
                        x: panel.x + PAD,
                        y: panel.y + row.y,
                    },
                );
            }
        }
        changed
    }

    fn draw_graph(
        &mut self,
        scene: &mut Scene,
        root: NodeId,
        panel: Rect,
        content: &Content,
        model: &Model,
    ) -> bool {
        while self.bars.len() < BARS + 1 {
            let node = scene.append(
                root,
                NodeKind::Quad(QuadStyle::default()),
                Style {
                    absolute: true,
                    ..Style::default()
                },
            );
            self.bars.push(node);
            self.bar_state.push((Rect::default(), Color(0, 0, 0, 0)));
        }
        let mut changed = false;
        let x0 = panel.x + PAD;
        let y0 = panel.y + content.graph_y;
        let scale = model
            .graph
            .iter()
            .map(|bar| bar.ms)
            .fold(16.7_f32, |max, ms| max.max(ms))
            .clamp(16.7, 240.);
        let first = BARS.saturating_sub(model.graph.len());
        for i in 0..BARS {
            let (rect, color) = match model.graph.get(i + first) {
                Some(bar) => {
                    let height = (bar.ms / scale * GRAPH_H).clamp(1., GRAPH_H);
                    let color = bar.dominant.map_or_else(
                        || match bar.ms {
                            ms if ms <= 9. => GOOD,
                            ms if ms <= 18. => Color(150, 200, 140, 255),
                            ms if ms <= 34. => WARN,
                            _ => BAD,
                        },
                        phase_color,
                    );
                    (
                        Rect::new(x0 + i as f32 * SLOT, y0 + GRAPH_H - height, BAR_W, height),
                        color,
                    )
                }
                None => (Rect::new(x0, y0, 0., 0.), Color(0, 0, 0, 0)),
            };
            let state = &mut self.bar_state[i];
            if *state != (rect, color) {
                *state = (rect, color);
                place(
                    scene,
                    self.bars[i],
                    rect,
                    QuadStyle {
                        fill: color,
                        ..QuadStyle::default()
                    },
                );
                changed = true;
            }
        }
        let budget = (16.7 / scale * GRAPH_H).clamp(0., GRAPH_H);
        let line = Rect::new(x0, y0 + GRAPH_H - budget, WIDTH - PAD * 2., 1.);
        let color = Color(255, 255, 255, 44);
        let state = &mut self.bar_state[BARS];
        if *state != (line, color) {
            *state = (line, color);
            place(
                scene,
                self.bars[BARS],
                line,
                QuadStyle {
                    fill: color,
                    ..QuadStyle::default()
                },
            );
            changed = true;
        }
        changed
    }

    fn draw_stack(
        &mut self,
        scene: &mut Scene,
        root: NodeId,
        panel: Rect,
        content: &Content,
        model: &Model,
    ) -> bool {
        let phases: Vec<&PhaseCost> = model
            .summary
            .phase_micros
            .iter()
            .filter(|phase| phase.name != WAIT)
            .collect();
        let total: f32 = phases.iter().map(|phase| phase.micros).sum();
        let width = WIDTH - PAD * 2.;
        let x0 = panel.x + PAD;
        let y0 = panel.y + content.stack_y;
        while self.segments.len() < phases.len() {
            let node = scene.append(
                root,
                NodeKind::Quad(QuadStyle::default()),
                Style {
                    absolute: true,
                    ..Style::default()
                },
            );
            self.segments.push(node);
            self.segment_state
                .push((Rect::default(), Color(0, 0, 0, 0)));
        }
        let mut changed = false;
        let mut x = x0;
        for (i, phase) in phases.iter().enumerate() {
            let fraction = if total > 0. { phase.micros / total } else { 0. };
            let segment = Rect::new(x, y0, (width * fraction).max(0.), STACK_H);
            x += segment.width;
            let color = phase_color(phase.name);
            let state = &mut self.segment_state[i];
            if *state != (segment, color) {
                *state = (segment, color);
                place(
                    scene,
                    self.segments[i],
                    segment,
                    QuadStyle {
                        fill: color,
                        ..QuadStyle::default()
                    },
                );
                changed = true;
            }
        }
        for i in phases.len()..self.segments.len() {
            let state = &mut self.segment_state[i];
            if state.0.width > 0. {
                *state = (Rect::default(), Color(0, 0, 0, 0));
                place(
                    scene,
                    self.segments[i],
                    Rect::default(),
                    QuadStyle::default(),
                );
                changed = true;
            }
        }
        changed
    }

    fn draw_outlines(&mut self, scene: &mut Scene, root: NodeId, model: &Model) -> Vec<Rect> {
        let rects: Vec<Rect> = if model.damage_overlay {
            model.damage.iter().take(OUTLINES).copied().collect()
        } else {
            Vec::new()
        };
        while self.outlines.len() < OUTLINES {
            let node = scene.append(
                root,
                NodeKind::Quad(QuadStyle::default()),
                Style {
                    absolute: true,
                    ..Style::default()
                },
            );
            self.outlines.push(node);
            self.outline_state.push(Rect::default());
        }
        let mut changed = Vec::new();
        for i in 0..OUTLINES {
            let rect = rects.get(i).copied().unwrap_or_default();
            if self.outline_state[i] != rect {
                let old = std::mem::replace(&mut self.outline_state[i], rect);
                changed.extend([old, rect].into_iter().filter(|rect| rect.width > 0.));
                let fill = if rect.width > 0. {
                    Color(255, 140, 90, 22)
                } else {
                    Color(0, 0, 0, 0)
                };
                place(
                    scene,
                    self.outlines[i],
                    rect,
                    QuadStyle {
                        fill,
                        border_color: Color(255, 150, 100, 210),
                        border_width: 1.5,
                        border_over: true,
                        ..QuadStyle::default()
                    },
                );
            }
        }
        changed
    }

    fn draw_flashes(&mut self, scene: &mut Scene, root: NodeId, model: &Model) -> Vec<Rect> {
        while self.flashes.len() < FLASH_QUADS {
            let node = scene.append(
                root,
                NodeKind::Quad(QuadStyle::default()),
                Style {
                    absolute: true,
                    ..Style::default()
                },
            );
            self.flashes.push(node);
            self.flash_state.push((Rect::default(), Color(0, 0, 0, 0)));
        }
        let mut changed = Vec::new();
        for i in 0..FLASH_QUADS {
            let (rect, fill, border) =
                match model.flashes.get(i).and_then(|&(rect, age)| {
                    flash_fill(age).map(|(fill, border)| (rect, fill, border))
                }) {
                    Some(placed) => placed,
                    None => (Rect::default(), Color(0, 0, 0, 0), Color(0, 0, 0, 0)),
                };
            if self.flash_state[i] != (rect, fill) {
                let (old, _) = std::mem::replace(&mut self.flash_state[i], (rect, fill));
                changed.extend([old, rect].into_iter().filter(|rect| rect.width > 0.));
                place(
                    scene,
                    self.flashes[i],
                    rect,
                    QuadStyle {
                        fill,
                        border_color: border,
                        border_width: 1.5,
                        border_over: true,
                        ..QuadStyle::default()
                    },
                );
            }
        }
        changed
    }
}

fn build(model: &Model) -> Content {
    let summary = model.summary;
    let live = !summary.idle;
    let fps_color = match summary.fps {
        _ if !live => DIM,
        fps if fps >= 50. => GOOD,
        fps if fps >= 25. => WARN,
        _ => BAD,
    };
    let frame_color = match summary.p99_ms {
        _ if !live => DIM,
        ms if ms <= 20. => GOOD,
        ms if ms <= 34. => WARN,
        _ => BAD,
    };
    let mut rows = Vec::with_capacity(10);
    let mut y = PAD;
    let push = |text: String, color: Color, size: f32, rows: &mut Vec<Row>, y: &mut f32| {
        rows.push(Row {
            text,
            y: *y,
            color,
            size,
        });
        *y += ROW_H;
    };
    push("zgx profiler".into(), TEXT, 12.5, &mut rows, &mut y);
    y += 2.;
    push(
        format!(
            "{:>5.1} fps{}  ·  {} frames  ·  {} hud",
            summary.fps,
            if live { "" } else { "  (idle)" },
            summary.app_frames,
            summary.hud_frames
        ),
        fps_color,
        11.,
        &mut rows,
        &mut y,
    );
    push(
        format!(
            "frame  p50 {:.1}  p95 {:.1}  p99 {:.1} ms",
            summary.p50_ms, summary.p95_ms, summary.p99_ms
        ),
        frame_color,
        11.,
        &mut rows,
        &mut y,
    );
    push(
        format!(
            "damage  {} rect{}  ·  {}  ·  {} dropped",
            summary.damage_rects,
            if summary.damage_rects == 1 { "" } else { "s" },
            area(summary.damage_area),
            summary.dropped
        ),
        TEXT,
        11.,
        &mut rows,
        &mut y,
    );
    push(
        format!(
            "reactive  {} writes  ·  {} effects  ·  ×{:.1}",
            summary.writes, summary.effects, summary.cascade
        ),
        TEXT,
        11.,
        &mut rows,
        &mut y,
    );
    let thrash = match model.thrashing.len() {
        0 => format!(
            "thrash  none  ·  {:.0} µs this frame",
            summary.frame_micros()
        ),
        1 => {
            let (signal, count) = model.thrashing[0];
            format!(
                "thrash  signal {signal:#x} ×{count}  ·  {:.0} µs",
                summary.frame_micros()
            )
        }
        n => format!("thrash  {n} signals  ·  {:.0} µs", summary.frame_micros()),
    };
    push(
        thrash,
        if model.thrashing.is_empty() { DIM } else { BAD },
        11.,
        &mut rows,
        &mut y,
    );
    if !summary.slow.is_empty() {
        let shown: Vec<String> = summary
            .slow
            .iter()
            .take(2)
            .map(|run| format!("{} {}", cut(&run.name, 24), micros(run.micros)))
            .collect();
        push(
            format!("slow  {}", shown.join("  ·  ")),
            if summary.slow[0].micros >= 2000. {
                WARN
            } else {
                TEXT
            },
            11.,
            &mut rows,
            &mut y,
        );
    } else {
        push(String::new(), TEXT, 11., &mut rows, &mut y);
    }
    let why = why(summary);
    if !why.is_empty() {
        push(
            format!("why  {}", cut(&why, 46)),
            DIM,
            11.,
            &mut rows,
            &mut y,
        );
    } else {
        push(String::new(), DIM, 11., &mut rows, &mut y);
    }
    if let Some(worst) = &summary.worst {
        let worst_color = match worst.interval_ms {
            ms if ms > 34. => BAD,
            ms if ms > 20. => WARN,
            _ => DIM,
        };
        let dominant = worst
            .dominant()
            .map(|phase| format!("  ·  {} {}", phase.name, micros(phase.micros)))
            .unwrap_or_default();
        let slow = worst
            .slow
            .first()
            .map(|run| format!("  ·  {} {}", cut(&run.name, 14), micros(run.micros)))
            .unwrap_or_default();
        push(
            format!("worst  {:.1} ms{dominant}{slow}", worst.interval_ms),
            worst_color,
            11.,
            &mut rows,
            &mut y,
        );
    } else {
        push(String::new(), DIM, 11., &mut rows, &mut y);
    }
    y += 4.;
    let graph_y = y;
    y += GRAPH_H + 6.;
    let stack_y = y;
    y += STACK_H + 6.;
    let legend: Vec<String> = {
        let mut phases = summary.phase_micros.clone();
        phases.sort_by(|a, b| b.micros.total_cmp(&a.micros));
        let work = phases.iter().filter(|phase| phase.name != WAIT).take(3);
        let wait = phases.iter().filter(|phase| phase.name == WAIT);
        work.chain(wait)
            .map(|phase| format!("{} {:.0}", phase.name, phase.micros))
            .collect()
    };
    push(
        if legend.is_empty() {
            "phases  idle".into()
        } else {
            legend.join(" · ")
        },
        DIM,
        10.,
        &mut rows,
        &mut y,
    );
    push(
        format!(
            "{}  ·  rss {:.0} MiB  ·  gpu {}",
            instructions(summary.instructions),
            summary.rss_mib,
            summary
                .gpu_micros
                .map_or_else(|| "-".to_string(), |micros| format!("{micros:.0} µs"))
        ),
        DIM,
        10.,
        &mut rows,
        &mut y,
    );
    push(
        format!(
            "F12 hud  ·  F11 damage {}  ·  F10 reset",
            if model.damage_overlay { "on" } else { "off" }
        ),
        DIM,
        10.,
        &mut rows,
        &mut y,
    );
    Content {
        rows,
        graph_y,
        stack_y,
        height: y + PAD - 4.,
    }
}

fn place(scene: &mut Scene, node: NodeId, rect: Rect, quad: QuadStyle) {
    scene.set_style(
        node,
        Style {
            absolute: true,
            width: Some(rect.width.max(0.)),
            height: Some(rect.height.max(0.)),
            ..Style::default()
        },
    );
    scene.set_kind(node, NodeKind::Quad(quad));
    scene.set_transform(
        node,
        Transform {
            x: rect.x,
            y: rect.y,
        },
    );
}

/// Why the last app frame did what it did: its first note, the rest counted,
/// a whole-window repaint named where the host said so.
fn why(summary: &Summary) -> String {
    let mut why = String::new();
    if let Some(note) = summary.notes.first() {
        why.push_str(note);
        if summary.notes.len() > 1 {
            why.push_str(&format!(" (+{})", summary.notes.len() - 1));
        }
    }
    if summary.full {
        if !why.is_empty() {
            why.push_str("  ·  ");
        }
        why.push_str("full repaint");
    }
    why
}

fn micros(micros: f32) -> String {
    if micros >= 1000. {
        format!("{:.1} ms", micros / 1000.)
    } else {
        format!("{micros:.0} µs")
    }
}

fn instructions(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M instr", count as f64 / 1e6)
    } else if count >= 1_000 {
        format!("{:.0}k instr", count as f64 / 1e3)
    } else if count > 0 {
        format!("{count} instr")
    } else {
        "-".to_string()
    }
}

fn cut(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(width - 1).collect();
    cut.push('…');
    cut
}

fn area(points: f32) -> String {
    if points >= 1_000_000. {
        format!("{:.1} Mpx²", points / 1_000_000.)
    } else if points >= 1_000. {
        format!("{:.0} kpx²", points / 1_000.)
    } else {
        format!("{points:.0} px²")
    }
}
