# ScarletUI Animation Lab

One native/WebGPU application for exploring animation, alongside Widget Factory.
Live: <https://ui.scarlet-os.org/animation-lab/>.

```sh
cargo run --release -p scarlet-ui-animation-lab
SCARLET_UI_ANIMATION_LAB_SMOKE=1 cargo run --release -p scarlet-ui-animation-lab
```

## Pages

- **Play**: ring and zigzag loaders, a squashing jelly friend, and star/heart
  particles. Load again, celebrate, change palette/speed, pause/resume, or reset.
- **Basics**: explicit and implicit size/color/offset/opacity transitions with
  interruption and immediate reset.
- **Layout**: composed cards, chart updates, layout changes, shuffle, and theme.

NavigationView shows a sidebar on wide windows and bottom navigation on narrow
ones, matching Widget Factory. The app keeps each page's model while switching pages. Only the selected page
has mounted render objects, so hidden looping drawings do not keep animating.
Layouts adapt to narrow widths and scroll vertically. The native smoke sequence
visits all pages and closes after about 23 seconds; it is not an FPS benchmark.

## Web build

```sh
rustup target add wasm32-unknown-unknown
cargo build --locked --release -p scarlet-ui-animation-lab --lib --target wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
wasm-bindgen target/wasm32-unknown-unknown/release/scarlet_ui_animation_lab.wasm \
  --target web --out-dir examples/animation-lab/pkg
python3 -m http.server 8765 --bind 127.0.0.1 --directory examples/animation-lab
```

Open localhost:8765 with an available WebGPU adapter. Native and browser entry
points share `src/lab.rs`, `play.rs`, `basics.rs`, `layout.rs`, and `paint.rs`.
The browser exports `start`, `stop`, `pause`, `resume`, and `runtime_error` and
pauses its frame loop while the page is hidden. The embedded font uses the
license in `../web-counter/assets/LICENSE`.

`python3 tools/build-web-samples.py` packages `/animation-lab/` with the existing
site, Widget Factory, Counter, API reference and guides. The existing Web samples
workflow validates the artifact and deploys it to the configured GitHub Pages
site after browser smoke passes. Browser GPU availability and touch behavior
still depend on the device; a build alone is not evidence of device coverage.

See [the animation guide](../../docs/ANIMATIONS.md) for the public API and limits.
