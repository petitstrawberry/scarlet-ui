"""Generate tiny original metric fixtures; requires fontTools only to regenerate.

These are intentionally geometric test glyphs, not redistributions of UI fonts.
The primary and fallback use the same em but different vertical font metrics.
"""
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen


def build(name, ascent, descent, characters, blank_chars=()):
    builder = FontBuilder(1000, isTTF=True)
    glyphs = {".notdef": (600, None), "space": (300, None)}
    cmap = {32: "space"}
    for char, width in characters:
        glyph = f"uni{ord(char):04X}"
        glyphs[glyph] = (width, None if char in blank_chars else (40, -180, width - 40, 740))
        cmap[ord(char)] = glyph
    builder.setupGlyphOrder(list(glyphs))
    builder.setupCharacterMap(cmap)
    outlines = {}
    for glyph, (_, bounds) in glyphs.items():
        pen = TTGlyphPen(None)
        if bounds:
            x0, y0, x1, y1 = bounds
            pen.moveTo((x0, y0))
            pen.lineTo((x1, y0))
            pen.lineTo((x1, y1))
            pen.lineTo((x0, y1))
            pen.closePath()
        outlines[glyph] = pen.glyph()
    builder.setupGlyf(outlines)
    builder.setupHorizontalMetrics({glyph: (width, 0) for glyph, (width, _) in glyphs.items()})
    builder.setupHorizontalHeader(ascent=ascent, descent=descent)
    builder.setupNameTable({"familyName": name, "styleName": "Regular", "fullName": name, "psName": name})
    builder.setupOS2(sTypoAscender=ascent, sTypoDescender=descent,
                    usWinAscent=ascent, usWinDescent=-descent)
    builder.setupPost()
    builder.setupMaxp()
    builder.save(Path(__file__).with_name(f"{name}.ttf"))


build("metrics-primary", 1160, -288,
      [("A", 700), ("V", 690), ("i", 280), ("W", 920), ("日", 1000), ("、", 1000), ("?", 600)])
build("metrics-fallback", 1069, -630, [("⏻", 900), ("☃", 1000)])
build("metrics-blank", 1160, -288, [("A", 700)], blank_chars=("A",))
