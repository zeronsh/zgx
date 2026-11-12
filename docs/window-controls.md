# Native window controls

`WindowContext::window` is a cloneable, thread-safe `WindowHandle`. Its methods queue requests onto the application thread:

```rust,no_run
# fn configure(cx: &mut zgx_desktop::WindowContext) {
let window = cx.window.clone();
window.set_title("Notes — saved");
window.set_inner_size(800., 600.); // logical client dimensions
window.set_maximized(true);
window.set_minimized(false);
window.set_visible(true);
window.request_focus();
# }
```

Title, size, visibility, and requested minimize/maximize state survive suspension and native recreation. Commands also work while a newly opened window is awaiting construction. A focus request made without a native window is delivered once after creation. Closed handles ignore subsequent requests. Updating the title invalidates the accessible window title as well.

Size requests reject zero, negative, nonfinite, or dimensions exceeding `u32::MAX`. The operating system can constrain any request. Observe `WindowContext::viewport` for actual logical client dimensions; receiving a command is not confirmation that the window manager accepted it. Native resize and scale events update the size used for recreation.

Winit currently does not implement dynamic visibility or focus requests on Wayland, and Wayland cannot programmatically unminimize. Focus requests on other platforms can also be rejected by focus-stealing prevention. Minimize/maximize recreation state records application requests; it does not promise to mirror every window-manager action.

Run the interactive example with `cargo run -p zgx-desktop --example window_controls`. On X11 with a window manager, append `-- --smoke-test` to request title, resize, minimize/restore, maximize/restore, hide/show, and close. The example checks actual viewport resize acknowledgments. External X11 property and map-state observations additionally verified minimization, maximization, restoration, hiding and showing; see `platform-validation/window-controls-x11.json`. This does not constitute native macOS or Wayland control validation.

Confirmed hidden, minimized, zero-sized or occluded surfaces skip frame preparation and GPU submission. Pending damage remains until restoration; application tasks and model updates continue. Unknown visibility is treated as drawable, so unsupported compositor queries do not blank the window. A redraw request cannot override explicit native occlusion. `hidden_updates` and `scripts/hidden_updates_smoke.py` check actual X11 unmapped states and latest restored pixels after 40 timed model updates. This does not imply that all reactive application computations stop while hidden.

## Title bars, glass and drag regions

