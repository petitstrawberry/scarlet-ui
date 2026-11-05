# Homepage fallback specimens

`public/assets/widget-factory.png` is an unmodified 800×600 physical-pixel
capture of the actual shared `Specimen` at its 400×300 logical size, refreshed
on 2026-11-05. `public/assets/counter-example.png` is the corresponding 480×288
capture of the real compact Counter at 240×144 logical pixels (`Count: 0`).
Both images display at their logical sizes in the site.

The captures use the current UI source at main `72fb413`, built in the existing
Nix environment with winit, Xvfb, a 2× scale factor, and SGFX's wgpu backend on
Mesa software rendering. The same embedded DejaVu Sans font as the Web samples
was installed. The actual client window was captured without browser chrome,
retouching, generated artwork, or replacing native controls with HTML.

A temporary native harness compiled the existing shared Specimen and Counter
view/state code unchanged. It enabled Specimen's native compilation, selected
the compact Counter's existing body, and used the embedded canvas dimensions
as the native client-window sizes. These are native captures of the shared UI;
they do not claim a successful WebGPU browser capture. Browser execution in
this environment failed during its Wasm callback/frame startup; no black or
blank frames were accepted as source images. The actual browser versions still
use their existing Rust entry points.

The static captures display when JavaScript or a WebGPU adapter is unavailable,
or initialization/runtime rendering fails. The Sketch image contains its
initial editable mountain-and-sun strokes. Source hashes are
`6e777afec1dec34db8ebee440ac3efd310ab53ec61d6af90d473b23449207aa1`
(Specimen) and `4c7e62512478ec1aa2f812cf35c092f230e18c9e28895b16379cee01392ebf51`
(Counter). Full captures, harness source and build logs are retained in local QA artifacts.

The full Widget Factory also uses the repository's existing Tabler icon set. Its MIT notice
is included in each combined site package as `tabler-license.txt`.

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

The root README uses `public/assets/project-card.png`, a repository-visible
snapshot of the site's homepage sharing image. Regenerate it with
`npm run update-readme-card` after refreshing the real control crops. The site
build uses the same artwork inputs and renderer; the regression test checks
that the snapshot matches. The surrounding README link opens the official site.

The homepage's “Built with ScarletUI” links were checked against the public
[Kureha Code README](https://github.com/petitstrawberry/kureha-code#readme) and
[Resonara README](https://github.com/petitstrawberry/resonara#readme) on 2026-11-05.
Both explicitly identify ScarletUI as their UI toolkit. Descriptions reflect
those READMEs; they do not imply production readiness or verified platform coverage.
No application screenshots are included: their public source trees did not
contain suitable screen captures. Add a visual showcase only with actual,
verified captures and their provenance, not illustrations or app icons.
