---
section: fixed
---
- **The default output plays whatever PCM format its device asks for** (zgx-audio, `device`): 8-, 16-, 24-, 32- and 64-bit integers, signed or unsigned, and 32- or 64-bit floats; before, a device whose default was anything but 16-bit integers or 32-bit floats (common on ALSA hardware) failed to start. A sample at exactly full scale no longer wraps to the most negative 24-bit value, and one that is not a number is written as silence. A device that asks for DSD says so, and is not started.
