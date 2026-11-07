# iOS

zgx runs on iOS through the desktop host (`zgx-desktop`), under winit's iOS backend: `Application`, Rust views and Luau apps work as they do elsewhere. Luau is interpreted, as it is on every platform. It is a spike, run on simulators and on an iPhone Air (`docs/ios-device.md`), not yet on main (see "Landing on main").

```sh
python3 scripts/ios/sim.py run --example gallery --shot out.png
python3 scripts/ios/sim.py run -p zgx-run --example ios_app --name settings_app --resource crates/zgx-script/examples/ios_settings --shot out.png
```

The script boots a simulator of its own (`zgx-ios-lab`, or `ZGX_IOS_DEVICE`) without opening Simulator.app. Simulator results say nothing about performance, memory or feel: it draws on the Mac's GPU.

## What is the system's, and what is zgx's

| Piece | Whose | Why |
| --- | --- | --- |
| Text, lists, rows, cards, layout | zgx | Colours (`$text.primary`, `$ios.*`), Dynamic Type sizes and tracking are read from UIKit; laid out to UIKit's numbers |
| Glass (`ui.glass`, `ui.glass_group`) | the system's | The compositor draws it from what lies under it, zgx's Metal layer included: a native view between the base and overlay planes |
| Scrolling | UIKit's physics, zgx's drawing | A `UIScrollView` nobody sees, its pan on the window's view, drives zgx's scroll views (`ios_scroll.rs`) |
| Keyboard, marked text, suggestions | the system's | A `UITextInput` responder over the document Windows' input methods use (`ios_text.rs`, `text_store.rs`) |
| SF Symbols (`ui.symbol`) | the system's | Drawn by `UIImage` |
| Switch | either | `ui.native_switch` is `UISwitch`: exact, and in a list about 6 points of a core more while scrolling, measured on a simulator. A drawn one is free |
| Tab bar (`ui.native_tabs`) | the system's | Its selection is a lens that morphs between tabs, which nothing drawn matches |

A window with no native view does none of the plane work, as on the desktop.

## Accessibility

There is one tree of elements, zgx's, through AccessKit's iOS adapter (`vendor/accesskit_ios`, its changes in `ZGX_PATCH.md`). System views are hidden from assistive technology and exposed as their zgx node. `ZGX_IOS_AX_DUMP=1` prints what is offered. The adapter builds nothing until a tree is read, shuts down when VoiceOver stops, and lets go 30 seconds after the last read when nothing says assistive technology runs (`vendor/accesskit_ios/tests/shutdown.rs`, on a simulator).

Bold Text makes every weight a step heavier, in text and in symbols, and text shapes again when it changes. `ui.prefers.without_color` and `ui.prefers.inverted_colors` say Differentiate Without Color and Smart or Classic Invert: inversion is the compositor's and a simulator doesn't show it, so what it does to a window of Metal needs a phone.

A text field's `accessibilityActivate` takes focus and brings the keyboard; rows, tabs and switches click. A search field is marked as one, and Voice Control answers to a row's name alone ("Wi-Fi", from "Wi-Fi, Home"), by `accessibilityUserInputLabels`.

## Not there yet

- Checked by a person on a phone: VoiceOver, Switch Control, Voice Control, Full Keyboard Access, feel. Measured there so far: glass, planes, frame rate, launch, memory (`scripts/ios/device.py measure`).
- System glass isn't cut by rounded or scrolling parents: it ignores masks and clips. A pane scrolled away is hidden whole.
- Morphs between panes that aren't in one group (a pane joining a group grows out of it and melts back); terminals; web views' new windows, downloads, finding and printing.
- Notifications.
- `Documents/zgx-gesture.log` records each gesture, scroll, refresh and keyboard reveal; `ZGX_IOS_SHOT=seconds` writes the window to `Documents/shot.png`.

## The edge fade

`ui.scroll_edge { edge, style, size, shown, drawn }` is iOS 26's scroll edge effect on a window's edge: Apple's own, through public API. The first one puts the app in a `UIScrollView` held still, because the effect works on a scroll view's own contents, and adds a view with `UIScrollEdgeElementContainerInteraction` over the box; an app without one pays nothing. What the script paints after the node shows above the effect. Against a real navigation bar over the same content, pixels match within 1/255 where both pick the same veil; the effect picks a darkening or a lightening one from what is under it, and at some offsets the two pick differently. `drawn = true` is a blur and gradient for an edge inside the window, and what other platforms get: not Apple's, and not matched to it.

