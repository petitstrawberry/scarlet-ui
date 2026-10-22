# Font sizing and text width

The `font_size` API specifies logical pixels per em. The `ab_glyph::PxScale`
type instead specifies ascent minus descent. Convert each selected face using
`em_pixels * height_unscaled / units_per_em` before measuring or rasterizing.
Using the same raw `PxScale` for different faces incorrectly makes font size
depend on each face's vertical metrics.

For the bundled Mplus 1 font, the em is 1000 font units and ascent minus descent
is 1448. A fullwidth Japanese glyph at 14 logical pixels should advance 14 px;
passing 14 straight to `PxScale` gives only 9.6685 px. Noto Sans Symbols 2 has
1699 vertical units per 1000-unit em, so raw-height scaling shrinks its fallback
glyphs further. Widths come from font advances; no extra letter spacing is added.

Measurement, TextView/TextField prefix widths and CPU/SGFX glyph positions share
the same per-face conversion. Fractional advances accumulate before rounding.
Glyph raster cache keys retain the exact physical scale, including fractional
font sizes and display scale, so different requested masks cannot collide after
integer truncation. Buttons measure label dimensions rather than multiplying
UTF-8 byte counts by an estimated character width.

Fallback discovery also verifies that a candidate has a vector outline for the
requested visible glyph. A cmap hit in a bitmap/color-only font cannot be drawn
by this renderer; discovery continues to another candidate instead of accepting
coverage that silently paints nothing. Whitespace remains valid without ink.
Initial font discovery publishes the font and its completed-attempt flag under
one lock, so concurrent layout cannot cache temporary no-font metrics while
the primary font is still loading.

## Measured result

With Mplus 1 as primary and Noto Sans CJK JP, Noto Sans Symbols 2 and
JetBrainsMono Nerd Font Mono as explicit fallbacks, the 14 px probe reports:

| Sample | Previous measured width | Corrected measured width |
|---|---:|---:|
| `AVATAR WiFi minimum 0123456789` | 173 px | 250 px |
| `日本語の文字幅、句読点。「確認」` | 155 px | 224 px |
| `Scarlet 日本語 123 — [OK]（保存）` | 165 px | 238 px |
| Fallback snowman, power and terminal symbols | 74 px | 108 px |

The fallback row includes its `Fallback: ` label and spaces. These corrected
widths agree with sums of the font tables' horizontal advances divided by units
per em. The probe also records physical glyph positions at each display scale;
ink bounds may be narrower than advance bounds because of glyph side bearings.
Before/after Winit CPU screenshots at 100%, 125% and 200% used the same current
Scarlet Rust 26d00a8 compiler and the same font files; the baseline was built
from an isolated pre-fix source snapshot.

Final validation passed 478 core tests (3 ignored) and 90 SGFX renderer tests.
Cache reuse tests retain strict storage-identity and zero-allocation assertions
within an unchanged font epoch; a concurrent fallback discovery legitimately
invalidates layout and is recorded before retrying that scenario.

## Reproduction

Run `cargo run -p scarlet-ui --example typography-smoke --features platform-winit`.
Set `SCARLET_UI_WINIT_RENDERER=cpu` to select the CPU renderer presented through
Softbuffer, as used for the screenshots.
For consistent Scarlet fonts, set `SCARLET_TYPOGRAPHY_FONT_DIR` to the bundled
`share/fonts` directory. `--metrics-only` prints logical measured dimensions and
physical glyph positions for 12, 14, 18 and 24 px at 100%, 125% and 200% scale.
The runtime example shows labels, a button, a single-line field and multiline
text using Latin, Japanese, mixed punctuation and explicit fallback characters.
On X11, `WINIT_X11_SCALE_FACTOR=1.25` selects a fractional display scale.

Tiny original metric fixtures in `crates/scarlet-ui-core/src/testing/fonts`
exercise different primary and fallback vertical metrics without relying on
installed fonts. The generator
requires fontTools only when regenerating the committed fixtures; normal Rust
tests need no additional tooling. Regression tests compare known em advances,
glyph placement, fallback baselines, fractional cache behavior, text prefixes,
zero-width join/variation formatting controls and multiline metrics. Existing
text layout and rendering tests cover editing.

The renderer currently lays out Unicode scalars and does not claim complete
script shaping, ligatures, bidirectional layout or kerning. This correction
changes size conversion and cache identity, not the shaping model. Join and
variation controls do not draw replacement marks or add phantom width, even if
the selected fonts omit them; a joined emoji may still draw as separate glyphs.
