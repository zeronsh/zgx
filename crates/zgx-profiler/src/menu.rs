//! The profiler's dev menu: the one clickable thing the HUD offers. The
//! HUD's own tree is inert so the app under it works untouched; this menu is
//! a second root, not inert, sized to nothing until its button is pressed,
//! so clicks anywhere else pass the app by as before.

use std::{rc::Rc, sync::Arc};

use zgx::{
    input::{EventContext, EventPhase, InputBinding, InputEvent, PointerButton},
    scene::{Color, Layout, NodeId, NodeKind, QuadStyle, Rect, Scene, Style, Transform},
    text_layout::{FontFamily, FontStyle},
    widgets::Ui,
};

use super::Inner;

pub(super) const MENU_W: f32 = 170.;
/// Rows: flash repaints, log frames, stress test, break app, one core.
pub(super) const ROWS: usize = 5;
const ROW_H: f32 = 22.;
const ROW_GAP: f32 = 2.;
const PAD: f32 = 8.;
const BUTTON: f32 = 18.;
const ICON_BARS: usize = 3;
const ICON_W: f32 = 10.;
const ICON_BAR: f32 = 1.5;
const ICON_GAP: f32 = 2.5;

const TEXT: Color = Color(214, 222, 235, 255);
const DIM: Color = Color(136, 148, 168, 255);
const GOOD: Color = Color(150, 214, 170, 255);
const WARN: Color = Color(233, 190, 120, 255);
const BAD: Color = Color(237, 130, 150, 255);
const CARD: Color = Color(20, 17, 24, 250);
const BORDER: Color = Color(255, 255, 255, 24);
const HOVER: Color = Color(255, 255, 255, 20);

#[derive(Default)]
pub(super) struct Menu {
    root: Option<NodeId>,
    button: Option<NodeId>,
    button_state: Option<(Rect, Color)>,
    button_bars: Vec<NodeId>,
    button_bars_state: Option<(Rect, Color)>,
    card: Option<NodeId>,
    card_state: Option<Rect>,
    rows: Vec<NodeId>,
    row_state: Vec<Option<(Rect, Color)>>,
    labels: Vec<NodeId>,
    label_state: Vec<Option<(String, Color)>>,
    states: Vec<NodeId>,
    state_state: Vec<Option<(String, Color)>>,
    bindings: Vec<InputBinding>,
    /// Where the interactive parts are, for the damage figures' exclusion.
    pub bounds: Option<Rect>,
}

impl Menu {
    /// Draw the menu for `inner`'s window: the button while the HUD shows,
    /// the rows while it is open. Nothing inert: the button and rows listen.
    pub fn update(&mut self, ui: &Ui, inner: &Rc<Inner>) {
        let mut scene = ui.scene.borrow_mut();
        let Some(panel) = inner.hud_rect.get() else {
            self.clear(&mut scene);
            return;
        };
        let root = self.ensure_root(&mut scene);
        let open = inner.menu_open.get();
        let button = Rect::new(
            panel.x + panel.width - BUTTON - 8.,
            panel.y + 7.,
            BUTTON,
            BUTTON,
        );
        let hovered = inner.menu_hover.get();
        let button_bg = match hovered == Some(usize::MAX) {
            true => HOVER,
            false => Color(255, 255, 255, 12),
        };
        let button = self.ensure_button(&mut scene, root, ui, inner, button, button_bg);
        let card = match open {
            true => Some(self.ensure_card(&mut scene, root, ui, inner, panel)),
            false => {
                self.take_card(&mut scene);
                None
            }
        };
        let mut interactive = button;
        if let Some(card) = card {
            interactive = interactive.union(card);
        }
        self.bounds = Some(interactive);
    }

    fn clear(&mut self, scene: &mut Scene) {
        let Some(root) = self.root.take() else {
            return;
        };
        self.button = None;
        self.button_state = None;
        self.button_bars.clear();
        self.button_bars_state = None;
        self.card = None;
        self.card_state = None;
        self.rows.clear();
        self.row_state.clear();
        self.labels.clear();
        self.label_state.clear();
        self.states.clear();
        self.state_state.clear();
        self.bindings.clear();
        self.bounds = None;
        if scene.contains(root) {
            scene.remove(root);
        }
    }

    fn ensure_root(&mut self, scene: &mut Scene) -> NodeId {
        match self.root.filter(|root| scene.contains(*root)) {
            Some(root) => root,
            None => {
                let scene_root = scene.root();
                let root = scene.append(
                    scene_root,
                    NodeKind::Container(Layout::Overlay),
                    Style {
                        absolute: true,
                        width: Some(scene.bounds(scene_root).width),
                        height: Some(scene.bounds(scene_root).height),
                        ..Style::default()
                    },
                );
                self.root = Some(root);
                root
            }
        }
    }