`WindowOptions::titlebar` styles the title bar: `Titlebar::Standard`, `Titlebar::Inset` (transparent and untitled, content running under it, the traffic lights inset in a 40-point strip, Electron's `hiddenInset`, or where `WindowOptions::traffic_lights` puts them: below) or `Titlebar::Hidden` (no title bar and no window buttons, the window keeping its corners and shadow). On Windows, `Inset` puts Windows 11's caption buttons (minimize, maximize, close; 46 by 32 points each) over the content's top right, which Windows treats as its own: tooltips, and snap layouts on the maximize button. Both are frameless windows that keep the system shadow and, on Windows 11, rounded corners, and resize from bands 5 points in from their edges. On Linux both are undecorated windows, as GTK's header bars are, resizing from the same bands through the window manager, and `Inset` draws libadwaita's window buttons in the order and on the side the desktop asks ([Linux](linux.md#title-bars)). `WindowOptions::glass` puts a frosted desktop behind the window, cut to its rounded corners: `Glass::new(GlassMaterial::Hud)` (the default), any `GlassMaterial`, and `.plain()` for only the blur, untinted and unsaturated, under an app's own tint. Glass makes the window transparent. On Windows 11 (22H2 and later) the glass is the system backdrop nearest the material: Acrylic (a blur) for `Hud`, `Menu`, `Popover`, `ToolTip`, `Sheet`, `FullScreenUi` and `Selection`; Mica for `WindowBackground`, `UnderWindowBackground`, `UnderPageBackground`, `ContentBackground` and `Sidebar`; Mica Alt for `Titlebar` and `HeaderView`. `.plain()` has no system backdrop, whatever the material: it is the acrylic blur of what is behind the window, untinted, from Windows 10 1809, through Windows' undocumented `SetWindowCompositionAttribute`, and with Transparency effects off the material's backdrop. Older Windows shows a plain transparent window. Both apply as the window is made, before it shows, keeping the content size asked for; `WindowContext::window_info.corner_radius` then holds the window's corner radius, so content inset from its edge stays concentric with it.

Content running under an `Inset` or `Hidden` title bar should stay clear of the window's own buttons. `WindowContext::window_info.titlebar_area` holds where it may draw, in points, as the web's Window Controls Overlay gives it (`getTitlebarAreaRect`, `env(titlebar-area-*)`): the strip the title bar leaves, less the window's buttons. `window_info.controls` holds the buttons themselves. Both follow resizes, scale and fullscreen; both are `None` for a standard title bar and in fullscreen (where a window shows no buttons). On Windows the strip is 32 points tall, with the caption buttons (138 points) at its right for `Inset`, and the whole strip for `Hidden`. On macOS they are read from AppKit's own buttons: the traffic lights where they are, and the strip beside them, 40 points tall or, where the buttons sit lower, as tall as centres them. On Linux, the buttons zgx draws and the strip beside them. Luau reads them as `ui.window.titlebar_area` and `ui.window.controls` once the host passes them in (`ScriptHost::window_chrome`; `zgx run` does).

`View::window_drag()` (Luau `window_drag = true`) makes a view a drag region, as `-webkit-app-region: drag`: a primary press on it, or on anything inside it that takes no input, moves the window, and a double click does what a double click on the title bar does (macOS follows the system setting: zoom, minimize or nothing; elsewhere the window toggles maximized). Focusable or pointer-capturing descendants, such as buttons and inputs, keep their presses, and pressing to move the window leaves focus where it was. A secondary click on a drag region opens the window's menu (restore, move, size, minimize, maximize, close) as a right click on a title bar does, on Windows and Wayland; macOS has no such menu.

```rust,no_run
use zgx::compose::prelude::*;
use zgx_desktop::{Application, Glass, Titlebar, WindowOptions};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "Notes".into(),
            titlebar: Titlebar::Inset,
            glass: Some(Glass::default()),
            ..Default::default()
        })
        .run(|cx| {
            cx.render(
                column()
                    .w_full()
                    .h_full()
                    .bg(rgba(0x1214168c))
                    // The strip under the traffic lights moves the window.
                    .child(div().w_full().h(40.).window_drag()),
            );
        })
}
```

On macOS an app whose title bar is taller or lower than AppKit's 40-point strip lines the traffic lights up with its own controls through `WindowOptions::traffic_lights`, as Electron's `trafficLightPosition`: the close button's top left, in the content's points from the window's top left, the minimize and zoom buttons following at AppKit's spacing. They stay there as the window resizes, changes focus or appearance, and comes back from full screen (which hides them), and they move with the window's zoom (`Ui::set_zoom`) as the content does, centred on the row a button there centres on at 100%. The buttons are 14 points tall on current macOS: `y = (row - 14) / 2` centres them in a row of that height. `window_info.controls` and `titlebar_area` follow them. It does nothing for other title bars and on other platforms.

```rust,no_run
use zgx_desktop::{Titlebar, WindowOptions};
let options = WindowOptions {
    titlebar: Titlebar::Inset,
    // A 52-point toolbar: the lights 19 points down, 18 in.
    traffic_lights: Some((18., 19.)),
    ..Default::default()
};
```

`cargo run -p zgx-desktop --example traffic_lights_probe -- --at 18,22 --hold` shows it, and `scripts/macos_traffic_lights_probe.py` checks where AppKit has the close button through a resize, appearance and focus changes, a zoom and full screen.

While it shows, `WindowHandle::set_glass(Some(glass))` gives the window other glass in place of its own, and `set_glass(None)` takes it away (macOS: the effect view behind the content; Windows 11: the system backdrop). The content shows glass only where the window is transparent, so a window that will gain glass later is made `transparent: true` (or with glass) to begin with. A window stays as transparent as it was made: with `None`, what is behind it shows through, unblurred, wherever the content paints nothing or paints translucent, on macOS and Windows alike, so an app that drops its glass paints its background opaque (its theme's `background`). `WindowHandle::set_theme(Some(Appearance::Dark))` shows the window dark whatever the system shows, its title bar and controls and zgx's themes that follow the system (`Themes::follow_system`) alike; `set_theme(None)` follows the system again. A window made again keeps both. `cargo run -p zgx-desktop --example glass_counter -- --appearance-fixture` checks each against AppKit's views and appearance on macOS and the DWM backdrop and dark mode on Windows; `--decorated` gives the window the platform's own title bar, to see it follow the theme.
