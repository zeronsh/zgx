//! A modal opening closes the anchored popups open outside it (popovers,
//! menus, submenus; in the window or in popup windows of their own), which
//! would otherwise stay drawn over it, as AppKit closes menus and popovers
//! when a sheet opens. Popups inside the modal stay; a layer opening (an
//! overlay, not a dialog) closes nothing.
use zgx::{compose::prelude::*, reactive::Signal, widgets::Ui};

struct Open {
    ui: Ui,
    dropdown: Signal<bool>,
    actions: Signal<bool>,
    more: Signal<bool>,
    palette: Signal<bool>,
    inner: Signal<bool>,
    layer: Signal<bool>,
}

/// A dropdown (popover), a menu with its submenu open, a palette modal with
/// a popover of its own, and a layer. `popups`: shown past the window, each
/// in a window of its own, as a desktop host shows them.
fn open(popups: bool) -> Open {
    let mut ui = Ui::new(600., 400.);
    if popups {
        ui.set_popup_screens(vec![zgx::scene::Rect::new(0., 0., 1000., 800.)]);
    }
    let [dropdown, actions, more, palette, inner, layer] = [(); 6].map(|_| ui.signal(false));
    ui.mount(
        column().children([
            popover(
                "Dropdown",
                dropdown.clone(),
                button().size(80., 20.).child(text("Pick")),
            )
            .w(200.)
            .h(300.)
            .child(text("Options")),
            menu(
                "Actions",
                actions.clone(),
                button().size(80., 20.).child(text("Actions")),
            )
            .child(submenu("More", more.clone()).child(menu_item("Child"))),
            modal("Palette", palette.clone()).child(
                popover(
                    "Inner",
                    inner.clone(),
                    button().size(80., 20.).child(text("Scope")),
                )
                .child(text("Scopes")),
            ),
            modal("Toast", layer.clone()).layer().child(text("Saved")),
        ]),
    );
    ui.prepare_frame();
    for open in [&dropdown, &actions, &more] {
        open.set(true);
        ui.prepare_frame();
    }
    Open {
        ui,
        dropdown,
        actions,
        more,
        palette,
        inner,
        layer,
    }
}

fn opening_a_modal_closes_the_popups_outside_it(popups: bool) {
    let open = open(popups);
    assert!(open.dropdown.get() && open.actions.get() && open.more.get());
    if popups {
        let scene = open.ui.scene.borrow();
        assert!(!scene.popups().is_empty(), "shown in popup windows");
    }
    open.palette.set(true);
    open.ui.prepare_frame();
    assert!(!open.dropdown.get(), "the dropdown closes under the modal");
    assert!(!open.actions.get(), "the menu closes");
    assert!(!open.more.get(), "and its submenu");
    assert!(open.palette.get(), "the modal stays open");
    if popups {
        assert!(
            open.ui.scene.borrow().popups().is_empty(),
            "no popup window is left over the modal"
        );
    }
    // A popover opened from inside the open modal stays open.
    open.inner.set(true);
    open.ui.prepare_frame();
    assert!(open.inner.get(), "the modal's own popover stays");
    open.palette.set(false);
    open.ui.prepare_frame();
    open.dropdown.set(true);
    open.ui.prepare_frame();
    open.palette.set(true);
    open.ui.prepare_frame();
    assert!(
        !open.dropdown.get(),
        "closed again by the modal's next opening"
    );
}

#[test]
fn opening_a_modal_closes_the_popups_open_in_the_window() {
    opening_a_modal_closes_the_popups_outside_it(false);
}

#[test]
fn opening_a_modal_closes_the_popups_in_windows_of_their_own() {
    opening_a_modal_closes_the_popups_outside_it(true);
}

#[test]
fn a_layer_opening_closes_nothing() {
    let open = open(false);
    open.layer.set(true);
    open.ui.prepare_frame();
    assert!(open.layer.get());
    assert!(
        open.dropdown.get() && open.actions.get() && open.more.get(),
        "a layer is an overlay, not a dialog: the popups stay"
    );
}
