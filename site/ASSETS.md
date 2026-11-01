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
