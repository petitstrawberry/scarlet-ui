# Stock-control source pixels

Lossless crops of actual ScarletUI rendering, refreshed on 2026-11-05 from
Widget Factory built with `cargo build --locked -p scarlet-ui-widget-factory`.
The UI source is main `72fb4135d85ea279a221c5db8e8efda2b210863d`; subsequent
README branch changes only affect documentation and site assets.

Capture used the existing Nix developer environment, Xvfb, winit with
`WINIT_X11_SCALE_FACTOR=2`, and the SGFX wgpu backend with Mesa software rendering.
The actual client window was 1720×1120 physical pixels, corresponding to its
860×560 logical size. Normal pointer clicks and wheel scrolling selected pages.
No widget styling, text, or state was redrawn or retouched.

Crop bounds `(x, y, width, height)` in the retained client-window captures:

- `tabs.png`: Overview's Mixer / Editor tab bar, `(960,856,250,80)` in `overview-tabs.png`.
- `button.png`: Controls' standard Factory Button, `(760,792,192,82)` in `controls.png`.
- `toggle.png`: Controls' enabled Toggle, `(936,897,84,44)` in `controls.png`.
- `slider.png`: Controls' complete Slider, `(775,973,620,40)` in `controls.png`.

Source SHA-256:

- `overview-tabs.png`: `3a00a14fbeb5cf4b998ce73ed7c94c92288fbd5ad54e1c82df0d182241e69c5d`
- `controls.png`: `94b2ede10b138e1cff695eefe78f9d909cccfc2748eaa7902f77b803fb2bdc99`

`social-image.mjs` lays these source pixels on white with the project title and
description. It reduces 2× captures by 0.8, with wide controls capped at 360
pixels to keep the full slider within the composition. The README snapshot is
regenerated with `npm run update-readme-card`; the site's sharing image is built
from the same inputs and renderer. A regression check requires identical output.

Full client-window captures, build logs and capture harnesses are retained in
the task's local QA artifacts. The repository includes the necessary crops only.
These are native UI captures, not browser captures or hardware-GPU claims.
