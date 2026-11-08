#!/usr/bin/env python3
"""zgx's drawn glass against the system's, pixel by pixel.

    python3 scripts/glass_compare.py system_dir drawn_dir

Each directory holds `{glass,clear}-{light,dark}-{patches,edge}.png`, a
1000 by 800 point window at 2x. `patches` is twenty solid patches, 200 points
square, each under an 80 point pane with corners of 16; `edge` is the window
black on its left half and white on its right, under one 600 by 400 pane at
(200, 200). Drawn: `cargo run -p zgx-gpu --example glass_compare` (its doc says
how to make each). The system's: the same layout of `NSGlassEffectView`s in a
key window on macOS 26, its own pixels read back (a capture in Display P3 is
converted to sRGB first).

It prints the mean difference, in sRGB levels of 255, inside the panes, on
their rim (the outer point and a half), and in the shadow past them.
"""
import argparse
import io

import numpy as np
from PIL import Image, ImageCms


def load(path):
    image = Image.open(path)
    profile = image.info.get('icc_profile')
    image = image.convert('RGB')
    if profile:
        source = ImageCms.ImageCmsProfile(io.BytesIO(profile))
        image = ImageCms.profileToProfile(image, source, ImageCms.createProfile('sRGB'), outputMode='RGB')
    return np.asarray(image, dtype=float)


def panes(scene):
    if scene == 'edge':
        return [(200, 200, 600, 400)]
    return [((i % 5) * 200 + 60, 800 - (i // 5 + 1) * 200 + 60, 80, 80) for i in range(20)]


def distance(shape, boxes, radius=16):
    """Each pixel's signed distance in points to the nearest pane's edge."""
    y, x = np.mgrid[0:shape[0], 0:shape[1]]
    x, y = (x + 0.5) / 2, (y + 0.5) / 2
    nearest = np.full(shape, np.inf)
    for bx, by, bw, bh in boxes:
        qx = np.abs(x - (bx + bw / 2)) - bw / 2 + radius
        qy = np.abs(y - (by + bh / 2)) - bh / 2 + radius
        d = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - radius
        nearest = np.minimum(nearest, d)
    return nearest


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('system')
    parser.add_argument('drawn')
    args = parser.parse_args()
    for variant in ['glass', 'clear']:
        for appearance in ['light', 'dark']:
            for scene in ['patches', 'edge']:
                name = f'{variant}-{appearance}-{scene}.png'
                system, drawn = load(f'{args.system}/{name}'), load(f'{args.drawn}/{name}')
                d = distance(system.shape[:2], panes(scene))
                error = np.abs(system - drawn).mean(axis=2)
                regions = {
                    'inside': d < -1.5,
                    'rim': (d >= -1.5) & (d < 0.5),
                    'shadow': (d >= 0.5) & (d < 60),
                }
                print(f'{name:28}' + '  '.join(f'{k} {error[m].mean():5.2f}' for k, m in regions.items()))


if __name__ == '__main__':
    main()
