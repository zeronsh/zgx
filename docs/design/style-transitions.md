# Style transitions

Status: draft design. Step one (opacity and colours from Rust) is drafted
in #1701; the rest is not built.

A style property that changes eases to its new value over a duration on a
curve, as CSS transitions do: a hover's background fading in, a focus ring
growing into place, an overlay scrollbar fading in. Today a change of style
lands at once. What eases is done by hand, three ways:

- `ui.tween` / `ui.spring` bindings, in Luau only, each a property's value
  bound to a 0..1 animation of a state (`zgx-script/src/anim.rs`);
- the anime engine (`zgx-script/src/anime/`), in Luau only, a clip per
  `anime.animate`;
- hand-stepped fades in zgx core, each its own timer: the tooltip's
  (`compose_tooltip.rs`, 8 ms sleeps), the checkbox's draw-on, the
  progress sweep, the overlay scrollbar's (`compose_scrollbar.rs`, on
  `next_interaction_deadline`).

None of them eases between a view's variants (hover, active, focus,
disabled): a Luau `hover = { … }` table must be plain values ("animate the
base style instead"), and Rust variants switch at once. And none honours
reduced motion except `shader_drive`, animated images and zgx-fx.

What needs it now: GNOME's focus ring (libadwaita's 200 ms
`cubic-bezier(0.25, 0.46, 0.45, 0.94)` on the outline's colour, width and
offset), GTK's overlay scrollbar fading in over a second, WinUI's flyouts
and scrollbar, and many Luau interfaces that today re-fire `anime.animate`
on every hover (the engine warns after 20 restarts).

## What it is

A view says which of its properties transition, and how. From then on,
whenever the value the view would draw for such a property changes (its
base style, a `reactive_style`, a variant coming into or out of effect, a
theme token resolving anew), the scene shows the old value easing to the
new one rather than the new one at once. Nothing else about the view
changes: layout, hit testing and semantics see the target at once, as a
CSS transition's computed value does.

What is transitioned is the drawn value, never the declared one: a
`reactive_style` reading state still decides the target, and reading the
view's style back gives the target.

## The API

### Rust

On `Styled` (so on `View`, `Styles` and variants alike):

```rust
button()
    .bg(tok::BUTTON_BG)
    .hover(|s| s.bg(tok::BUTTON_HOVER))
    .transition(Transition::of([Prop::Background, Prop::BorderColor])
        .duration(tok::MOTION_DURATION_FAST)
        .ease(tok::MOTION_EASE_STANDARD))
```

- `Transition::of(props)` names the properties; `Transition::all()` covers
  every animatable one the view sets. `duration` and `ease` take a value or
  a token (`Themed<f32>`, `Themed<Ease>`), so the theme's motion tokens
  (and reduced motion through them) apply.
- `.delay(…)` waits before starting, as CSS's `transition-delay`.
- `.transition(…)` may be called more than once; a later one for the same
  property wins, as with any style.
- A variant may set its own transition (`.hover(|s| s.transition(…))`):
  the transition that applies to a change is the one in effect *after* it,
  as in CSS (a hover can fade in fast and out slow).

`Prop` is the existing animatable property set, the one the anime engine's
`Key::for_element` and `Prop::animatable()` already name: opacity, colours
(background, text, border, outline, fill), radius, outline width and
offset, blur and the colour filters, transform, and sizes.

### Luau

React's shape is CSS's: `transition` beside the properties it eases. Two
spellings, the table preferred, the string for those coming from CSS:

```lua
ui.button {
  bg = "$button.bg",
  hover = { bg = "$button.hover" },
  transition = { bg = { duration = "$motion.duration.fast", ease = "$motion.ease.standard" } },
}

ui.box { opacity = open, transition = { opacity = 200 } }        -- ms, default ease
ui.box { transition = "opacity 200ms ease-out, bg 120ms" }       -- CSS's grammar
ui.box { transition = { all = "$motion.duration.normal" } }
```

`hover`, `focus`, `active` and `disabled` tables may hold a `transition`
too. The lint learns the property names; the types file grows a
`Transition` type. With this, `hover = { … }` tables that ease are plain
values again, and the "animate the base style instead" error points at
`transition`.

## How it runs

### One engine in zgx core

Transitions live in zgx core, not zgx-script: Rust views (the focus ring,
the scrollbar, the tooltip) need them, and Luau lowers onto them as it does
onto every other style. They share the anime engine's pieces rather than
its engine: its easing (`Ease`, which moves from `zgx-script/src/anime/ease.rs`
to zgx core, and learns to resolve `$motion.ease.*`) and interpolation
(`V::lerp`, and `theme::value::mix` for token colours, which keeps a
colour easing between two tokens right through a theme change).

A `Ui` holds one `Transitions` set: a map from node to that node's running
transitions (property, from, to, start, duration, ease). It is empty for a
window with nothing easing.

### Starting

`mount_style` resolves a view's style as today. Where the view has a
transition (a bit in its rare styles, read where the resolved style is
compared with what was written), each transitioned property whose new
value differs from the one drawn starts a transition from the drawn value
to the new one, instead of writing the new one. Everything else is written
as today.

### Stepping, and idling

