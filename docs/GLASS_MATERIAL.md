# Shared glass material trial

## Default chrome

NavigationView (both constructors), HeaderBar and TabView default to Bubble
Glass. Sidebar and bottom navigation use the existing Floating geometry; tabs
use the existing content-sized material bar. HeaderBar paints glass within its
existing bounds and adds no padding or safe-area reservation. Its component
outline defaults to 16px, independently configurable with `.corner_radius`.
Navigation content underlap remains an explicit layout choice.

Use `.chrome_style(ChromeStyle::Simple)` on any of these components for the
previous solid treatment, or `ChromeStyle::BubbleGlass` to restore the default.
Presets reset appearance colors; apply color/material overrides afterward.
`NavigationSidebarStyle::Simple` and `Floating` remain supported.
`HeaderBar::surface(color)` explicitly selects a solid fill, including CLEAR
when the bar is already wrapped in a styled Surface, preventing double glass.
Low-level render-object constructors with explicit colors retain their prior
semantics. Surface itself still defaults to flat; ordinary content backgrounds
are not made translucent.

## Bubble Glass composition

`SurfaceStyle::bubble_glass()` packages the shared glass appearance, including
its directional rim response. It imposes neither a corner radius nor a cast
shadow. Shape, clipping and layout remain separate choices:

```rust
use scarlet_ui::prelude::*;

let material = GlassMaterial::regular().opacity(0.55).blur_radius(12.0);
let appearance = SurfaceStyle::bubble_glass().material(material);
let toolbar = Text::new("Library")
    .padding(12.0)
    .surface_style(appearance)
    .corner_radius(16.0); // the application chooses the shape

let glass_only = Text::new("Glass").glass(material).corner_radius(6.0);
let shadow_only = Text::new("Shadow")
    .surface_shadow(ElevationRole::Raised).corner_radius(6.0);
let shaped = Surface::section(Text::new("Section"))
    .corner_radius(4.0).surface_style(appearance);
```

These modifiers currently paint rectangular or rounded-rectangular bounds;
they do not infer the silhouette of arbitrary child content. Applying a style
to an existing `Surface` preserves its radius, clipping and tonal role.
`.material(...)`, `.elevation(...)` and `.corner_radius(...)` remain available
individually. Shadows use paint outsets and do not enlarge the layout/hit bounds.
Styles add no padding or safe-area reservation.

`GlassMaterial::tint(color)` replaces the tint including its alpha;
`.opacity(alpha)` sets only that tint's opacity, leaving foreground and optical
rim intact. `.blur_radius(radius)` controls diffusion independently. Refractive
quality uses 14% of this radius; Frosted uses the full radius. TintOnly keeps
its 0.88 opacity floor for readability. These controls affect in-app backdrop
sampling, not operating-system window transparency.

## Navigation and shoulder tuning

Floating navigation and material-backed content tabs use a lighter neutral
structural wash (0.018 additional text tint) and a 0.075 selection overlay.
Navigation hover raises the selected overlay to 0.11. Backdrop sampling,
refraction, blur and existing elevation shadows retain their geometry.

Refractive light comes from a low-amplitude curved-shoulder response, strongest
on the upper/lower shoulders. A compact dark contour is firmer and slightly
wider at the sides, becoming finer at the top/bottom. This follows the supplied
iOS reference's directional contrast; the screenshot alone cannot identify
whether its dark edge comes from a stroke, refraction, or contact shading.
White highlights are clipped against the white reference background, so their
full strength cannot be recovered from that screenshot. This is a visual
approximation, without a bright all-around outline or painted face gradient.
CPU, portable SGFX and analytic WGSL use the same signed shading/reflection
model. The hidden
`glass_highlight_edge` helper remains available for source compatibility but is
no longer used by built-in rendering.

Validation: `cargo test -p scarlet-ui-core --lib` and
`SCARLET_REQUIRE_GPU_TESTS=1 SCARLET_TAB_PARITY_DIR=/tmp/glass-study cargo test
-p scarlet-ui-renderer-sgfx --test tab_material_parity -- --nocapture`.
The latter exports diagnostic PPMs for portrait/landscape navigation over plain
and patterned backgrounds, and light/dark shoulder specimens at 1x, 1.5x and 2x.
Both SGFX paths run on the actual Metal adapter. These are offscreen render
commands, not native-window or browser screenshots. Native Widget Factory built
and launched on macOS, but interactive visual QA was blocked by the locked Mac.

## Material presets

`GlassMaterial` expresses visual intent independently of the renderer:
- `regular()`: general material, 18 logical-pixel support and 0.64 tint opacity.
- `button()`: a neutral, more visible 0.78-tint face with the same backdrop blur.
  It separates resting buttons from a plain white canvas without adding a bevel.
- `editing()`: the same family with a denser 0.78 neutral button-surface tint
  for editable text; it stays distinguishable on a plain white canvas.
- `thumb()`: 6-pixel support and 0.72 tint for small interactive faces.
- `floating()`: 18-pixel support and 0.70 tint for menus and popovers.

Enabled glass buttons also have a soft ambient shadow and a compact contact
shadow, so their silhouette remains visible even with a white face on white.
Hover strengthens the ambient shadow; pressing lowers both shadows. Disabled
and header buttons do not gain these shadows, and explicit-color buttons keep
their original treatment. This changes neither the layout nor the hit area.

