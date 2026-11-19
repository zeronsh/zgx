"""Two fonts with no x-height in their OS/2 table (sxHeight 0), for the
x-height a span aligned to its middle takes (`Font::x_height_em`):
ZgxLowX's `x` is a box 400 units of 1,000 tall, so its x-height is measured
from it; ZgxNoX has no `x`, so its x-height falls back. Every other letter
and digit is a box 700 tall. Run with fontTools:
python3 make_x_height.py <dir>."""
import string
import sys
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen


def box(height):
    pen = TTGlyphPen(None)
    pen.moveTo((50, 0)); pen.lineTo((50, height)); pen.lineTo((550, height)); pen.lineTo((550, 0)); pen.closePath()
    return pen.glyph()


def font(family, with_x, out):
    fb = FontBuilder(1000, isTTF=True)
    letters = [c for c in string.ascii_letters + string.digits if with_x or c != "x"]
    names = [".notdef", "space"] + [f"g{ord(c)}" for c in letters]
    fb.setupGlyphOrder(names)
    cmap = {0x20: "space"}
    cmap.update({ord(c): f"g{ord(c)}" for c in letters})
    fb.setupCharacterMap(cmap)
    empty = TTGlyphPen(None).glyph()
    glyphs = {".notdef": empty, "space": empty}
    glyphs.update({f"g{ord(c)}": box(400 if c == "x" else 700) for c in letters})
    fb.setupGlyf(glyphs)
    metrics = {".notdef": (600, 0), "space": (300, 0)}
    metrics.update({f"g{ord(c)}": (600, 50) for c in letters})
    fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=900, descent=-100)
    fb.setupNameTable({"familyName": family, "styleName": "Regular", "uniqueFontIdentifier": f"{family}-Regular",
                       "fullName": f"{family} Regular", "psName": f"{family}-Regular"})
    fb.setupOS2(sTypoAscender=900, sTypoDescender=-100, usWinAscent=900, usWinDescent=100, sxHeight=0,
                version=2)
    fb.setupPost()
    fb.save(out)


out = Path(sys.argv[1])
font("ZgxLowX", True, out / "ZgxLowX.ttf")
font("ZgxNoX", False, out / "ZgxNoX.ttf")
