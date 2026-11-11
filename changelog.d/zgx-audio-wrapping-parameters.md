---
section: fixed
---
- **An effect's angle goes the short way round** (zgx-audio). `Parameter::wrapping()` makes a parameter's range one turn of a circle: any value set is taken round into it, and a move goes the short way. `Binaural::AZIMUTH` wraps, so a source passing behind the listener goes from 179 to -179 degrees through 180, not round the front.
