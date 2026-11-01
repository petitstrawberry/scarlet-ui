# Widget Factory on WebGPU (experimental)

The native and browser entry points share `src/factory.rs`: the original
Overview, Controls, Inputs, Display and SGFX pages, layouts, widget states,
icons, text editors, split/tab controls and animated 3D cube. This is not a DOM
reimplementation or a reduced showcase. The same ScarletUI/SGFX encoder draws
into WebGPU; browser event translation lives in `scarlet-ui-platform-web`.

## Build / run

SGFX dependencies use pinned Git revisions; no sibling checkout is required.
With Rust edition-2024 support:

```sh
rustup target add wasm32-unknown-unknown
cargo build --locked -p scarlet-ui-widget-factory --lib --target wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
wasm-bindgen target/wasm32-unknown-unknown/debug/scarlet_ui_widget_factory.wasm \
  --target web --out-dir examples/widget-factory/pkg
python3 -m http.server 8765 --bind 127.0.0.1 --directory examples/widget-factory
```

Open localhost:8765 in a browser with an available WebGPU adapter. The embedded
font is shared with the counter; see its `assets/LICENSE`. Native usage is still
`cargo run -p scarlet-ui-widget-factory`.

## Browser input / limitations

- Pointer capture, keyboard key presses/releases, ordinary Unicode characters,
  navigation/editing keys, modifiers, wheel scrolling and canvas focus.
- Finger contacts enter the shared touch gesture arena: taps activate controls,
  drags scroll content after the movement threshold, and horizontal Slider drags
  keep their own axis. Cancellation/blur ends active contacts without a click.
  Mouse and pen input keep their existing pointer behavior.
- Wheel pixel/line/page units are converted to logical CSS pixels; DPR affects
  backing resolution, not logical input coordinates. Ctrl+wheel/browser zoom
  shortcuts remain browser-owned.
- Existing SGFX Canvas renders the animated cube with depth testing; no canvas
  or widget is replaced by a browser DOM control.
- IME composition, dead-key composition, clipboard integration, accessibility
  tree, native dialogs/popups and multiple windows are not implemented. The
  existing in-canvas Select widget is retained; native OS popup APIs are distinct.
- Hardware GPU/browser-engine coverage and device-loss recovery require separate
  validation. The native design is retained, but full native platform parity is
  not claimed.

## Manual checks

Visit every page. Toggle and drag Slider; edit TextField with ordinary text,
select-all/backspace and arrows; edit multiline TextView; change Select values;
scroll outer and nested areas; drag SplitView divider and change tab placement;
confirm the SGFX cube rotates; resize/zoom and repeat hit-testing. Check startup
and `runtime_error()` reporting, stop/start and repeated startup. A successful
build is not evidence that these browser checks passed.

## Shared menu model

`WidgetFactory::menu_model()` supplies `Window::menu_bar()` on both targets.
Web uses the existing ScarletUI MenuBar/MenuItem/Menu rendering components at
the top of the canvas, with dropdowns/submenus and a right-click popup using the
same model. It reserves layout space rather than covering the application.
Actions use the existing menu callback registry; enabled and checked state are
refreshed from the scene model after rendering. The demo provides reset,
disabled, checked-toggle, nested slider-preset and text-setting commands.

Arrow/Enter/Space navigation and Escape/outside-click dismissal are supported.
Shortcut strings are preserved as **displayed labels**, not registered as Web
application accelerators. Browser shortcuts are not intercepted: the demo's
`Ctrl+L` must still focus the browser address bar, while clicking its "Set sample
text" entry invokes the application action. Platform-native menu integration
is unchanged; this work does **not** add macOS NSMenu support.

Menu QA: open each root; ensure disabled rows cannot activate; toggle the check
state and reopen; invoke reset and nested presets; dismiss with outside click
and Escape; right-click near canvas edges and confirm clamping/hit-testing;
verify Ctrl+L belongs to the browser; repeat after resize/125% zoom.

Current menu limits: opening is click/tap/right-click (no Alt/F10, typeahead or touch
long-press); context popup uses the window's complete menu model, not a per-widget
provider. Dropdown width follows shared label/shortcut/chevron measurements and is
clamped to the canvas; oversized menus do not yet scroll. The included demo menus fit. Accessibility and
native macOS menu integration remain separate work.


The browser surface is edge-to-edge without a decorative focus outline. Keyboard
focus remains within the ScarletUI controls (text caret/selection, etc.). Menus
use shared theme rendering with a compact 28-pixel bar, measured popup widths,
aligned check/label/shortcut/chevron columns, and edge-aware submenu gaps. This
changes the menu presentation only, not the Factory's internal page design.
