---
section: added
---
- **Style transitions, step one** (zgx): `Styled::transition(Transition::of([TransitionProp::Opacity, TransitionProp::Background]).duration(…).ease(…))` eases a view's opacity, background or border colour from what is drawn to each new value (a hover, a `reactive_style`, a theme), as CSS's `transition` does: from the drawn value when it changes again, back as fast as it went, landing at once where its duration is a motion token under reduced motion. It steps on the window's interaction deadlines only while something eases, and a view without one costs nothing (docs/design/style-transitions.md).
