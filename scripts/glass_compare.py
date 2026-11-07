#!/usr/bin/env python3
"""zgx's drawn glass against the system's, by measurement.

    python3 scripts/glass_compare.py [--top N] system.png drawn.png [more pairs...]

The system's: `GLASS_OUT=system.png ZGX_NO_ACTIVATE=1 cargo run -p zgx-desktop
--example glass` on macOS 26, its window's own pixels (`--top 28` for its title
bar, in points). Drawn: `cargo run -p zgx-gpu --example glass_compare -- drawn.png`.
Both lay out twenty stripes under a pane at 2x. Over the pane's middle (clear of
its rim and of any text) it compares mean colour, saturation (a channel's spread)
and the stripes' surviving contrast, in sRGB levels of 255: a capture in Display
P3 is converted first.
"""
import argparse
import io

import numpy as np
from PIL import Image, ImageCms

LUMA = np.array([0.2126, 0.7152, 0.0722])


def load(path):
    image = Image.open(path)
    profile = image.info.get('icc_profile')
    image = image.convert('RGB')
    if profile:
        source = ImageCms.ImageCmsProfile(io.BytesIO(profile))
        image = ImageCms.profileToProfile(image, source, ImageCms.createProfile('sRGB'), outputMode='RGB')
    return np.asarray(image, dtype=float)


def middle(pixels, top):
    # Points to pixels: the pane is at (60, 60), 300 by 200, at 2x.
    y = top * 2
    return pixels[y + 200:y + 520, 300:540]


def stats(pixels):
    luma = (pixels @ LUMA).mean(axis=1)
    return {
        'mean': pixels.mean(axis=(0, 1)).round(1).tolist(),
        'saturation': round(float((pixels.max(axis=2) - pixels.min(axis=2)).mean()), 1),
        'contrast': round(float(luma.max() - luma.min()), 1),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--top', type=int, default=0, help="points of title bar over the system's")
    parser.add_argument('images', nargs='+')
    args = parser.parse_args()
    for system, drawn in zip(args.images[0::2], args.images[1::2]):
        a, b = middle(load(system), args.top), middle(load(drawn), 0)
        print(system.rsplit('/', 1)[-1], 'against', drawn.rsplit('/', 1)[-1])
        print('  system', stats(a))
        print('  drawn ', stats(b))
        print('  mean error by channel', np.abs(a - b).mean(axis=(0, 1)).round(1).tolist())


main()