    /// The menu's root node, above the HUD.
    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    fn ensure_button(
        &mut self,
        scene: &mut Scene,
        root: NodeId,
        ui: &Ui,
        inner: &Rc<Inner>,
        rect: Rect,
        bg: Color,
    ) -> Rect {
        let node = match self.button.filter(|node| scene.contains(*node)) {
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
                self.button_bars = (0..ICON_BARS)
                    .map(|_| {
                        scene.append(
                            root,
                            NodeKind::Quad(QuadStyle::default()),
                            Style {
                                absolute: true,
                                ..Style::default()
                            },
                        )
                    })
                    .collect();
                self.bindings.push(ui.input.listen(node, {
                    let inner = inner.clone();
                    move |cx| button_event(&inner, cx)
                }));
                self.button = Some(node);
                self.button_bars_state = None;
                node
            }
        };
        if self.button_state.as_ref() != Some(&(rect, bg)) {
            self.button_state = Some((rect, bg));
            place(
                scene,
                node,
                rect,
                QuadStyle {
                    fill: bg,
                    radius: 5.,
                    ..QuadStyle::default()
                },
            );
        }
        let color = match inner.menu_hover.get() == Some(usize::MAX) {
            true => TEXT,
            false => DIM,
        };
        if self.button_bars_state != Some((rect, color)) {
            self.button_bars_state = Some((rect, color));
            let x = rect.x + (rect.width - ICON_W) / 2.;
            let top = rect.y + (rect.height - icon_height()) / 2.;
            for (i, bar) in self.button_bars.iter().enumerate() {
                place(
                    scene,
                    *bar,
                    Rect::new(x, top + i as f32 * (ICON_BAR + ICON_GAP), ICON_W, ICON_BAR),
                    QuadStyle {
                        fill: color,
                        radius: ICON_BAR / 2.,
                        ..QuadStyle::default()
                    },
                );
            }
        }
        rect
    }

    fn ensure_card(
        &mut self,
        scene: &mut Scene,
        root: NodeId,
        ui: &Ui,
        inner: &Rc<Inner>,
        panel: Rect,
    ) -> Rect {
        let card = Rect::new(
            (panel.x + panel.width - MENU_W - 12.).max(4.),
            (panel.y - card_height() - 8.).max(4.),
            MENU_W,
            card_height(),
        );
        let node = match self.card.filter(|node| scene.contains(*node)) {
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
                self.card = Some(node);
                self.card_state = None;
                node
            }
        };
        if self.card_state != Some(card) {
            self.card_state = Some(card);
            place(
                scene,
                node,
                card,
                QuadStyle {
                    fill: CARD,
                    radius: 8.,
                    border_color: BORDER,
                    border_width: 1.,
                    ..QuadStyle::default()
                },
            );
        }
        self.ensure_rows(scene, root, ui, inner, card);
        card
    }

    fn take_card(&mut self, scene: &mut Scene) {
        if let Some(card) = self.card.take().filter(|card| scene.contains(*card)) {
            scene.remove(card);
        }
        self.card_state = None;
        for row in self.rows.drain(..) {
            if scene.contains(row) {
                scene.remove(row);
            }
        }
        for label in self.labels.drain(..) {
            if scene.contains(label) {
                scene.remove(label);
            }
        }
        for state in self.states.drain(..) {
            if scene.contains(state) {
                scene.remove(state);
            }
        }
        self.row_state.clear();
        self.label_state.clear();
        self.state_state.clear();
    }

    fn ensure_rows(
        &mut self,
        scene: &mut Scene,
        root: NodeId,
        ui: &Ui,
        inner: &Rc<Inner>,
        rect: Rect,
    ) {
        while self.rows.len() < ROWS {
            let index = self.rows.len();
            let row = scene.append(
                root,
                NodeKind::Quad(QuadStyle::default()),
                Style {
                    absolute: true,
                    ..Style::default()
                },
            );
            let label = scene.append(
                root,
                NodeKind::Text {
                    text: Arc::from(""),
                    color: TEXT,
                    font_size: 11.,
                },
                Style {
                    absolute: true,
                    ..Style::default()
                },
            );
            scene.set_font(
                label,
                FontStyle {
                    family: FontFamily::Monospace,
                    weight: 400,
                    ..FontStyle::default()
                },
            );
            let state = scene.append(
                root,
                NodeKind::Text {
                    text: Arc::from(""),
                    color: DIM,
                    font_size: 11.,
                },
                Style {
                    absolute: true,
                    ..Style::default()
                },
            );
            scene.set_font(
                state,
                FontStyle {
                    family: FontFamily::Monospace,
                    weight: 400,
                    ..FontStyle::default()
                },
            );
            self.bindings.push(ui.input.listen(row, {
                let inner = inner.clone();
                move |cx| row_event(&inner, index, cx)
            }));
            self.rows.push(row);
            self.row_state.push(None);
            self.labels.push(label);
            self.label_state.push(None);
            self.states.push(state);
            self.state_state.push(None);
        }
        for index in 0..ROWS {
            let row_rect = Rect::new(
                rect.x + PAD,
                rect.y + PAD + index as f32 * (ROW_H + ROW_GAP),
                MENU_W - PAD * 2.,
                ROW_H,
            );
            let hovered = inner.menu_hover.get() == Some(index);
            let (label, state, state_color) = row_face(inner, index);
            let row = self.rows[index];
            let bg = match hovered {
                true => HOVER,
                false => Color(0, 0, 0, 0),
            };
            if self.row_state[index] != Some((row_rect, bg)) {
                self.row_state[index] = Some((row_rect, bg));
                place(
                    scene,
                    row,
                    row_rect,
                    QuadStyle {
                        fill: bg,
                        radius: 5.,
                        ..QuadStyle::default()
                    },
                );
            }
            let label_node = self.labels[index];
            if self.label_state[index]
                .as_ref()
                .map(|(known, _)| known.as_str())
                != Some(label)
            {
                self.label_state[index] = Some((label.to_string(), TEXT));
                scene.set_kind(
                    label_node,
                    NodeKind::Text {
                        text: Arc::from(label),
                        color: TEXT,
                        font_size: 11.,
                    },
                );
            }
            scene.set_transform(
                label_node,
                Transform {
                    x: row_rect.x + 8.,
                    y: row_rect.y + (ROW_H - 11.) / 2.,
                },
            );
            let state_node = self.states[index];
            let known = self.state_state[index].clone();
            if known.as_ref().map(|(known, _)| known.as_str()) != Some(state)
                || known.map(|(_, color)| color) != Some(state_color)
            {
                self.state_state[index] = Some((state.to_string(), state_color));
                scene.set_kind(
                    state_node,
                    NodeKind::Text {
                        text: Arc::from(state),
                        color: state_color,
                        font_size: 11.,
                    },
                );
            }
            scene.set_transform(
                state_node,
                Transform {
                    x: row_rect.x + row_rect.width - 44.,
                    y: row_rect.y + (ROW_H - 11.) / 2.,
                },
            );
        }
    }
}

