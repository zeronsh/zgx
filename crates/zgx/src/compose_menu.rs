//! Retained menu keyboard navigation and trigger metadata.
use crate::{
    compose::{TaskRunner, TaskToken},
    compose_hover::{HoverDelay, SAFE_TIMEOUT, SUBMENU_HOVER, contains, in_safe_polygon},
    input::{EventPhase, InputEvent, Key, PointerButton},
    reactive::Signal,
    scene::NodeId,
    semantics::{PopupKind, Role},
    widgets::Ui,
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};
pub(crate) struct MenuContext {
    pub open: Signal<bool>,
    pub parent: Option<Rc<MenuContext>>,
    pub panel: Cell<Option<NodeId>>,
    overlay: Cell<Option<NodeId>>,
    trigger: Option<NodeId>,
    submenu: bool,
    children: RefCell<Vec<Weak<MenuContext>>>,
    /// When this submenu opens and closes as the pointer rests on, or
    /// leaves, its trigger (`View::hover_delay`).
    pub hover: Cell<HoverDelay>,
    /// The pointer is heading for an open submenu (in its safe polygon, on a
    /// sibling item): moving over the items between stays that menu open.
    hold: Cell<bool>,
    /// A close of its submenu waits on the pointer's close delay: entering
    /// another item closes nothing meanwhile.
    closing: Cell<bool>,
}
impl MenuContext {
    pub(crate) fn new(
        open: Signal<bool>,
        parent: Option<Rc<MenuContext>>,
        trigger: Option<NodeId>,
        submenu: bool,
    ) -> Rc<Self> {
        let context = Rc::new(Self {
            open,
            parent,
            panel: Cell::new(None),
            overlay: Cell::new(None),
            trigger,
            submenu,
            children: RefCell::new(Vec::new()),
            hover: Cell::new(SUBMENU_HOVER),
            hold: Cell::new(false),
            closing: Cell::new(false),
        });
        if let Some(parent) = &context.parent {
            let mut children = parent.children.borrow_mut();
            children.retain(|child| child.strong_count() > 0);
            children.push(Rc::downgrade(&context));
        }
        context
    }
    /// Close this menu's open submenus but `keep` (the one the pointer
    /// moved onto), as moving over another item does.
    pub(crate) fn close_children(&self, keep: Option<&Signal<bool>>) {
        let children: Vec<Rc<MenuContext>> = self
            .children
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        for child in children {
            if keep.is_none_or(|keep| !keep.same(&child.open)) && child.open.with_untracked(|o| *o)
            {
                child.open.set(false);
            }
        }
    }
    /// Whether the pointer is heading for an open submenu of this menu, over
    /// the items between (see `hold`).
    pub(crate) fn holding(&self) -> bool {
        self.hold.get() || self.closing.get()
    }
    /// The hover delay of this menu's submenu `open`.
    pub(crate) fn child_hover(&self, open: &Signal<bool>) -> HoverDelay {
        self.children
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .find(|child| child.open.same(open))
            .map_or(SUBMENU_HOVER, |child| child.hover.get())
    }
    /// This menu's open submenus.
    fn open_children(&self) -> Vec<Rc<MenuContext>> {
        self.children
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|child| child.open.with_untracked(|o| *o))
            .collect()
    }
    /// Every panel under this menu that is open: its open submenus' and theirs.
    fn open_panels(&self, out: &mut Vec<NodeId>) {
        for child in self.open_children() {
            out.extend(child.panel.get());
            child.open_panels(out);
        }
    }
    /// Focus the first enabled item of this menu's open submenu `open`.
    pub(crate) fn focus_submenu(&self, ui: &Ui, open: &Signal<bool>) {
        let panel = self
            .children
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .find(|child| child.open.same(open))
            .and_then(|child| child.panel.get());
        if let Some(first) = panel.and_then(|panel| items(ui, panel).first().copied()) {
            ui.input.focus(&ui.scene, Some(first));
        }
    }
    pub(crate) fn close_chain(&self) {
        crate::compose_modal::closing_by_pick();
        self.open.set(false);
        if let Some(parent) = &self.parent {
            parent.close_chain();
        }
    }
}
/// A menu panel's enabled items, in order.
fn items(ui: &Ui, panel: NodeId) -> Vec<NodeId> {
    let scene = ui.scene.borrow();
    let semantics = ui.semantics.borrow();
    let mut pending = scene
        .children(panel)
        .iter()
        .rev()
        .copied()
        .collect::<Vec<_>>();
    let mut items = Vec::new();
    while let Some(node) = pending.pop() {
        if semantics
            .get(node)
            .is_some_and(|node| node.role == Role::MenuItem)
            && ui.input.is_enabled(&scene, node)
        {
            items.push(node);
        }
        pending.extend(scene.children(node).iter().rev().copied());
    }
    items
}
pub(crate) fn mount(
    ui: &mut Ui,
    panel: NodeId,
    menu: Rc<MenuContext>,
    anchor: NodeId,
    overlay: NodeId,
    dismiss_backdrop: bool,
    runner: Option<Rc<TaskRunner>>,
) {
    menu.overlay.set(Some(overlay));
    mount_safe_polygon(ui, panel, menu.clone(), runner);
    if menu.submenu {
        let current = menu.clone();
        ui.bind(panel, move || {
            if !current.open.get() {
                return;
            }
            let siblings = current
                .parent
                .as_ref()
                .map(|parent| {
                    parent
                        .children
                        .borrow()
                        .iter()
                        .filter_map(Weak::upgrade)
                        .filter(|child| !Rc::ptr_eq(child, &current))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for sibling in siblings {
                sibling.open.set(false);
            }
        });
    }
    let search = Rc::new(RefCell::new(crate::typeahead::Typeahead::default()));
    let reset = search.clone();
    let open = menu.open.clone();
    let semantics = ui.semantics.clone();
    ui.bind(panel, move || {
        let expanded = open.get();
        reset.borrow_mut().reset();
        semantics.borrow_mut().update(anchor, |node| {
            node.has_popup = Some(PopupKind::Menu);
            node.expanded = Some(expanded);
        });
    });
    let weak = ui.downgrade();
    ui.on_event(overlay, false, move |cx| {
        if cx.phase == EventPhase::Capture || cx.default_prevented() {
            return;
        }
        if cx.phase == EventPhase::Target
            && cx.target == overlay
            && let InputEvent::PointerDown {
                x,
                y,
                button: PointerButton::Primary,
            } = cx.event
        {
            if let Some(ui) = weak.upgrade() {
                let mut ancestor = menu.parent.clone();
                let mut destination = None;
                while let Some(parent) = ancestor {
                    if parent.open.get()
                        && let Some(panel) = parent.panel.get()
                    {
                        let scene = ui.scene.borrow();
                        if scene.contains(panel) {
                            let bounds = scene.bounds(panel);
                            if x >= bounds.x
                                && y >= bounds.y
                                && x < bounds.x + bounds.width
                                && y < bounds.y + bounds.height
                            {
                                destination = Some(parent.clone());
                                break;
                            }
                        }
                    }
                    ancestor = parent.parent.clone();
                }
                if destination.is_none() && !dismiss_backdrop {
                    cx.prevent_default();
                    cx.stop_propagation();
                    return;
                }
                let mut closing = Some(menu.clone());
                while let Some(current) = closing {
                    if destination
                        .as_ref()
                        .is_some_and(|target| Rc::ptr_eq(target, &current))
                    {
                        break;
                    }
                    current.open.set(false);
                    if let Some(overlay) = current.overlay.get() {
                        crate::compose_modal::deactivate(&ui, overlay);
                    }
                    closing = current.parent.clone();
                }
                if let Some(parent) = destination
                    && parent.open.get()
                    && parent.overlay.get() == ui.input.focus_scope()
                    && parent
                        .panel
                        .get()
                        .is_some_and(|panel| ui.scene.borrow().contains(panel))
                {
                    ui.input.dispatch(&ui.scene, cx.event.clone());
                }
            }
            cx.prevent_default();
            cx.stop_propagation();
            return;
        }
        let InputEvent::KeyDown { key, modifiers, .. } = &cx.event else {
            return;
        };
        if *key == Key::Tab {
            menu.close_chain();
            cx.stop_propagation();
            return;
        }
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if cx.target != panel
            && cx.target != overlay
            && !ui
                .semantics
                .borrow()
                .get(cx.target)
                .is_some_and(|node| node.role == Role::MenuItem)
        {
            return;
        }
        // A submenu opens towards the line's end: right to left, the left.
        // The panels are in an overlay; the menu's own trigger says which
        // way its text runs.
        let rtl = {
            let mut root = menu.clone();
            while let Some(parent) = root.parent.clone() {
                root = parent;
            }
            let scene = ui.scene.borrow();
            root.trigger
                .filter(|t| scene.contains(*t))
                .is_some_and(|t| scene.direction(t) == crate::scene::Direction::Rtl)
        };
        let (back, into) = if rtl {
            (Key::ArrowRight, Key::ArrowLeft)
        } else {
            (Key::ArrowLeft, Key::ArrowRight)
        };
        if *key == back && menu.submenu {
            menu.open.set(false);
            if let Some(trigger) = menu.trigger {
                ui.input.focus(&ui.scene, Some(trigger));
            }
            cx.prevent_default();
            cx.stop_propagation();
            return;
        }
        if *key == into {
            let child = menu
                .children
                .borrow()
                .iter()
                .filter_map(Weak::upgrade)
                .find(|child| child.submenu && child.trigger == Some(cx.target));
            if let Some(child) = child {
                child.open.set(true);
                cx.prevent_default();
                cx.stop_propagation();
            }
            return;
        }
        let items = items(&ui, panel);
        if items.is_empty() {
            return;
        }
        let current = items
            .iter()
            .position(|node| Some(*node) == ui.input.focused());
        let target = match key {
            Key::ArrowDown => Some(items[current.map_or(0, |index| (index + 1) % items.len())]),
            Key::ArrowUp => Some(
                items[current.map_or(items.len() - 1, |index| {
                    (index + items.len() - 1) % items.len()
                })],
            ),
            Key::Home => Some(items[0]),
            Key::End => items.last().copied(),
            Key::Character(text) if !modifiers.control && !modifiers.alt && !modifiers.meta => {
                if crate::typeahead::bounded_prefix(text).is_none() {
                    return;
                }
                let semantics = ui.semantics.borrow();
                let found = search
                    .borrow_mut()
                    .find(text, items.len(), current, |index| {
                        semantics.get(items[index]).map(|node| node.label.clone())
                    });
                found.map(|index| items[index])
            }
            _ => return,
        };
        if let Some(target) = target {
            ui.input.focus(&ui.scene, Some(target));
        }
        cx.prevent_default();
        cx.stop_propagation();
    });
}

/// A chosen item flashes off and on, then its menu closes and `then` (its
/// action) runs, as AppKit's menus confirm a choice (where menus are windows
/// of their own, but on Windows, whose menus close at once). Choosing again
/// meanwhile does nothing.
pub(crate) fn flash(
    ui: &Ui,
    runner: &crate::compose::TaskRunner,
    menu: Rc<MenuContext>,
    item: NodeId,
    then: impl FnOnce() + 'static,
) {
    thread_local! {
        /// Whether a flash runs (one at most), and its task, kept until the
        /// next.
        static FLASHING: Cell<bool> = const { Cell::new(false) };
        static TASK: RefCell<Option<crate::compose::TaskToken>> = const { RefCell::new(None) };
    }
    if FLASHING.replace(true) {
        return;
    }
    // Off: neither focused nor hovered.
    if let Some(panel) = menu.panel.get() {
        ui.input.focus(&ui.scene, Some(panel));
    }
    ui.input.clear_hover(&ui.scene);
    /// Ends the flash however its task does (dropped with its window too).
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            FLASHING.set(false);
        }
    }
    let weak = ui.downgrade();
    let task = runner.spawn(Box::pin(async move {
        let _done = Done;
        let step = Duration::from_millis(50);
        crate::timer::sleep(step).await;
        if let Some(ui) = weak.upgrade() {
            ui.input.focus(&ui.scene, Some(item));
        }
        crate::timer::sleep(step).await;
        drop(_done);
        menu.close_chain();
        then();
    }));
    TASK.with(|slot| *slot.borrow_mut() = Some(task));
}
/// A menu's trigger opens it as it is pressed, as AppKit's pop-up and
/// pull-down buttons do: dragged on to an item and released there, that
/// item is chosen; released over the trigger (a click), the menu stays open
/// to click in; dragged away and released elsewhere, it closes. A press on
/// the trigger of an open menu lands on its backdrop, which closes it. Only
/// a trigger listening already (a button) is one: a bare anchor (a context
/// menu's point) stays out of hit testing.
pub(crate) fn mount_press(
    ui: &mut Ui,
    anchor: NodeId,
    overlay: NodeId,
    panel: NodeId,
    open: Signal<bool>,
) {
    if !ui.input.has_listeners(anchor) {
        return;
    }
    // Where the press opening the menu was, while it is held. The menu's
    // focus scope cancels the press on the trigger, so its overlay (inside
    // the scope, over the window and holding the items) follows it.
    let pressed = Rc::new(Cell::new(None::<(f32, f32)>));
    let (start, close) = (pressed.clone(), open.clone());
    let binding = ui.input.listen(anchor, move |cx| {
        if cx.phase == EventPhase::Capture || cx.default_prevented() {
            return;
        }
        match cx.event {
            InputEvent::PointerDown {
                x,
                y,
                button: PointerButton::Primary,
            } if !open.with_untracked(|open| *open) => {
                // The press opens the menu, focus going into it, rather
                // than to the trigger.
                cx.prevent_default();
                start.set(Some((x, y)));
                open.set(true);
            }
            // Released before the menu showed (a click in one batch of
            // input): on the trigger, it stays open.
            InputEvent::PointerUp {
                button: PointerButton::Primary,
                ..
            } => start.set(None),
            _ => {}
        }
    });
    ui.retain(anchor, binding);
    let weak = ui.downgrade();
    let binding = ui.input.listen(overlay, move |cx| {
        // Before what is under the pointer hears it (or as the backdrop).
        if cx.phase == EventPhase::Bubble || pressed.get().is_none() {
            return;
        }
        let Some(ui) = weak.upgrade() else {
            return;
        };
        match cx.event {
            // A new press: the one that opened the menu has ended unheard.
            InputEvent::PointerDown { .. } => pressed.set(None),
            InputEvent::PointerMove { x, y } => {
                if let Some(item) = item_at(&ui, panel, x, y)
                    && ui.input.focused() != Some(item)
                {
                    ui.input.focus(&ui.scene, Some(item));
                }
            }
            InputEvent::PointerUp {
                x,
                y,
                button: PointerButton::Primary,
            } => {
                let (px, py) = pressed.take().expect("checked above");
                cx.prevent_default();
                cx.stop_propagation();
                // A click (no drag): it stays open to click in, though an
                // item opened under the pointer (a pop-up button's,
                // `open_over`), as AppKit's does.
                let clicked = (x - px).hypot(y - py) <= cx.drag_threshold();
                if clicked {
                    return;
                }
                if let Some(item) = item_at(&ui, panel, x, y) {
                    ui.input.dispatch_to(&ui.scene, item, InputEvent::Activate);
                    return;
                }
                let trigger = ui.scene.borrow().bounds(anchor);
                let on_trigger = x >= trigger.x
                    && y >= trigger.y
                    && x < trigger.x + trigger.width
                    && y < trigger.y + trigger.height;
                if !on_trigger {
                    close.set(false);
                }
            }
            _ => {}
        }
    });
    ui.retain(anchor, binding);
}
/// The enabled item of the menu `panel` at window point `(x, y)`.
fn item_at(ui: &Ui, panel: NodeId, x: f32, y: f32) -> Option<NodeId> {
    let scene = ui.scene.borrow();
    if !scene.contains(panel) {
        return None;
    }
    let semantics = ui.semantics.borrow();
    let hit = scene.hit_test(x, y)?;
    std::iter::once(hit)
        .chain(scene.ancestors(hit))
        .take_while(|node| *node != panel)
        .find(|node| {
            semantics
                .get(*node)
                .is_some_and(|semantic| semantic.role == Role::MenuItem)
                && ui.input.is_enabled(&scene, *node)
        })
        .filter(|item| scene.is_within(*item, panel))
}

