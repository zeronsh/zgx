"""Two fonts of squares for zgx-gpu's fallback tests: ZgxKana covers the
hiragana あいう (U+3042, U+3044, U+3046), ZgxHan the simplified Chinese 车
and 见 (U+8F66, U+89C1), as a Japanese font lacks them. Run with fontTools:
python3 make_han_kana.py <out dir>."""
import sys
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

def square():
    pen = TTGlyphPen(None)
    pen.moveTo((100, 0)); pen.lineTo((100, 800)); pen.lineTo((900, 800)); pen.lineTo((900, 0)); pen.closePath()
    return pen.glyph()

def build(name, codepoints, path):
    fb = FontBuilder(1000, isTTF=True)
    glyphs = [".notdef"] + [f"u{cp:04X}" for cp in codepoints]
    fb.setupGlyphOrder(glyphs)
    fb.setupCharacterMap({cp: f"u{cp:04X}" for cp in codepoints})
    fb.setupGlyf({g: (TTGlyphPen(None).glyph() if g == ".notdef" else square()) for g in glyphs})
    fb.setupHorizontalMetrics({g: (1000, 0) for g in glyphs})
    fb.setupHorizontalHeader(ascent=900, descent=-100)
    fb.setupNameTable({"familyName": name, "styleName": "Regular", "uniqueFontIdentifier": f"{name}-Regular", "fullName": f"{name} Regular", "psName": f"{name}-Regular"})
    fb.setupOS2(sTypoAscender=900, sTypoDescender=-100, usWinAscent=900, usWinDescent=100)
    fb.setupPost()
    fb.save(path)

out = sys.argv[1]
build("ZgxKana", [0x3042, 0x3044, 0x3046], f"{out}/ZgxKana.ttf")
build("ZgxHan", [0x8F66, 0x89C1], f"{out}/ZgxHan.ttf")
