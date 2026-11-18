//! A menu shortcut acts before the key typed right after it: Ctrl/Cmd+G
//! moves the caret to the other field, and a letter sent with no gap after
//! it lands there, not in the field the caret left
//! (`scripts/shortcut_order_smoke.py` types twenty such pairs). Ctrl/Cmd+R
//! prints both fields.
use zgx::{compose::prelude::*, input::Key};
use zgx_desktop::{AppMenu, AppMenuEntry, Application, MenuAccelerator, WindowOptions};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .menus(vec![AppMenu::new(
            "Go",
            [
                AppMenuEntry::item("Other field", "other")
                    .accelerator(MenuAccelerator::primary(Key::Character("g".into()))),
                AppMenuEntry::item("Report", "report")
                    .accelerator(MenuAccelerator::primary(Key::Character("r".into()))),
            ],
        )])?
        .window(WindowOptions {
            title: "zgx shortcut order".into(),
            width: 460.,
            height: 220.,
            ..Default::default()
        })
        .run(|cx| {
            let first = cx.ui.signal(String::new());
            let second = cx.ui.signal(String::new());
            let view = cx.render(
                column()
                    .p(12.)
                    .gap(12.)
                    .children((!zgx_desktop::has_native_app_menu()).then(|| cx.app_menu_bar()))
                    .child(
                        text_input("First", first.clone())
                            .id("first")
                            .w(400.)
                            .h(40.),
                    )
                    .child(
                        text_input("Second", second.clone())
                            .id("second")
                            .w(400.)
                            .h(40.),
                    )
                    .child("Ctrl/Cmd+G moves to the other field; Ctrl/Cmd+R reports both."),
            );
            let fields = [view.find("first"), view.find("second")];
            let input = cx.ui.input.clone();
            let scene = cx.ui.scene.clone();
            input.focus(&scene, fields[0]);
            let mut at = 0;
            cx.on_menu_action(move |action| match action.0.as_str() {
                "other" => {
                    at = 1 - at;
                    input.focus(&scene, fields[at]);
                    println!("OTHER {at}");
                }
                "report" => println!("FIELDS first={:?} second={:?}", first.get(), second.get()),
                _ => {}
            });
            println!("READY");
        })
}
