# iOS

zgx runs on iOS through the desktop host (`zgx-desktop`), under winit's iOS backend: `Application`, Rust views and Luau apps work as they do elsewhere. Luau is interpreted, as it is on every platform. It is a spike: nothing has run on a real iPhone yet.

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

## Not there yet

- Checked by a person on a phone: VoiceOver, Switch Control, Voice Control, Full Keyboard Access, feel. Measured there so far: glass, planes, frame rate, launch, memory (`scripts/ios/device.py measure`).
- System glass isn't cut by rounded or scrolling parents: it ignores masks and clips. A pane scrolled away is hidden whole.
- Morph transitions between panes beyond a group's merging; terminals; web views' new windows, downloads, finding and printing.
- Notifications.
- `Documents/zgx-gesture.log` records each gesture, scroll, refresh and keyboard reveal; `ZGX_IOS_SHOT=seconds` writes the window to `Documents/shot.png`.

## The edge fade

`ui.scroll_edge { edge, style, size, shown, drawn }` is iOS 26's scroll edge effect on a window's edge: Apple's own, through public API. The first one puts the app in a `UIScrollView` held still, because the effect works on a scroll view's own contents, and adds a view with `UIScrollEdgeElementContainerInteraction` over the box; an app without one pays nothing. What the script paints after the node shows above the effect. Against a real navigation bar over the same content, pixels match within 1/255 where both pick the same veil; the effect picks a darkening or a lightening one from what is under it, and at some offsets the two pick differently. `drawn = true` is a blur and gradient for an edge inside the window, and what other platforms get: not Apple's, and not matched to it.

Which veil the effect shows over the same content depends on how the scroll got there: Apple's own bar, ending at one offset from three different starts, shows three different veils, and repeats exactly from a fresh launch. Don't compare zgx with a native bar at one offset and call a different veil a bug; compare one that chose the same (soft in light matches to 1/255).

## A frame is one change

A frame's base plane, overlay planes, native views and any animation it ends change in one Core Animation transaction, or none of them do. The base presents first: if it finds no drawable, nothing else changes and the frame is tried again. A plane shows or moves only with a frame presented for it, and what it holds never scales up to a new size. A frame that changes a plane or lands an animation waits for its GPU work to finish before it commits, since a phone otherwise shows the rest without it, for one refresh, which a simulator's GPU is fast enough to hide. `ZGX_IOS_PLANE_DEBUG=1` tints each plane and logs each change; `Documents/zgx-gesture.log` says when a present found no drawable.

## The keyboard

A field low on a page and a `ui.keyboard_bar` move with the keyboard in the render server, nothing drawn: `ios_scroll` reads the keyboard's spring from the layout guide's animation, in its transaction, and builds a keyframe animation that holds the gap once the keyboard reaches them. The keyboard is near its fastest (about 2800 pt/s on a phone-sized simulator) when it reaches a field, so the field starts as the keyboard does and its speed blends up to the keyboard's, riding it exactly from about 0.1 s (`keyboard_rise`). The page's offset or the bar's lift changes in one transaction with the animation's removal. The page moves only as the view under the edge effect, so it needs an edge effect: without one, and for a finger's drag or a keyboard whose animation isn't one spring, it follows by frames, a frame or two behind.

While the page rides the keyboard its offset doesn't change, so a page's bar is told where it shows, frame by frame (`ui.native_shown_scroll`): the edge effect and the small title change with the motion, as a real scroll's do. A field moves again if a finger took it under the keyboard and someone types, or if focus reached it after the keyboard moved. `Documents/zgx-gesture.log` has every change of the page's offset while the keyboard is up.

`ui.keyboard_bar { absolute = true, left = 16, right = 16, bottom = 100, ... }` is a chat's composer. It takes the top two drawing levels for itself, so what it shows goes through its children; a background of its own stays behind with the page. One bar in a window. A touch on a control leaves the focused field and the keyboard alone, as UIKit's controls do.

## In Luau

`ui.ios` (a Settings-style app's sections, rows, pages with large titles, stacks with edge swipes, sheets, search, pull to refresh), `ui.alert`, `ui.action_sheet`, `ui.haptic`, `ui.share`, `ui.system_menu`, `ui.on_escape`, `ui.native_pan`, `ui.pull_refresh`, `ui.scroll_edge`, `ui.webview`. `crates/zgx-script/examples/ios_showcase` holds them in one app.
