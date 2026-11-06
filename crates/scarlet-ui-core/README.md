# scarlet-ui-core

`scarlet-ui-core` contains the ScarletUI implementation shared by all platforms.
It owns views, elements, scenes, rendering pipelines, state, preview core, and
the platform traits.

This crate intentionally does not depend on SWS or desktop window libraries.
Backends implement these public traits instead:

```text
PlatformBackend
  -> create_window(WindowCreateRequest)
  -> Box<dyn PlatformWindow>

PlatformWindow
  -> poll_event()
  -> present_with_damage()
  -> sync_text_input()
  -> window controls
```

Normal applications should depend on `scarlet-ui`, not this crate directly.
`scarlet-ui` re-exports the app-facing API and wires the selected backend.

See the workspace [ScarletUI Architecture](../../docs/ARCHITECTURE.md) for the
View, Element, RenderObject, reconciliation, and state-lifetime contracts.

Useful checks:

```bash
cargo check
cargo check --no-default-features --features legacy-scarlet-std
cargo test
```


## In-app glass quality

Built-in glass surfaces follow `set_glass_quality(GlassQuality::Refractive)`
(exported from the prelude). The application-wide setting has three levels:

- `TintOnly`: translucent color with at least 88% opacity; no backdrop copy or blur.
- `Frosted`: the material's blur and tint.
- `Refractive` (default): reduced blur plus bounded convex-edge displacement and a thin
  directional reflection. The flat center and foreground text stay undistorted.
  Blur uses 14% of the material radius at source resolution. Large panels do
  not impose a stronger blur or reduce the background to a 256px image.

Request a scene redraw/rebuild after changing this setting so retained paint
commands are refreshed. The Widget Factory's Controls page demonstrates this
with a state-driven quality selector. `GlassMaterial::new` still controls blur
and tint; menu surfaces use a shorter kernel and lighter wash than editing faces.

This samples already-painted **application content**, not the OS desktop.
Refraction is a screen-space approximation of a convex bezel (Snell ratio 1/1.5),
not Apple's implementation, full ray tracing, chromatic dispersion, or animated
fluid merging. Uniform backgrounds naturally show little distortion.

CPU and SGFX use the same bounded displacement profile. Native Winit enables
SGFX WGSL pipelines: two full-resolution separable blur passes and per-pixel
rounded-rectangle normals, refraction and reflection. This avoids the seams
of a coarse displacement grid. One window-sized scratch image is reused with
the snapshot; each blur tap is sampled in the shader rather than drawn as a
separate blended quad. Fixed-pipeline backends retain the mesh implementation.
Local shadow, rounded-fill, refraction, and rim meshes are
reused across frames and translations (LRU: 64 shapes / 65,536 vertices, at most
2 MiB of vertex payload). Cached optical meshes include their own rounded
clipping; ancestor clips still apply. Glass no longer bypasses the retained
GPU mesh and shadow-coverage caches for the rest of the scene.
Backdrop copies, blur passes and foreground draws share an ordered command stream
(split only at IR capacity), avoiding a queue submission per material. Fully
clipped materials are rejected before optical tessellation and scratch allocation.
On the fixed-pipeline path, after 16 backdrop filters or near the frame vertex
limit, optics degrade to ordinary blur. Existing blur budgets still apply (64 filters,
8 Mpixels copied per frame, bounded working textures); exhaustion falls back to
opaque tint. These are resource limits, not a frame-rate guarantee.

Scroll content can retain full-resolution GPU raster tiles (512px, at most 64
slots / 64 MiB, reused with bounded eviction). Warm scrolling composites those
tiles; newly exposed or changed recordings are rasterized on the GPU, without
CPU image uploads. Immutable recordings also cache backdrop/extension metadata.
Glass can share the cache when earlier opaque paint in the recording covers
its entire sampling footprint. External backdrop reads, including filters at
scrollport edges, remain live. Halo pixels protect tile boundaries. Fractional
translations/DPI use the normal precise path to preserve existing text snapping.
Rejected/discarded frames invalidate uncommitted tiles. Cache retention is an
optimization hint; it never substitutes a reduced-resolution image.

Floating bottom Navigation paints after content and overlaps vertical
ScrollViews. A separate container inset lets the last item scroll above the
bar, adds to authored `content_bottom_inset`, and clears on return to sidebar
layout. Static pages retain safe space. Glass TabView remains a separate
component: a compact, centered, label-width bar (upper edge at regular widths,
lower edge at compact widths), without a second glass layer on its selection.

Technique and design references:

- [Apple: Meet Liquid Glass](https://developer.apple.com/videos/play/wwdc2025/219/)
- [Apple: iPad tabs and sidebars](https://developer.apple.com/videos/play/wwdc2024/10147/)
- [Kube: Liquid Glass in CSS/SVG](https://kube.io/blog/liquid-glass-css-svg/)
- [liquid-glass-react displacement field](https://github.com/rdev/liquid-glass-react/blob/ac48eab18d1f7f444ae30002d240cae29c863a21/src/shader-utils.ts)
- [liquid-glass-studio WGSL optics](https://github.com/iyinchao/liquid-glass-studio/blob/f7b28c36305a862f5cffed3ddd51511cf1204f56/src/shaders-wgsl/fragment-main.wgsl)

The mathematical approximation here is independently implemented; those
projects were reviewed as algorithm references.

A reproducible GPU diagnostic is available with:

```sh
SCARLET_BENCH_GLASS_COUNT=12 cargo test --release --locked -p scarlet-ui-renderer-sgfx --test backdrop_gpu warm_glass_scene_costs -- --ignored --nocapture
cargo test --release --locked -p scarlet-ui-renderer-sgfx --test backdrop_gpu warm_scroll_layer_costs -- --ignored --nocapture
```

It compares flattened and retained warm frames for tint, frosted and refractive
modes (600×400, 24 moving objects, one shadow and 1–16 glass panels; default 1).
The test backend revision matches the native Winit backend. It reports CPU
encoding time separately from frame time including synchronous GPU readback;
neither number is an application FPS measurement. Run it without other GPU tests
or builds for a useful comparison.
The second diagnostic compares direct rendering with GPU raster retention for
1,000 rows. Pixel tests separately cover scrolling, local glass, exposed tiles,
rounded clips, content changes, rejected submissions and 1x/2x rendering.
