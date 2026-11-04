//! Menus opened at the pointer: a secondary click (or a control-click on
//! macOS, or Shift+F10 or the Menu key on what has focus) on a view with
//! [`View::context_menu`] builds its entries then and shows them in a menu
//! where the pointer is (under the focused view, from the keyboard) (or under the view, with
//! [`View::context_menu_below`]), over everything. The innermost view with a
//! context menu takes the click.
use crate::{
    compose::{Environment, View, div, menu},
    input::{EventPhase, InputEvent, Key, Modifiers, PointerButton},
    scene::NodeId,
    style::{Styled, Styles},
    widgets::Ui,
};
use std::{cell::Cell, rc::Rc};

/// Where a context menu was asked for: in window points, and from the top
/// left of the view that has it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContextMenuEvent {
    /// Across the window, in points.
    pub x: f32,
    /// Down the window, in points.
    pub y: f32,
    /// Across the view, from its left edge.
    pub local_x: f32,
    /// Down the view, from its top edge.
    pub local_y: f32,
}

/// The run of a paragraph (a [`rich_text`](crate::compose::rich_text)
/// span) a context menu was asked for on, for a menu built for the whole
/// paragraph ([`View::context_menu_with_span`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ContextMenuSpan {
    /// Its place among the paragraph's spans, from 0, as they were given.
    pub index: usize,
    /// Where it links (`TextSpan::href`), if it does.
    pub href: Option<std::sync::Arc<str>>,
    /// Its text.
    pub text: std::sync::Arc<str>,
}

/// What a menu is built for besides the point: the node the press or key
/// reached first, the node whose menu it is, and where it is.
pub(crate) struct MenuAt<'a> {
    pub target: NodeId,
    pub node: NodeId,
    pub ui: &'a Ui,
    pub environment: &'a Environment,
}
pub(crate) type ContextMenuBuilder =
    Box<dyn for<'a> FnMut(&ContextMenuEvent, &MenuAt<'a>) -> Vec<View>>;

impl View {
    /// Show a menu at the pointer on a secondary click (a control-click on
    /// macOS): `build` returns its entries (`menu_item`s, `submenu`s,
    /// separators) when asked; none shows nothing and lets the click through.
    pub fn context_menu(
        mut self,
        build: impl FnMut(&ContextMenuEvent) -> Vec<View> + 'static,
    ) -> Self {
        let mut build = build;
        let rare = self.rare_mut();
        rare.context_menu = Some(Box::new(move |event, _| build(event)));
        rare.context_menu_spans = false;
        self
    }
    /// `context_menu`, told the run of the paragraph it was asked for on
    /// (`None` off its runs, or on a view that isn't a paragraph of spans),
    /// so one menu serves every run and link in it. A link span with no
    /// menu of its own asks it too: none back shows its Open Link and Copy
    /// Link.
    pub fn context_menu_with_span(
        mut self,
        mut build: impl FnMut(&ContextMenuEvent, Option<&ContextMenuSpan>) -> Vec<View> + 'static,
    ) -> Self {
        let rare = self.rare_mut();
        // The run, found as it is asked (`compose_rich::locate_for`): its
        // paragraph keeps what finds it, and nothing else pays for it.
        rare.context_menu = Some(Box::new(move |event, at| {
            let span = crate::compose_rich::locate_for(at.ui, at.node, at.target, event.x, event.y);
            let items = build(event, span.as_ref());
            match span.and_then(|span| span.href) {
                Some(url) if items.is_empty() => crate::compose_rich::link_items(
                    url,
                    at.environment.services.get(),
                    at.environment.services.get(),
                ),
                _ => items,
            }
        }));
        rare.context_menu_spans = true;
        self
    }
    /// Open the context menu under the view, aligned to its left edge, as a
    /// tab's or a toolbar button's does, rather than at the pointer.
    pub fn context_menu_below(mut self, below: bool) -> Self {
        self.rare_mut().context_menu_below = Some(below);
        self
    }
    /// Styles for the context menu's panel, over a menu's defaults.
    pub fn context_menu_style(mut self, styles: Styles) -> Self {
        self.rare_mut().context_menu_style = Some(Box::new(styles));
        self
    }
}

