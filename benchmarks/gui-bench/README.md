# ScarletUI adapter for gui-bench

This adapter runs the existing [zortax/gui-bench](https://github.com/zortax/gui-bench)
scenario generator, process sampler, versioned JSON-lines protocol and offline
HTML report. It does not implement a separate benchmark harness. The upstream
revision is pinned to `221ba14287ac99110a13cccf5b8aa38e2881fff7`.

`lumen-fx/guibench` is another useful suite for broad application comparisons
(hello/list/forms/textview across nine toolkits). gui-bench was selected here
because its neutral node model already covers sparse updates, tree topology,
style complexity, stimulus latency and resizing through one adapter protocol.

## Run

From the ScarletUI repository:

```sh
python3 benchmarks/gui-bench/prepare.py /tmp/scarletui-gui-bench
cd /tmp/scarletui-gui-bench
env -u CARGO_TARGET_DIR cargo run -p gui-bench -- run \
  --framework scarletui --preset smoke --repetitions 1
```

The script creates a **new** checkout; an existing destination is refused.
It applies `upstream.patch`, copies this adapter, and points its dependency at
the current ScarletUI checkout. Subsequent source changes require preparing
another checkout or copying `src/` again. The generated checkout is disposable;
preserve `results/latest.json`, `results/latest.html` and the generated
`Cargo.lock` before deleting it.

Use the normal ScarletUI native build prerequisites (Rust edition 2024, a native
graphics stack; on Linux also fontconfig development files and pkg-config).
Upstream specifies nightly-2026-04-16; the macOS comparison was built with Rust 1.97.1. Use the same compiler/profile/display/GPU
settings for all compared frameworks. Keep benchmark windows visible and
unobscured. stdout is reserved for the adapter protocol; disable optional
ScarletUI/SGFX debug logging in the environment.

On a multi-monitor desktop, set `SCARLET_UI_BENCH_POSITION=x,y` to request a
position in logical screen coordinates on the intended display, for example
`SCARLET_UI_BENCH_POSITION=80,80` for the MacBook's built-in display when its
origin is `(0,0)`. Check the recorded `output_scale_milli` for every frame;
window-manager rules can move a window only after its first frame. The position
hint alone is not proof of a matched display or scale.

For comparisons, prepare with `--all-frameworks` and provide upstream's `../zgui`
checkout, then select each framework with repeated `--framework` arguments.
The supported zgui revision is `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a`.
GPUI-CE is already vendored by upstream. Separate adapter build invocations are
preserved. The default preparation excludes other adapters from Cargo resolution
so ScarletUI can be smoke-tested without unrelated framework dependencies.
Upstream's `preflight` still checks its original zgui/GPUI prerequisites; it is
intended for the full comparison checkout.

```sh
# A correctness smoke test with both measurement modes:
env -u CARGO_TARGET_DIR cargo run -p gui-bench -- run \
  --framework scarletui --preset smoke --diagnostic --repetitions 1

# Longer measurements after checking the smoke report:
env -u CARGO_TARGET_DIR cargo run -p gui-bench -- run \
  --framework scarletui --preset standard --repetitions 3

# Re-render an existing result without measuring again:
cargo run -p gui-bench -- report results/latest.json
```

Run the adapter's rendering/reconciliation regressions in the prepared checkout:

```sh
cargo test -p bench-core -p bench-scarletui -- --test-threads=1
python3 /path/to/scarlet-ui/benchmarks/gui-bench/verify.py results/latest.json
```

The verifier requires all ScarletUI conditions to complete with valid frames,
exact phase counts, increasing timestamps and observed resize changes. It fails
if a window manager overrides resize requests; use a normal floating window or
a dedicated desktop session, and keep that failed run as evidence of the
environment constraint. Running a tiling window manager can affect all observed
sizes, including startup. The verifier does not establish statistical stability.

## Measurement contract

- One startup record, exactly `warmup_frames` warmup records and
  `sample_frames` measurement records. The handshake precedes window creation.
- Native winit + SGFX only. CPU fallback is an error. The shared IBM Plex Sans
  font is installed before creating the tree; labels use 12 logical pixels.
- Each logical node owns a `State<u64>` subscription. Deterministic
  `bench_core::affected_indices` selects the nodes to update; selected local
  counters increment, matching zgui's retained reactive update semantics.
- Flat trees use a small native wrapping layout (4 px gaps); wide, balanced
  (8-way), and deep trees use native vertical stacks. Nodes are not virtualized.
  Structural updates alternately remove/restore the final 10% using stable keys.
- Frame intervals end when SGFX accepts a submission through
  `RenderingPipeline::render_for_present`. They include queue/presentation
  backpressure, event handling and adapter output overhead. They are **not**
  display scanout, device execution time or physical input latency.
- Resize requests synchronize with native configure events and the actual
  client extent after pumping events. Observed sizes never come from cached
  requested dimensions; the latter is important for manually pumped AppKit loops.
  Ungranted resizes and unsubmitted frames are invalid and carry a note.
- Passive cases request retained redraws without mutating application state.
  A coalesced update also requests retained redraw before recording a submission.
- Diagnostic mode currently has the same probes as headline mode. All internal
  pipeline stages, device GPU timestamps, and framework-owned memory counters
  remain unavailable (`null` / `false`), rather than invented zero values.
- The runner records gui-bench and ScarletUI source revisions/dirty state and
  warns about unavailable counters. Resource comparisons need Linux `/proc` and
  DRM/NVML. macOS measures native frame cadence but has no comparable PSS/VRAM.

## Matched comparison

The integration patch also repairs the existing adapters so they receive the
same workload. All three increment selected per-node counters; GPUI uses 12 px
IBM Plex Sans, 16 px line height, 1 px vertical padding, and the same 38 px
layout-update minimum. zgui's `Wide` structural case now actually removes and
restores its trailing children. The zgui renderer setup follows the pinned
revision's current surface API.

All observed dimensions are logical pixels. `output_scale_milli` additionally
records the actual display scale, including Retina. The comparison validator
checks dimensions, scales, complete measurement frames and all requested
repetitions. It withholds headline percentiles when scales differ. The runner executes matched conditions together and
rotates framework order by scenario index and repetition.
Matched here means the same logical nodes, mutations, font, window and scale;
layout rounding and intrinsic-sizing strategies remain native to each toolkit.
The comparison does not claim pixel-identical images or isolated layout cost.

```sh
# Prepare zgui next to the new gui-bench checkout at the supported revision.
mkdir -p /tmp/gui-comparison/zgui
git -C /tmp/gui-comparison/zgui init
git -C /tmp/gui-comparison/zgui remote add origin https://github.com/zortax/zgui
git -C /tmp/gui-comparison/zgui fetch --depth 1 origin 80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a
git -C /tmp/gui-comparison/zgui checkout --detach FETCH_HEAD
python3 benchmarks/gui-bench/prepare.py /tmp/gui-comparison/gui-bench --all-frameworks
cd /tmp/gui-comparison/gui-bench
env -u CARGO_TARGET_DIR cargo run -p gui-bench -- run \
  --framework zgui --framework gpui --framework scarletui \
  --preset standard --scenario n64 --repetitions 3
python3 /path/to/scarlet-ui/benchmarks/gui-bench/compare.py \
  results/latest.json --output results/comparison.json
```

`--scenario` can be repeated; filters are ORed. `compare.py` produces JSON and
CSV with the median of the per-repetition p50/p95 values and the min/max of
repetition medians. That range is observed spread, not a confidence interval.
If any repetition fails, that framework/scenario has no headline percentile;
the failed statuses remain in the JSON and CSV instead of becoming zero or a
percentile calculated only from the surviving repetitions.
Raw frames, startup and warmup records remain in the upstream JSON/HTML report.

The prepared `bench-run` profile retains release optimization, thin LTO and one
codegen unit, with debug symbols disabled for **all** adapters to limit build
disk usage. GPUI's native macOS backend uses its `runtime_shaders` feature,
so no separate Metal compiler download is required. Its vendored manifest also
enables the AppKit types actually used by the native window implementation.
Runtime shader initialization affects startup; it is outside measurement-phase
percentiles after warmup.

## Interpreting results

- Frame intervals and stimulus latency include each toolkit's native event loop,
  redraw policy, presentation pacing and CPU submission boundary. They do not
  isolate renderer work or prove which toolkit reaches display scanout first.
- Decorated shadows still use native presets (ScarletUI `Raised`, GPUI
  `shadow_sm`, zgui's CSS shadow). Keep these rows as native-style
  characterization, separate from the matched minimal workload.
- This adapter drives public pipeline/platform APIs, like ScarletUI's native
  scroll benchmark; it does not include `ApplicationRunner` scene synchronization.
- Cold-start records cover post-handshake initialization; the upstream report's
  headline percentiles use measurement-phase frames, not the startup record.
- Linux `/proc`/DRM counters are unavailable on macOS. Framework-owned GPU
  allocation reports have different coverage and cannot substitute for a common
  process-memory measurement.
- Under yabai, allow only the benchmark windows to float for the measurement,
  and remove that temporary rule afterward. A rule matching
  `^(gui-bench: ScarletUI|zgui benchmark|GPUI benchmark)$` avoids unrelated windows.

The first macOS smoke validation (2026-11-10) found a 64-node deep-layout timeout
and unobserved resize requests. The layout regression now counts leaf layout
calls to prevent overflowing intrinsic stacks from being mistaken for flexible
children. Scroll viewports explicitly report their fill behavior; fixed-size
siblings and Spacer allocation remain covered by regression tests. The original
failed run remains preserved separately from the new comparison measurements.

The ScarletUI dense-update optimization batches clipped paint-bound lookup and
nearest-boundary invalidation into tree traversals. It also replaces excessive
pairwise damage merging with one conservative bounding region. Retained picture
caches stay in use. The regression test compares an alpha-blended update across
many repaint boundaries with a freshly rendered reference frame.

Further dense-update work batches component rebuilds and legacy paint-buffer
refresh into live-tree traversals. Large layout invalidation sets use one
conservative root layout; retained paint recordings index dirty membership once
per synchronization. SGFX retained meshes submit new vertex buffers together,
preserving pending bytes until the whole upload batch succeeds so failed batches
can be retried before drawing. Tests cover removed dirty children, overlapping
paint roots, self-only paint scope, and partially accepted upload batches.

The macOS measurement loop pumps native events once before each stimulus.
A no-op/offscreen update that returns `Idle` requests a retained redraw and
retries immediately, because that redraw is already runnable. Suspended windows
and further unsuccessful retries still wait and process native events. Each
sample still requires a real SGFX submission, and resize requests still wait for
an observed OS configure. The previous double pump and unconditional retry wait
inflated light workloads; compare library revisions with this same driver to
separate the driver correction from framework improvements.

The v5 comparison preserves both a v4-library control and the improved library,
each measured across the same 18 scenarios with three repetitions using the
corrected driver. Historical v4 results and unchanged zgui/GPUI measurements are
labelled separately in the report. Surface color changes retain child paint
recordings; dense self-only damage uses one traversal; stable sibling lists
reconcile in place. Pixel, clipping, identity, and replacement tests cover these
paths.

The v6 run rebases the comparison on main `f88d11a6`, measuring both the current
baseline and the improved implementation. Earlier v5 results use a different
source revision and remain historical context. The v6 renderer reuses local
rectangular text clips only at integral physical translations and falls back
for fractional translation or clips truncated at the local target. GPU readback
compares clipped retained text with flat rendering at 1x, 1.25x, 2x and 3x,
including negative origins. Large mesh caches admit new recordings after reuse
across frames, retaining a bounded 4096-candidate history for each of two frames.
Opt-in Surface paint outside the window can be deferred on self-only updates;
geometry/composition changes flush pending paint before replay. Retained groups,
backdrop-dependent scenes and paint extensions keep eager recording.

Text render objects retain their intrinsic measurements across parent constraint
changes, invalidating on content, font size or font-stack revision changes. A
1024-label regression test exercises shared-cache eviction, tightened/relaxed
constraints and all invalidation inputs. The final v6 comparison includes this
layout fix; the earlier v6 measurement is retained under `pre-layout/`.

The v7 text-update optimization shares lazily parsed Rustybuzz faces with each
immutable outline font, using `self_cell` to keep owned font bytes alive without
leaks or lifetime casts. Font-stack clones share the face, including TTC indices.
The shaped-layout cache uses indexed lookup and FIFO eviction with incremental
byte accounting, retaining its 512-entry / 4-MiB limits. Font selection borrows
faces instead of cloning reference-counted owners for each scalar. Tests cover
fresh-parser equivalence, concurrent initialization and release, and cache key
isolation and both capacity limits. The report compares newly measured v7 with
the preserved v6 run, keeping saved zgui/GPUI results clearly labelled.
