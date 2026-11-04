//! macOS's menu bar, on the main thread (`fn main`), where AppKit's menus
//! are made:
//! - the Services submenu it shows is the one AppKit fills with the
//!   system's services, when the menu bar is installed and when it is
//!   replaced (the app's menus changed);
//! - a shortcut pressed before the menu bar is made (it comes after the
//!   first frame) still runs its item;
//! - each of the app's menus shows once.
fn main() {
    #[cfg(target_os = "macos")]
    {
        use std::sync::{Arc, Mutex};
        use zgx::input::Key;
        use zgx_desktop::app_menu::{
            AppMenu, AppMenuEntry, MenuAccelerator, debug_key_before_menu_bar,
            debug_menu_bar_services_filled, debug_menu_bar_titles, debug_menu_change,
        };
        for install in ["installed", "replaced"] {
            assert!(
                debug_menu_bar_services_filled(&[]).unwrap(),
                "AppKit fills another Services menu than the menu bar's ({install})"
            );
        }
        println!("menu_services: AppKit fills the menu bar's Services submenu");

        let chosen = Arc::new(Mutex::new(Vec::new()));
        muda::MenuEvent::set_event_handler(Some({
            let chosen = chosen.clone();
            move |event: muda::MenuEvent| chosen.lock().unwrap().push(event.id.0)
        }));
        let menus = [AppMenu::new(
            "File",
            [AppMenuEntry::item("Export", "export")
                .accelerator(MenuAccelerator::primary(Key::Character("e".into())))],
        )];
        assert!(
            debug_key_before_menu_bar(&menus, "e").unwrap(),
            "⌘E before the menu bar was made was not taken by it"
        );
        assert_eq!(*chosen.lock().unwrap(), ["export"]);
        assert!(
            !debug_key_before_menu_bar(&menus, "j").unwrap(),
            "a key no item has was taken"
        );
        println!("menu_services: a shortcut before the menu bar is made runs its item");

        // Each of the app's menus shows once, with the items zgx adds to an
        // Edit menu (Find) and a View menu (zoom).
        let menus = [
            AppMenu::new(
                "Edit",
                [AppMenuEntry::Standard(
                    zgx_desktop::app_menu::StandardItem::Copy,
                )],
            ),
            AppMenu::new("View", [AppMenuEntry::item("Sidebar", "sidebar")]),
        ];
        assert_eq!(
            debug_menu_bar_titles(&menus).unwrap(),
            ["Application", "Edit", "View", "Window"]
        );
        println!("menu_services: each of the app's menus shows once");

        // The app's menus changing only their items' titles and states are
        // shown in place: AppKit keeps the bar it shows (a new main menu is
        // drawn again, a flash across the bar), its items as asked. Any other
        // change makes a new one.
        let view = |back: &str, enabled: bool, sidebar: bool| {
            [AppMenu::new(
                "View",
                [
                    AppMenuEntry::item(back, "back").enabled(enabled),
                    AppMenuEntry::item("Sidebar", "sidebar").checked(sidebar),
                ],
            )]
        };
        let zgx_desktop::app_menu::MenuChange {
            in_place,
            kept,
            items,
            ..
        } = debug_menu_change(&view("Back", false, false), &view("Go Back", true, true)).unwrap();
        assert!(in_place && kept, "in place {in_place}, bar kept {kept}");
        assert!(
            items.contains(&("Go Back".into(), true, false))
                && items.contains(&("Sidebar".into(), true, true)),
            "{items:?}"
        );
        let mut more = view("Back", true, true);
        more[0].entries.push(AppMenuEntry::item("Reload", "reload"));
        let zgx_desktop::app_menu::MenuChange {
            in_place,
            kept,
            items,
            ..
        } = debug_menu_change(&view("Back", true, true), &more).unwrap();
        assert!(!in_place && !kept, "in place {in_place}, bar kept {kept}");
        assert!(
            items.iter().any(|(title, ..)| title == "Reload"),
            "{items:?}"
        );
        println!("menu_services: menus changing only their items' titles and states keep the bar");

        // A bar AppKit can't make (here two items on one key, as
        // `set_menus` never passes on) is said, and the last good bar
        // stays: no exit, no empty bar.
        let key = |key: &str| {
            MenuAccelerator::new(
                Key::Character(key.into()),
                zgx::input::Modifiers {
                    meta: true,
                    ..Default::default()
                },
            )
        };
        let broken = [AppMenu::new(
            "View",
            [
                AppMenuEntry::item("Back", "back").accelerator(key("j")),
                AppMenuEntry::item("Forward", "forward").accelerator(key("j")),
            ],
        )];
        let zgx_desktop::app_menu::MenuChange {
            kept,
            failed,
            items,
            ..
        } = debug_menu_change(&view("Back", true, false), &broken).unwrap();
        assert!(
            failed.is_some() && kept,
            "failed {failed:?}, bar kept {kept}"
        );
        assert!(
            items.iter().any(|(title, ..)| title == "Sidebar")
                && !items.iter().any(|(title, ..)| title == "Forward"),
            "{items:?}"
        );
        println!("menu_services: a bar AppKit can't make keeps the last good one");
    }
}
