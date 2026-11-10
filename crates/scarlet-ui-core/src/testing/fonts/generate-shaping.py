"""Original geometric OpenType fixtures for shaping; fontTools is generation-only."""
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.ttLib import TTFont, TTCollection

root = Path(__file__).parent
builder = FontBuilder(1000, isTTF=True)
widths = {'.notdef': 600, 'space': 300, 'A': 700, 'V': 690, 'f': 350,
          'i': 280, 'fi': 510, 'acutecomb': 0}
builder.setupGlyphOrder(list(widths))
builder.setupCharacterMap({32: 'space', 65: 'A', 86: 'V', 102: 'f', 105: 'i', 0x301: 'acutecomb'})
outlines = {}
for name, width in widths.items():
    pen = TTGlyphPen(None)
    if name not in ('.notdef', 'space'):
        bounds = (0, 0, 120, 100) if name == 'acutecomb' else (40, -180, width - 40, 740)
        x0, y0, x1, y1 = bounds
        pen.moveTo((x0, y0)); pen.lineTo((x1, y0)); pen.lineTo((x1, y1)); pen.lineTo((x0, y1)); pen.closePath()
    outlines[name] = pen.glyph()
builder.setupGlyf(outlines)
builder.setupHorizontalMetrics({name: (width, 0 if name == 'acutecomb' else 40) for name, width in widths.items()})
builder.setupHorizontalHeader(ascent=800, descent=-200)
builder.setupNameTable({'familyName': 'shaping-fixture', 'styleName': 'Regular', 'fullName': 'shaping-fixture', 'psName': 'shaping-fixture'})
builder.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
builder.setupPost(); builder.setupMaxp()
addOpenTypeFeaturesFromString(builder.font, '''
languagesystem DFLT dflt;
languagesystem latn dflt;
feature kern { pos A V -120; } kern;
feature liga { sub f i by fi; } liga;
markClass acutecomb <anchor 0 0> @TOP;
feature mark { pos base A <anchor 350 800> mark @TOP; } mark;
''')
builder.font['head'].created = builder.font['head'].modified = 3406620150
builder.save(root / 'shaping.ttf')
collection = TTCollection()
collection.fonts = [TTFont(root / 'metrics-primary.ttf'), TTFont(root / 'shaping.ttf')]
collection.save(root / 'shaping-collection.ttc')
