# ScarletUI WebGPU counter (experimental)

This is the Rust ScarletUI `Text`, `Button`, `State` and scene/layout pipeline,
not a DOM recreation. `SgfxPaintEncoder` records its drawing into SGFX, and
`sgfx-backend-wgpu` executes it on a WebGPU canvas. There is no Canvas2D or
WebGL fallback. The font is embedded; its license is in `assets/`.

## Build and run

Use a Rust toolchain supporting edition 2024. SGFX dependencies are pinned to
published Git revisions; no sibling checkout or local Cargo patch is required.

From the `scarlet-ui` root:

```sh
rustup target add wasm32-unknown-unknown
cargo build -p scarlet-ui-web-counter --target wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
wasm-bindgen target/wasm32-unknown-unknown/debug/scarlet_ui_web_counter.wasm \
  --target web --out-dir examples/web-counter/pkg
python3 -m http.server 8765 --directory examples/web-counter
```

The CLI version must match `wasm-bindgen` in `Cargo.lock`. If using a custom
`CARGO_TARGET_DIR`, adjust the input artifact path. Open `http://localhost:8765`
in a browser with an available WebGPU adapter. Non-local origins require HTTPS.
`navigator.gpu` alone is not enough: `requestAdapter()` may return `null`.
An unsupported adapter is reported visibly instead of silently substituting a
CPU renderer. JavaScript first awaits WASM initialization, then explicitly
awaits the exported asynchronous `start()` so adapter errors are catchable.

## Integration and lifetime

Enable the facade's `platform-web` feature on the wasm target, install a font
using `graphics::set_default_font`, and await `scarlet_ui::run_web(canvas, app)`.
Keep the returned `WebApp` alive. It owns the runner, GPU resources, input
listeners and cancellable animation-frame handle. Dropping it stops rendering
and removes listeners even when the tab is backgrounded. `is_running()` and
`error()` expose terminal status. There is no self-retaining RAF cycle.

The canvas must have an explicit CSS size, no border/padding, and
`touch-action: none`. Its backing size is CSS pixels multiplied by device pixel
ratio (rounded to ScarletUI's milli-scale precision). Size and DPR changes are checked each frame; pointer coordinates stay
in CSS pixels. Pointer capture supports release outside the canvas, and capture
loss/cancellation cancels the pressed control. CSS rotation/skew is unsupported.

## Browser verification checklist

These are checks to run, not claims of a successful GPU run:

1. Confirm actual text and the Increment button appear in the canvas, with no
   initialization, GPU validation or WASM errors in the console.
2. Click Increment repeatedly and verify the displayed count advances exactly
   once per click. Press inside and release outside: it must not activate.
3. Cancel a pointer gesture or lose capture, then click again: no stuck press.
4. Resize narrower/wider and change browser zoom or move between displays with
   different DPR. Confirm the canvas backing width/height track
   `ceil(clientWidth * round(devicePixelRatio * 1000) / 1000)` / corresponding height, and text and
   pointer hit testing remain aligned.
5. Background/restore the tab and use Back/Forward; rendering should resume
   without a second runner or doubled clicks.
6. In the console, import `./pkg/scarlet_ui_web_counter.js`, call `stop()`, then
   `await start()` repeatedly. Verify only one increment per click afterward.
7. In a browser without a WebGPU adapter, verify the visible startup error and
   absence of a false successful-rendering state.

Keyboard navigation, basic Unicode character input and wheel scrolling are
translated by the web platform in addition to pointer input.
Text editors use an invisible textarea bridge for browser editing, selection
and composition while remaining SGFX-rendered. See Widget Factory’s browser
input notes for real-device verification limits.
Limitations: single canvas/window; no accessibility tree, native popups, dialogs
or multi-window support yet.
WebGPU device loss or allocation/render failures terminate the runner and are
reported; automatic device recovery is not implemented.