The set steps on the host's interaction deadlines, as zgx core's other
timed motion does (the overlay scrollbar's fade, the drag spring, the
caret): `Ui::next_interaction_deadline` gives a deadline a refresh ahead
while the set is non-empty and none while it is empty, and
`Ui::advance_interactions` steps it, the host framing on that deadline as
on the others. So a window easing nothing has no deadline, sets no timer
and does no work.

(This note first proposed a `FrameClock` task per `Ui`, as the anime
engine's loop runs. Step one took the deadlines instead: zgx core reaches
the `FrameClock` only through a component's environment, which the style
effect isn't given, and the deadlines already are core's frame schedule
for timed motion, leaving nothing behind at rest.)

Each frame, every running transition's value is computed and written to
the scene by the cheapest path for its property:

- opacity, blur and the colour filters, transform: `set_effects`,
  `set_transform` (composite only, no repaint of the node's content);
- background, border and outline colour, radius, outline width and offset:
  the node's `QuadStyle` (`set_kind`, a repaint without layout). A new
  write path patches these fields of the current quad without the full
  `apply_resolved`, which they take today;
- sizes: `set_style`, which lays out. Allowed, warned by the lint (as
  `zoom` is), since a transitioned width lays out every frame.

A transition that reaches its end writes its target exactly (no rounding
left behind) and leaves the set. When the set empties the task goes back
to waiting: a finished transition leaves no timer and no frame request.

Time comes from `zgx::clock`, so a video rendered offline (`VirtualClock`)
eases deterministically, frame for frame.

### Interruption and retargeting

The new target always starts from the value drawn now, never from the old
start: a hover left halfway reverses from where it is.

- **Retargeted**: a transition running when its property changes again
  starts afresh from the drawn value to the new target, its full duration.
- **Reversed** (the new target is the running transition's start): its
  duration is the elapsed share of the old one, as CSS's reversing
  shortening factor, so a hover left at once returns as fast as it came.
- **Cancelled**: removing `transition` from a view, or the node leaving
  the scene, drops its entries and writes the target.
- **Owned by an animation**: a property an anime clip or a `ui.tween`
  binding drives is that animation's; a transition never fights it. The
  anime engine's `claim` table gains a core-side counterpart, and an
  explicit animation wins over a transition, as CSS animations win over
  transitions.

### Theme changes

A token re-resolving changes the drawn value like any other change, so a
transitioned property eases into a new theme. That is CSS's behaviour and
reads well for a single control, but a whole window cross-fading on a
theme switch is a choice. Proposed: a theme switch writes targets at once
(no transitions start while `Themes::set` applies), and a later opt-in can
ease it. Open for Wing.

## Reduced motion

`SystemPreferences::reduced_motion` already sets the motion duration tokens
to 0, so any transition whose duration is a `$motion.duration.*` token
lands at once. Literal durations are the app's, but zgx follows the
platforms' guidance (Apple's and WCAG's are about *motion*):

- transitions of transform, position and size land at once under reduced
  motion, whatever their duration;
- opacity and colour transitions keep their duration: a fade is not
  motion, and replacing motion with a fade is what the guidance asks.

The built-in transitions (the GNOME focus ring, the scrollbar fade, the
tooltip fade once moved onto this) all use the tokens, so they follow it.

## Zero cost for nodes without transitions

- No transition: a bit in the node's rare styles, already read when its
  style resolves, is clear; nothing else is looked at. The `Transitions`
  set and its task do nothing for a window with none running.
- No new schema tokens (the motion tokens exist).
- The new `QuadStyle` patch path is taken only by a running transition.
- Gate: the x86 gate's cells (`widgets`, `pointer-move`, `mount`,
  `style`, the `micro-*` and app cells) must read ≤ 0, as #1666 showed a
  few new tokens can cost every cell; and a new cell, hover transitions
  over a list, says what a transition costs the frame that has one.

## Moving the hand-made fades onto it

Once it exists: the tooltip fade, the checkbox's draw-on and the progress
sweep (each an 8–16 ms sleep loop today, ignoring reduced motion) become
transitions of opacity or a clip, on frames rather than sleeps; GNOME's
focus ring gets its 200 ms; GTK's scrollbar fade-in uses it. The overlay
scrollbar's fade and widening stay on their deadline schedule (they are
timed waits, not a value easing), but their easing curves come from the
shared `Ease`.

## Steps

1. `Ease` and the interpolation helpers into zgx core; the `Transitions`
   set and its frame task; opacity and colour transitions from Rust; the
   `QuadStyle` patch path. Tests: an eased value at mid-duration, the exact
   target at the end, `wants_frame()` false after, retarget and reverse,
   reduced motion, a gate showing ≤ 0.
2. Sizes, outline offset and width, transform; the lint warning for sizes.
3. Luau: the `transition` prop (table and string), in variants too; types
   and lint.
4. The built-in fades move onto it; GNOME's focus ring transition.

## Open questions

- Should a theme switch ease transitioned properties (CSS does) or land at
  once (proposed)?
- Should zgx's own controls get transitions by default (a hover fading in
  over `motion.duration.fast`), or only where a platform's look has them?
- Springs as a transition curve (`ui.spring`'s stiffness and damping),
  beside duration-and-ease, as SwiftUI offers?
