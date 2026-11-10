"""A font of one colour glyph (COLR v0, U+E000): a square in opaque red, for
zgx-gpu's tests. Run with fontTools: python3 make_colr.py <out.ttf>."""
import sys
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.colorLib.builder import buildCOLR, buildCPAL

def square(lo, hi):
    pen = TTGlyphPen(None)
    pen.moveTo((lo, lo)); pen.lineTo((lo, hi)); pen.lineTo((hi, hi)); pen.lineTo((hi, lo)); pen.closePath()
    return pen.glyph()

fb = FontBuilder(1000, isTTF=True)
order = [".notdef", "square", "square.layer"]
fb.setupGlyphOrder(order)
fb.setupCharacterMap({0xE000: "square"})
empty = TTGlyphPen(None).glyph()
fb.setupGlyf({".notdef": empty, "square": square(0, 800), "square.layer": square(0, 800)})
fb.setupHorizontalMetrics({".notdef": (500, 0), "square": (1000, 0), "square.layer": (1000, 0)})
fb.setupHorizontalHeader(ascent=900, descent=-100)
fb.setupNameTable({"familyName": "ZgxColrSquare", "styleName": "Regular", "uniqueFontIdentifier": "ZgxColrSquare-Regular", "fullName": "ZgxColrSquare Regular", "psName": "ZgxColrSquare-Regular"})
fb.setupOS2(sTypoAscender=900, sTypoDescender=-100, usWinAscent=900, usWinDescent=100)
fb.setupPost()
fb.font["COLR"] = buildCOLR({"square": [("square.layer", 0)]})
fb.font["CPAL"] = buildCPAL([[(1.0, 0.0, 0.0, 1.0)]])
fb.save(sys.argv[1])
