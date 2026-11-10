"""Three fonts for zgx-gpu's fallback test: every glyph a box, so only which
face draws a character matters. ZgxKana has a Japanese sentence's characters
and a Chinese one's but its simplified forms 车 and 见; ZgxHans has those two;
ZgxEvery has them all, standing in for a face that holds everything (Arial
Unicode MS on macOS). Run with fontTools: python3 make_kana_han.py <dir>."""
import sys
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

JAPANESE = "今日は駅で会いましょう。"
CHINESE = "明天在站面吧。"  # 明天在车站见面吧 without 车 and 见
SIMPLIFIED = "车见"


def box():
    pen = TTGlyphPen(None)
    pen.moveTo((100, 0)); pen.lineTo((100, 800)); pen.lineTo((900, 800)); pen.lineTo((900, 0)); pen.closePath()
    return pen.glyph()


def font(family, text, out):
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder([".notdef", "space", "box"])
    cmap = {0x20: "space"}
    cmap.update({ord(c): "box" for c in text})
    fb.setupCharacterMap(cmap)
    empty = TTGlyphPen(None).glyph()
    fb.setupGlyf({".notdef": empty, "space": empty, "box": box()})
    fb.setupHorizontalMetrics({".notdef": (500, 0), "space": (500, 0), "box": (1000, 0)})
    fb.setupHorizontalHeader(ascent=900, descent=-100)
    fb.setupNameTable({"familyName": family, "styleName": "Regular", "uniqueFontIdentifier": f"{family}-Regular",
                       "fullName": f"{family} Regular", "psName": f"{family}-Regular"})
    fb.setupOS2(sTypoAscender=900, sTypoDescender=-100, usWinAscent=900, usWinDescent=100)
    fb.setupPost()
    fb.save(out)


out = Path(sys.argv[1])
font("ZgxKana", JAPANESE + CHINESE, out / "ZgxKana.ttf")
font("ZgxHans", SIMPLIFIED, out / "ZgxHans.ttf")
font("ZgxEvery", JAPANESE + CHINESE + SIMPLIFIED, out / "ZgxEvery.ttf")