The tint uses the semantic surface color. These are prototype design tokens,
not separate assets or per-widget highlight effects. `new(radius, tint)` permits
explicit customization. Focus uses a three-logical-pixel rounded scarlet ring,
following the control contour while leaving the glass tint and blur unchanged.
It does not darken the face, wash it red, or add an underline. The ring stays
inside the existing bounds, so focus does not change layout or hit targets.
`focused_editor()` is retained as a no-op for source compatibility. CPU pixel
regressions verify unchanged center pixels and at least 3:1 ring contrast on
plain light and explicitly supplied dark material surfaces. Geometry, input,
semantic state color and foreground
content are independent of material. Default editable TextView/TextField surfaces
use editing material; explicit caller colors use uniform custom faces. Structural
header/titlebar backgrounds are uniform rather than faux-glass gradients. Main
window bodies, text, tracks and semantic separators are not blurred merely for
consistency. No refraction or desktop sampling is claimed.

## Scene semantics

The first-class `BackdropFilterRoundedRect` command samples only the preceding
composed app scene, blurs it, applies uniform tint, and replaces the rounded
region with antialiased clip coverage. Subsequent text and icons remain sharp.
Nested clip layers are resolved at the material boundary. A later overlapping
material can therefore sample an earlier material's output.

A custom `ElementRenderObject` emitting this command, including through a display
list or popup, must return true from `reads_backdrop()`. This pure query is needed
before routing the frame: probing by calling `paint()` twice is unsafe because
paint can update damage history. Built-in material controls provide the query.

Backdrop-dependent scenes bypass isolated raster picture caches. Changed frames,
including composite-only scrolling, replay the full scene from a clean target.
Idle scenes do not redraw perpetually. Ordinary cache routing returns when no
object reads a backdrop. This is conservative correctness, not the final damage
optimization. A future dependency graph must propagate dirty source regions to
later filters in paint order; previous final-frame pixels are not valid sources.

## Backends and safeguards

CPU uses premultiplied three-pass separable box blur. Support is capped at 64
physical pixels; each ROI is capped at one million pixels and the frame at eight
million sampled pixels. Retained blur scratch is bounded to about 32 MB. This is
full-resolution blur. Work exceeding these bounds uses opaque semantic tint for
readability; explicit zero-radius material remains translucent. Both renderers
expose a per-frame `backdrop_fallback_count()` diagnostic.

SGFX copies the expanded ROI at the command position, downsamples, performs
separable weighted texture taps, and composites the rounded result. It uses
portable fixed texture rendering, not the programmable path unsupported by the
pinned SGFX web backend. Texture/work budgets, 64 filters per frame and a
reconstruction-quality guard produce a solid semantic-tint fallback instead of
silently broadening a tiny blur on a huge panel. `backdrop_fallback_count()`
exposes that event. Backend resource failures still propagate normally.

A standalone legacy popup window cannot see the parent app's pixels. Its Menu
buffer remains opaque and readable rather than claiming cross-window blur.

## Performance and quality policy

Short virtual rows without backdrop reads share their enclosing scroll layer's
GPU tiles. Allocating separate 512-pixel textures for every short row wastes
space and can evict the parent viewport's working set on Retina displays.
Nested rows at most 256 physical pixels tall replay into parent tiles;
independent tall content and backdrop materials keep their own caches. Pixel
parity and bounded-allocation regressions cover this routing.

Release CPU benchmark medians on this shared cloud machine (1x / 2x): baseline
0.58 / 2.50 ms; one material 1.18 / 4.05 ms; three 1.54 / 14.82 ms; eighty
29.34 / 143.79 ms. These are one synthetic scene and noisy shared-machine
observations, not 60fps guarantees. Reproduce with the `backdrop-bench` example.
Real SwiftShader Vulkan readback verifies SGFX output, but software Vulkan timing
must not be presented as physical GPU performance.

The full Widget Factory Controls scene was also replayed offscreen through the
actual release CPU pipeline, with background updates and the sidebar included:
1180x812 logical pixels, 20 changed frames, median 23.7 ms at 1x and 101.0 ms at 2x.
Shared-machine p95 was highly variable, so these values are observations rather
than a stable performance contract. This full-resolution, full-replay 2x path is
not ready for smooth 60fps interaction. Idle frames correctly stop redrawing.
Run `cargo test --release -p scarlet-ui-widget-factory
measure_factory_glass_frame_budget -- --ignored --nocapture --test-threads=1`
to reproduce. This is pipeline timing, not a native-window screenshot or GPU
benchmark.


Material semantics should remain stable as implementation quality changes.
The current guards are fixed budgets, not an adaptive quality system. A future
policy can select full blur, bounded lower-resolution blur or opaque fallback
using measured frame time, memory/capabilities and explicit reduced-transparency
preferences. Use hysteresis and restore quality after sustained recovery; do not
permanently downgrade all CPU devices or infer capability from a device label.
Keep tint/contrast and interaction states legible at every quality level.

## Evidence and remaining work

CPU focused tests cover 1x/2x sampling, clips, sharp foreground, overlapping
materials, premultiplied alpha, clean damaged replay and bounded warm allocations.
SGFX has real Vulkan pixel readback at 1x/2x for the same core behaviors. Native
CPU Widget Factory was inspected with changing background colors and fine lines,
and an overlapping popup; dismissal left no stale pixels.

Full-control rollout, real hardware/web execution, dark themes, reduced
transparency and platform accessibility still need integrated review. The
material does not solve the existing keyboard traversal or font shaping issues.