/// The menu mounted for a view, removed when the next one opens or the view
/// goes (a closed menu shows nothing until then).
struct Open {
    ui: crate::widgets::WeakUi,
    node: Cell<Option<NodeId>>,
}
impl Open {
    fn close(&self) {
        if let (Some(node), Some(mut ui)) = (self.node.take(), self.ui.upgrade())
            && ui.scene.borrow().contains(node)
        {
            ui.remove_portal(node);
            ui.remove(node);
        }
    }
}
impl Drop for Open {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) fn mount(
    ui: &mut Ui,
    node: NodeId,
    mut build: ContextMenuBuilder,
    style: Option<Styles>,
    below: bool,
    environment: Environment,
) {
    let weak = ui.downgrade();
    let open = Rc::new(Open {
        ui: ui.downgrade(),
        node: Cell::new(None),
    });
    let current = open.clone();
    let binding = ui.input.listen(node, move |cx| {
        if cx.phase == EventPhase::Capture || cx.default_prevented() {
            return;
        }
        let (x, y) = match cx.event {
            InputEvent::PointerDown {
                x,
                y,
                button: PointerButton::Secondary,
            } => (x, y),
            InputEvent::PointerDown {
                x,
                y,
                button: PointerButton::Primary,
            } if cfg!(target_os = "macos") && cx.pointer_modifiers().control => (x, y),
            // From the keyboard (Shift+F10, or the Menu key): under what
            // has focus, at its start.
            InputEvent::KeyDown {
                key: Key::ContextMenu,
                ..
            }
            | InputEvent::KeyDown {
                key: Key::Function(10),
                modifiers: Modifiers { shift: true, .. },
                ..
            } => {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let scene = ui.scene.borrow();
                let b = scene.bounds(cx.target);
                let rtl = scene.direction(cx.target) == crate::scene::Direction::Rtl;
                (if rtl { b.x + b.width } else { b.x }, b.y + b.height)
            }
            _ => return,
        };
        let Some(mut ui) = weak.upgrade() else {
            return;
        };
        // Hung in the window, where the box shows.
        let origin = ui.scene.borrow().transformed_bounds(node);
        let (local_x, local_y) = ui.scene.borrow().local_point(node, x, y);
        let event = ContextMenuEvent {
            x,
            y,
            local_x,
            local_y,
        };
        let at = MenuAt {
            target: cx.target,
            node,
            ui: &ui,
            environment: &environment,
        };
        let items = build(&event, &at);
        if items.is_empty() {
            return;
        }
        cx.prevent_default();
        cx.stop_propagation();
        // A context click focuses what it lands on, as any button's press
        // does in a browser (a field's caret goes there): the menu gives
        // focus back to it as it closes.
        let pressed = matches!(cx.event, InputEvent::PointerDown { .. });
        let within = ui.input.focused().is_some_and(|focused| {
            let scene = ui.scene.borrow();
            focused == node
                || (scene.contains(focused) && scene.ancestors(focused).any(|a| a == node))
        });
        if pressed && !within && ui.input.options(node).is_some_and(|o| o.focusable) {
            ui.input.focus(&ui.scene, Some(node));
        }
        current.close();
        let shown = ui.runtime.signal(true);
        // An editor it takes the focus from keeps its selection drawn.
        ui.set_context_menu(shown.clone());
        let anchor = if below {
            div().w(origin.width).h(origin.height)
        } else {
            div().w(1.).h(1.)
        };
        let mut panel = menu("Context menu", shown, anchor).children(items);
        if let Some(style) = &style {
            panel = panel.merge_styles(style);
        }
        // What it hangs from: the pointer's point, or the view's box (a
        // menu opens under its anchor).
        let view = if below {
            div()
                .absolute()
                .left(origin.x)
                .top(origin.y)
                .w(origin.width)
                .h(origin.height)
                .child(panel)
        } else {
            div().absolute().left(x).top(y).w(0.).h(0.).child(panel)
        };
        let root = ui.root();
        let mounted = crate::compose::mount(&mut ui, root, view, environment.clone());
        // Mounted at the root, it is its element's: devtools find it there.
        ui.add_portal(node, mounted);
        current.node.set(Some(mounted));
    });
    ui.retain(node, binding);
    ui.retain(node, open);
    ui.listen_popup_soon(node);
}
