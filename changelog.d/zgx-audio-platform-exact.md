---
section: fixed
---
- **zgx-audio renders the same samples on every platform** (zgx-audio): the limiter's release, exponential ramps, the compressor's and ducker's times, the reverb's and peaking filter's gains, the spatial maths and the binaural tables now come from the crate's own exponentials, logarithms and angles. These are made of IEEE arithmetic alone, not the platform's maths library, whose last bits differ by system. A last bit off in the limiter's release compounded to 84 dB under the signal, so a render on Linux differed from the same render on a Mac. A test keeps the platform's functions out of the crate's code.
