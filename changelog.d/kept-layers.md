---
section: performance
---

- A frame where only what's inside layers changes (pictures on springs under a layer shader, an opacity group fading) patches the window's last frame: the changed layers are painted again and their composites made again, and everything else is drawn as it was kept, where before the whole window's items were made again. A parallax wallpaper under a still page takes 42% fewer instructions a moving frame. Blends that read what's behind them still draw as before.