Which veil the effect shows over the same content depends on how the scroll got there: Apple's own bar, ending at one offset from three different starts, shows three different veils, and repeats exactly from a fresh launch. Don't compare zgx with a native bar at one offset and call a different veil a bug; compare one that chose the same (soft in light matches to 1/255).

## A frame is one change

A frame's base plane, overlay planes, native views and any animation it ends change in one Core Animation transaction, or none of them do. A plane's layer never scales what it holds, and a plane moves or resizes only with a frame drawn for that size: a late frame leaves its contents where they were, never stretched over the page (`Documents/zgx-gesture.log` has each change of a plane's size, and any frame that didn't fit). The base presents first: if it finds no drawable, nothing else changes and the frame is tried again. A plane shows or moves only with a frame presented for it, and what it holds never scales up to a new size. A frame that changes a plane or lands an animation waits for its GPU work to finish before it commits, since a phone otherwise shows the rest without it, for one refresh, which a simulator's GPU is fast enough to hide. `ZGX_IOS_PLANE_DEBUG=1` tints each plane and logs each change; `Documents/zgx-gesture.log` says when a present found no drawable.

## The keyboard

A field low on a page and a `ui.keyboard_bar` move with the keyboard in the render server, nothing drawn: `ios_scroll` reads the keyboard's spring from the layout guide's animation, in its transaction, and builds a keyframe animation that holds the gap once the keyboard reaches them. The keyboard is near its fastest (about 2800 pt/s on a phone-sized simulator) when it reaches a field, so the field starts as the keyboard does and its speed blends up to the keyboard's, riding it exactly from about 0.1 s (`keyboard_rise`). The page's offset or the bar's lift changes in one transaction with the animation's removal. The page moves only as the view under the edge effect, so it needs an edge effect: without one, and for a finger's drag or a keyboard whose animation isn't one spring, it follows by frames, a frame or two behind.

While the page rides the keyboard its scroll offset goes on frame by frame while what is drawn holds still (`ScrollDriver::hold`), so whatever reads the offset, an edge effect, a small title or an app's own, follows the page as it follows a real scroll, with no special call. A `ui.list` makes room for the keyboard the same way, and what reads its offset follows too. A field moves again if a finger took it under the keyboard and someone types, or if focus reached it after the keyboard moved. `Documents/zgx-gesture.log` has every change of the page's offset while the keyboard is up.

`ui.ios.composer { value = text, on_send = send, on_plus = add }` is one built on it: a thin pill at rest that opens into a taller box for a few lines, wider, rounder and taller together on the keyboard's own spring (mass 1, stiffness 555, damping 47.1, read from UIKit, the same opening and closing), so it finishes as the keyboard does. Lines grow on the same spring to `max_lines` then scroll inside, and the send button comes in with text. With Reduce Motion it changes at once under a short fade. Its height rides the keyboard exactly.

`ui.keyboard_bar { absolute = true, left = 16, right = 16, bottom = 100, ... }` is a chat's composer. It takes the top two drawing levels for itself, so what it shows goes through its children; a background of its own stays behind with the page. One bar in a window. A touch on a control leaves the focused field and the keyboard alone, as UIKit's controls do.

## In Luau

