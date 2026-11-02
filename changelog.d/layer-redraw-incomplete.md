---
section: fixed
---
- **A cached layer draws again once a picture in it has decoded** (zgx-gpu): a layer (a layer shader's, an isolated group's) drawn while a picture in it was still decoding was kept as it was drawn, without the picture, until something inside changed style: Zeron's wallpaper under an effect (scanlines, dither) came up blank as its picture changed, until a zoom drew it again. A layer drawn without something on its way (a picture decoding, a shader fill compiling) is drawn again rather than reused.
