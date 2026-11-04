# Shared glass material trial

`GlassMaterial` expresses visual intent independently of the renderer:
- `regular()`: button material, 18 logical-pixel support and 0.64 tint opacity.
- `editing()`: the same family with a denser 0.78 neutral button-surface tint
  for editable text; it stays distinguishable on a plain white canvas.
- `thumb()`: 6-pixel support and 0.72 tint for small interactive faces.
- `floating()`: 18-pixel support and 0.70 tint for menus and popovers.

The tint uses the semantic surface color. These are prototype design tokens,
not separate assets or per-widget highlight effects. `new(radius, tint)` permits
explicit customization. `focused_editor()` strengthens blur/opacity and applies
small neutral contrast based on the material luminance. Editing focus does not
mix in the primary accent or add a decorative line. CPU pixel regressions cover
visible neutral differences on light and dark blank backgrounds. Geometry, input, semantic state color and foreground
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