/// Heading from a submenu's trigger to its panel across this menu's other
/// items keeps it open: while the pointer is in the safe polygon from where
/// it left the trigger (`in_safe_polygon`) the items it crosses close and
/// open nothing (`MenuContext::holding`). Out of it, over this menu, the
/// submenu closes after its close delay; resting in it, after a moment.
fn mount_safe_polygon(
    ui: &mut Ui,
    panel: NodeId,
    menu: Rc<MenuContext>,
    runner: Option<Rc<TaskRunner>>,
) {
    let weak = ui.downgrade();
    let exit: Cell<Option<(f32, f32)>> = Cell::new(None);
    // What waits to close the submenu: nothing (0), the safe polygon's
    // timeout (1) or the close delay (2), and its task.
    let waiting: Rc<Cell<u8>> = Rc::default();
    let task: Rc<RefCell<Option<TaskToken>>> = Rc::default();
    ui.on_document_event(panel, move |event| {
        let InputEvent::PointerMove { x, y } = *event else {
            return;
        };
        let p = (x, y);
        let open = menu.open_children();
        let Some(ui) = weak.upgrade() else { return };
        let (Some(child), true) = (open.first(), ui.scene.borrow().contains(panel)) else {
            exit.set(None);
            menu.hold.set(false);
            {
                waiting.set(0);
                menu.closing.set(false);
                task.borrow_mut().take();
            }
            return;
        };
        let bounds = {
            let scene = ui.scene.borrow();
            let at = |node: Option<NodeId>| {
                node.filter(|node| scene.contains(*node))
                    .map(|node| scene.world_bounds(node))
            };
            (
                child.trigger.and_then(|trigger| at(Some(trigger))),
                at(child.panel.get()),
                at(Some(panel)),
            )
        };
        let (Some(trigger), Some(floating), Some(mine)) = bounds else {
            menu.hold.set(false);
            return;
        };
        // Over the trigger: where the pointer left it from, if it does. Over
        // the submenu (or one of its own), or off this menu: nothing held.
        if contains(trigger, p) {
            exit.set(Some(p));
            menu.hold.set(false);
            {
                waiting.set(0);
                menu.closing.set(false);
                task.borrow_mut().take();
            }
            return;
        }
        let mut panels = Vec::new();
        menu.open_panels(&mut panels);
        let over_open = {
            let scene = ui.scene.borrow();
            panels
                .iter()
                .any(|node| scene.contains(*node) && contains(scene.world_bounds(*node), p))
        };
        if over_open || !contains(mine, p) {
            menu.hold.set(false);
            {
                waiting.set(0);
                menu.closing.set(false);
                task.borrow_mut().take();
            }
            return;
        }
        // A submenu the pointer never left the trigger of (opened by the
        // keyboard) closes as an item is entered, not as it moves.
        let Some(exit) = exit.get() else {
            menu.hold.set(false);
            return;
        };
        let held = in_safe_polygon(exit, trigger, floating, p);
        menu.hold.set(held);
        // Leaving it starts the close delay once; each move inside it starts
        // the wait over (resting in it is not heading there).
        let (delay, kind) = match held {
            true => (SAFE_TIMEOUT, 1),
            false => (child.hover.get().close, 2),
        };
        if kind == 2 && waiting.get() == 2 {
            return;
        }
        waiting.set(kind);
        menu.closing.set(kind == 2 && !delay.is_zero());
        let closing: Vec<Signal<bool>> = open.iter().map(|child| child.open.clone()).collect();
        let (menu_hold, waited) = (menu.clone(), waiting.clone());
        let after = move || {
            menu_hold.hold.set(false);
            menu_hold.closing.set(false);
            waited.set(0);
            for open in &closing {
                open.set(false);
            }
        };
        *task.borrow_mut() = match &runner {
            Some(runner) if !delay.is_zero() => Some(runner.spawn(Box::pin(async move {
                crate::timer::sleep(delay).await;
                after();
            }))),
            _ => {
                after();
                None
            }
        };
    });
}
