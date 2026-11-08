//! Menus', popovers' and tooltips' shapes come from their tokens
//! (`menu.radius`, `menu.item.inset.x`, `tooltip.padding.x`…), whose
//! defaults are the built-in look: an open menu with the keyboard on an
//! item, a popover and a tooltip draw the same bytes with the tokens left
//! alone as with them set to the built-in values, the way a platform sets
//! its own (`Themes::set_system_tokens`); and set otherwise, they change.
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers},
    scene::Rect,
    theme::{Themes, TokenInput, tok},
    widgets::Ui,
};
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const SIZE: (f32, f32) = (420., 260.);

/// The three, drawn with `tokens` set as a platform's.
fn drawn(tokens: Vec<(zgx::theme::TokenId, TokenInput)>) -> Vec<u8> {
    drawn_with(tokens, None)
}

/// `drawn`, in the app's theme `app` (named "app") if any, over the
/// system's tokens.
fn drawn_with(
    tokens: Vec<(zgx::theme::TokenId, TokenInput)>,
    app: Option<&zgx::theme::Theme>,
) -> Vec<u8> {
    let set = |ui: &Ui, tokens: Vec<(zgx::theme::TokenId, TokenInput)>| {
        let themes = Themes::of(&ui.runtime);
        themes.set_system_tokens(tokens);
        if let Some(app) = app {
            themes.register(app.clone());
            themes.set("app").unwrap();
        }
    };
    let first = tokens.clone();
    let mut gpu = GpuRenderer::new(SIZE.0 as u32, SIZE.1 as u32).unwrap();
    let mut ui = Ui::new(SIZE.0, SIZE.1);
    gpu.install_text(&mut ui.scene.borrow_mut());
    set(&ui, first);
    let open = ui.signal(false);
    let view = ui.mount(
        column()
            .size(SIZE.0, SIZE.1)
            .p(8.)
            .bg(tok::BACKGROUND)
            .child(
                row()
                    .gap(140.)
                    .child(
                        menu(
                            "Edit",
                            open.clone(),
                            button().id("anchor").child(text("Edit")),
                        )
                        .w(150.)
                        .child(menu_item("Cut").shortcut("Ctrl+X"))
                        .child(menu_item("Copy"))
                        .child(menu_item("Paste")),
                    )
                    .child(
                        popover("Details", ui.signal(true), button().child(text("Details")))
                            .w(120.)
                            .child(text("A popover")),
                    ),
            ),
    );
    ui.prepare_frame();
    ui.input.focus(&ui.scene, view.find("anchor"));
    open.set(true);
    ui.prepare_frame();
    // The keyboard on the menu's second item.
    ui.dispatch(InputEvent::KeyDown {
        key: Key::ArrowDown,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
    for _ in 0..3 {
        ui.prepare_frame();
        let _ = ui.scene.borrow_mut().flush();
    }
    gpu.render(&ui.scene.borrow(), &[Rect::new(0., 0., SIZE.0, SIZE.1)])
        .unwrap();
    let mut pixels = gpu.readback().unwrap();
    // The tooltip in a window of its own: an open menu takes the pointer.
    let mut ui = Ui::new(SIZE.0, SIZE.1);
    gpu.install_text(&mut ui.scene.borrow_mut());
    set(&ui, tokens);
    let view = ui.mount(
        column()
            .size(SIZE.0, SIZE.1)
            .p(80.)
            .bg(tok::BACKGROUND)
            .child(
                row().child(
                    button()
                        .id("tipped")
                        .child(text("Tip"))
                        .tooltip("A tooltip"),
                ),
            ),
    );
    ui.prepare_frame();
    let bounds = ui.scene.borrow().bounds(view.find("tipped").unwrap());
    ui.dispatch(InputEvent::PointerMove {
        x: bounds.x + 4.,
        y: bounds.y + 4.,
    });
    for _ in 0..3 {
        ui.prepare_frame();
        let _ = ui.scene.borrow_mut().flush();
    }
    gpu.render(&ui.scene.borrow(), &[Rect::new(0., 0., SIZE.0, SIZE.1)])
        .unwrap();
    pixels.extend(gpu.readback().unwrap());
    pixels
}

fn number(value: &str) -> TokenInput {
    TokenInput::parse(value).unwrap()
}

#[test]
fn menus_popovers_and_tooltips_default_to_the_built_in_look() {
    let _gpu = gpu_fixture();
    let alone = drawn(Vec::new());
    let built_in = drawn(vec![
        (tok::MENU_RADIUS.0, number("{radius.md}")),
        (tok::MENU_PADDING_X.0, number("16")),
        (tok::MENU_PADDING_Y.0, number("16")),
        (tok::MENU_BORDER.0, number("0")),
        (tok::MENU_ITEM_PADDING_X.0, number("12")),
        (tok::MENU_ITEM_PADDING_Y.0, number("8")),
        (tok::MENU_ITEM_INSET_X.0, number("0")),
        (tok::MENU_ITEM_INSET_Y.0, number("0")),
        (tok::MENU_ITEM_RADIUS.0, number("0")),
        (tok::POPOVER_RADIUS.0, number("{radius.md}")),
        (tok::POPOVER_BORDER.0, number("0")),
        (tok::TOOLTIP_PADDING_X.0, number("8")),
        (tok::TOOLTIP_PADDING_Y.0, number("4")),
        (tok::TOOLTIP_PADDING_BOTTOM.0, number("4")),
        (tok::TOOLTIP_BORDER.0, number("0.5")),
        (tok::TOOLTIP_RADIUS.0, number("{radius.sm}")),
        (tok::TOOLTIP_FONT_SIZE.0, number("{font.size.xs}")),
    ]);
    assert!(
        alone == built_in,
        "the tokens' defaults are the built-in look"
    );
    // And each shape follows its token.
    for (token, value) in [
        (tok::MENU_RADIUS.0, "8"),
        (tok::MENU_PADDING_Y.0, "2"),
        (tok::MENU_ITEM_INSET_X.0, "4"),
        (tok::MENU_ITEM_RADIUS.0, "4"),
        (tok::MENU_BORDER.0, "1"),
        (tok::POPOVER_RADIUS.0, "8"),
        (tok::POPOVER_BORDER.0, "1"),
        (tok::TOOLTIP_PADDING_X.0, "9"),
        (tok::TOOLTIP_PADDING_BOTTOM.0, "8"),
        (tok::TOOLTIP_FONT_SIZE.0, "12"),
    ] {
        assert!(
            drawn(vec![(token, number(value))]) != alone,
            "{} at {value} changes nothing",
            token.name()
        );
    }
}

#[test]
fn windows_look_reshapes_menus_popovers_and_tooltips() {
    let _gpu = gpu_fixture();
    let alone = drawn(Vec::new());
    let windows = drawn(zgx::theme::windows_look());
    assert!(windows != alone, "Windows' look changes nothing");
    // Drawn again, the same: its tokens are the frame's alone.
    assert!(drawn(zgx::theme::windows_look()) == windows);
}

#[test]
fn an_apps_theme_overrides_the_systems_look() {
    let _gpu = gpu_fixture();
    // An app theme setting every shape the system's look sets, to the
    // built-in values: the built-in look, whatever the system says.
    let mut app = zgx::theme::Theme::new("app", zgx::theme::ThemeKind::Dark).extending("zgx-dark");
    for (token, value) in [
        (tok::MENU_RADIUS.0, "{radius.md}"),
        (tok::MENU_PADDING_X.0, "16"),
        (tok::MENU_PADDING_Y.0, "16"),
        (tok::MENU_ITEM_PADDING_X.0, "12"),
        (tok::MENU_ITEM_PADDING_Y.0, "8"),
        (tok::MENU_ITEM_INSET_X.0, "0"),
        (tok::MENU_ITEM_INSET_Y.0, "0"),
        (tok::MENU_ITEM_RADIUS.0, "0"),
        (tok::POPOVER_RADIUS.0, "{radius.md}"),
        (tok::TOOLTIP_PADDING_X.0, "8"),
        (tok::TOOLTIP_PADDING_Y.0, "4"),
        (tok::TOOLTIP_PADDING_BOTTOM.0, "4"),
        (tok::TOOLTIP_FONT_SIZE.0, "{font.size.xs}"),
        (tok::FOCUS_RING.0, "{accent}"),
    ] {
        app = app.with(token, number(value));
    }
    let built_in = drawn_with(Vec::new(), Some(&app));
    let over_windows = drawn_with(zgx::theme::windows_look(), Some(&app));
    assert!(
        over_windows == built_in,
        "the app's theme wins over the system's tokens"
    );
    assert!(drawn(zgx::theme::windows_look()) != built_in);
}