`ui.ios` (a Settings-style app's sections, rows, pages with large titles, stacks with edge swipes, sheets, search, pull to refresh), `ui.alert`, `ui.action_sheet`, `ui.haptic`, `ui.share`, `ui.system_menu`, `ui.on_escape`, `ui.native_pan`, `ui.pull_refresh`, `ui.scroll_edge`, `ui.webview`. `crates/zgx-script/examples/ios_showcase` holds them in one app.

A stack pushes and pops on a navigation controller's own springs, read from UIKit as it runs: a button's push or pop moves its progress on a spring of damping ratio 1 and response 0.3 s, starting at 8.75 widths a second, 53 ms after the call for a push and 40 ms for a pop, as on an iPhone at 120 Hz, where UIKit's own push matches that spring within half a point. A swipe's page follows the finger through a spring of 0.85 and 0.08 s, and a release goes home on 0.85 and 0.3 s, as `_UINavigationInteractiveTransitionBase` decides from its code: each sample's speed and acceleration averaged over the last three, it goes back when where the page would be after 0.35 s is past 187.5 points or half the width, whichever is less, and leaves at the finger's speed, 4.25 times it while every average since the finger landed is 1.5 widths a second or more. The back button's glass shows from a push and goes when the pop is done; its chevron fades on a critically damped spring of 0.415 s, as UIKit's does. With Reduce Motion a push or pop cross-fades over 0.5 s, ease-in-out, as `_UINavigationCrossfadeAnimator` does. A page's large title collapses into the small one by time once it has scrolled under the 54 point bar: a 0.2 s ease-in-out fade out, and the small one fading in rising 15 points on a 0.447 s spring. `ios.stack { log_motion = true }` puts each position in the gesture log.

A sheet (`ui.ios.sheet`) is UIKit's: medium is 0.56 of the window less its safe areas (0.63 in a window 568 points tall or less), large reaches the top safe area, and present, detent changes and dismiss run on a critically damped spring of 0.344 s. A drag projects at deceleration rate 0.99 to the nearest detent (dismissed among them), a flick over 1000 points a second steps one detent on, and a tap on the grabber goes to the detent before. Its grabber says "Sheet Grabber", "Half screen" or "Expanded", and what a tap will do, in UIKit's words.

## Landing on main

What landing the iOS target on main needs, in an order where each step stands alone:

1. **Engine fixes found here, each a PR of its own** (on every platform, tested headless): stack segments kept per thread (#1555, kept segment compiled for iOS only), a component's caller's accessibility reaching its root (#1576), `value_text` on any element (#1577), a hint from state (#1578), glass per-corner radii (#1584), `ui.spring_to` with its rest test scaled to the range (#1586), and `aria_modal` for a sheet or dialog an app builds itself (#1599). Still to propose: `keyboard_ride` on scroll views (with step 2's `ScrollDriver`).
2. **Core pieces the iOS host stands on, cfg-free and inert elsewhere:** native views between planes beyond one overlay (`PlaneSplit` levels, `above` planes in `native_host`), `native_control` (switch, tabs, edge effect, keyboard bar), `ScrollDriver` (`elastic.rs`: a platform scroller drives a scroll view), `symbol.rs`, and the plane fit checks in `zgx-gpu/src/planes.rs` (a drawable sized to its plane, the sublayer it lives on).
3. **The iOS host, all `cfg(target_os = "ios")`:** `zgx-desktop/src/ios_*.rs` and `native_host/ios.rs`, `ios_web.rs`, the `application.rs` lifecycle changes, and the vendored crates: `vendor/accesskit_ios` (its patches in `ZGX_PATCH.md`) and the winit fork's iOS view changes (`vendor/winit/src/platform_impl/ios`).
4. **Scripts:** `native.rs` (`ui.glass_group`, `ui.symbol`, `ui.native_switch`, `ui.native_tabs`, `ui.haptic`, `ui.alert`, `ui.system_menu`, `ui.native_pan`, `ui.share`, `ui.scroll_edge`, `ui.keyboard_bar`, `ui.pull_refresh`, `ui.action_sheet`), the kit (`ios_kit.luau`, `ui.ios`) and its types; `tests/ios_kit.rs` runs on every platform, the kit being Luau.
5. **CI:** a macOS step building and linting for the simulator (`rustup target add aarch64-apple-ios-sim`, then `cargo clippy -p zgx-run --example ios_app --target aarch64-apple-ios-sim -- -D warnings`), and the desktop jobs, which already build the iOS-free paths: on draft #1571 this branch passes Linux, macOS and Windows (build, clippy with warnings denied, tests), after one Windows-only `unused_mut` the iOS overlay's renderer needed. A simulator smoke (boot, launch `ios_app`, `ZGX_IOS_SHOT`) would need a runner that can launch simulator apps; device builds need signing and stay local (`scripts/ios/device.py`).
6. **Docs:** this page and `docs/ios-device.md`; `docs/reference/native.md` for the new `ui.*`, and a reference page for `ui.ios`; changelog fragments per PR.

Open before it lands: whether a field riding the keyboard holds its gap (the kit's) or closes it as UIKit's table does (Wing's call); zgx's normal line height against the font's own (draft, Wing's call); and what needs a phone and a person (VoiceOver, Switch Control, Voice Control, Dynamic Type to AX5 in practice).
