---
section: fixed
---
- **Pictures drawn together share the image budget evenly** (zgx-gpu), whichever is drawn first: small ones keep their size, large ones share what's left, and take their room back as others go.
- **At 16,384 images, those not drawn this frame give way** (zgx-gpu), rather than a new one being left out.
- **A layer scaled far down rasterizes at what shows** (zgx-gpu), down to a sixteenth rather than a half, so a picture laid out at full size under a scaling parent takes about a megabyte, not 13.
