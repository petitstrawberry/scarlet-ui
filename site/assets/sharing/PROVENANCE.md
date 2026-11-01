# Stock-control source pixels

Lossless crops of actual ScarletUI/SGFX rendering, captured on 2026-11-01 in
approved local Chromium/SwiftShader QA at ordinary browser zoom 200%. These are
not HTML mockups, generated illustrations, or redrawn widget designs. The stock
36×20 logical Toggle occupies 72×40 physical pixels, confirming 2× rendering;
DPR was not separately introspected.

Coordinates below refer to retained native window screenshots (1188×848):

- `tabs.png`: Factory Overview stock TabView, Mixer / Editor group only,
  `(768,664)-(1008,724)` in `factory-tabs-2x-window.png`.
- `button.png`: Sketch stock Clear button with small white margins,
  `(893,328)-(1007,388)` in `sketch-controls-2x-window.png`.
- `toggle.png`: Factory Controls enabled Toggle with margins,
  `(911,350)-(993,400)` in `factory-controls-2x-window.png`.
- `slider.png`: Factory Controls Slider,
  `(779,426)-(1126,468)` in `factory-controls-2x-window.png`.

`social-image.mjs` reduces these native 2× captures by 0.8 with Lanczos filtering,
producing 1.6× logical controls without enlarging low-resolution source pixels.
It lays them out on white without changing labels or widget styling. Refresh
captures when defaults change. Full screenshots remain in local QA artifacts;
only necessary UI pixels are included in the repository.
