# Homepage fallback specimen

`public/assets/widget-factory.png` is the unmodified 400×300 pixel canvas capture
of the actual ScarletUI homepage Specimen's default state. It was captured from
the locally built WebAssembly/SGFX application in Chromium with CPU SwiftShader
on 2026-11-01. Browser chrome was excluded using the observed canvas bounds;
controls, labels, colors and state were not retouched or generated.

The static capture is displayed when JavaScript or a WebGPU adapter is
unavailable, or initialization/runtime rendering fails. It does not claim a
native-window or hardware-GPU capture. The live version uses the same
SgfxCanvas, Button, Slider and State types as the native Widget Factory.
The sketch capture shows its initial editable mountain-and-sun strokes.

The full Widget Factory also uses the repository's existing Tabler icon set. Its MIT notice
is included in each combined site package as `tabler-license.txt`.

`public/assets/counter-example.png` is the actual 240×144 default-state capture
of the compiled counter (`Count: 0` and `+1`), taken with the same local software-WebGPU
configuration and lossless canvas-only crop. It is the lower example's fallback.

## Sharing images

Sharing cards are rendered at build time by `scripts/social-image.mjs`, using
pinned Sharp and the repository's DejaVu Sans font. The bold face in
`assets/fonts/DejaVuSans-Bold.ttf` is the unmodified DejaVu Sans Bold face from
DejaVu Fonts, distributed under the adjacent font license. It is a build input,
not a new logo or a web-font download. Titles and descriptions come from each
built page. The homepage card adds selected real stock-control pixels from
`assets/sharing/`; other pages remain typographic. Capture provenance and crop
bounds are documented in `assets/sharing/PROVENANCE.md`. No AI-generated artwork or external image service is involved.

## README and application links

The root README reuses `public/assets/widget-factory.png` above without edits.
It links to the live site; it is a browser sample capture, not a native app screenshot.

The homepage's “Built with ScarletUI” links were checked against the public
[Kureha Code README](https://github.com/petitstrawberry/kureha-code#readme) and
[Resonara README](https://github.com/petitstrawberry/resonara#readme) on 2026-11-05.
Both explicitly identify ScarletUI as their UI toolkit. Descriptions reflect
those READMEs; they do not imply production readiness or verified platform coverage.
No application screenshots are included: their public source trees did not
contain suitable screen captures. Add a visual showcase only with actual,
verified captures and their provenance, not illustrations or app icons.