fn icon_height() -> f32 {
    ICON_BARS as f32 * ICON_BAR + (ICON_BARS - 1) as f32 * ICON_GAP
}

fn card_height() -> f32 {
    PAD * 2. + ROWS as f32 * ROW_H + (ROWS - 1) as f32 * ROW_GAP
}

/// A row's label, its state word and the state's colour.
fn row_face(inner: &Rc<Inner>, index: usize) -> (&'static str, &'static str, Color) {
    match index {
        0 => match inner.flash.get() {
            true => ("flash repaints", "on", GOOD),
            false => ("flash repaints", "off", DIM),
        },
        1 => match inner.log_on.get() {
            true => ("log frames", "on", GOOD),
            false => ("log frames", "off", DIM),
        },
        2 => match inner.stressing.get() {
            true => ("stress test", "run", WARN),
            false => ("stress test", "idle", DIM),
        },
        3 => ("break app", "", BAD),
        _ => match (inner.serial.get(), crate::serial::supported()) {
            (true, _) => ("one core", "on", GOOD),
            (false, true) => ("one core", "off", DIM),
            (false, false) => ("one core", "n/a", DIM),
        },
    }
}

fn button_event(inner: &Rc<Inner>, cx: &mut EventContext) {
    if cx.phase == EventPhase::Capture {
        return;
    }
    match cx.event.clone() {
        InputEvent::PointerDown {
            button: PointerButton::Primary,
            ..
        }
        | InputEvent::Activate => {
            cx.prevent_default();
            inner.menu_open.set(!inner.menu_open.get());
            inner.menu_hover.set(None);
            inner.refresh();
        }
        InputEvent::PointerEnter => {
            inner.menu_hover.set(Some(usize::MAX));
            inner.refresh();
        }
        InputEvent::PointerLeave if inner.menu_hover.get() == Some(usize::MAX) => {
            inner.menu_hover.set(None);
            inner.refresh();
        }
        _ => {}
    }
}

fn row_event(inner: &Rc<Inner>, index: usize, cx: &mut EventContext) {
    if cx.phase == EventPhase::Capture {
        return;
    }
    match cx.event.clone() {
        InputEvent::PointerDown {
            button: PointerButton::Primary,
            ..
        }
        | InputEvent::Activate => {
            cx.prevent_default();
            inner.menu_clicked(index);
        }
        InputEvent::PointerEnter => {
            inner.menu_hover.set(Some(index));
            inner.refresh();
        }
        InputEvent::PointerLeave if inner.menu_hover.get() == Some(index) => {
            inner.menu_hover.set(None);
            inner.refresh();
        }
        _ => {}
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
