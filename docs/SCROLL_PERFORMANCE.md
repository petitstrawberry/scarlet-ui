# Scroll performance

## Rendering and virtualization

GPU paint backends receive visible commands directly. They do not first rasterize
repaint-boundary pictures on the CPU. CPU backends explicitly opt into retained
picture caching; sparse picture storage follows actual paint bounds rather than
virtual list height. Partial presentation damage includes everything cleared by
the backend, including coalesced gaps, without overlapping alpha composition.

BitmapImage shares immutable decoded source pixels. DrawBufferRect carries its
source crop and destination size, so GPU resampling replaces repeated CPU cover
scaling. The SGFX texture pool reuses compatible capacities, preserves resource
identities, and pads the current image edges for linear sampling.

ListView snapshots items when their state changes. LazyVStack and GridView merge
sorted visible ranges and lay out only entering items when scrolling. Retained
items are remeasured on resize or view updates. Components forward viewport
hints, nested scrollers own their hints, and new rows invalidate their owning
retained boundary. Content growth/shrink propagates through tight layouts and
clamps scroll offsets.

SGFX text lowering uses shared positioned glyph runs, bounded to 512 entries and
4 MiB of conservatively counted references. Unchanged text retains positions and
masks even when the smaller individual-glyph cache evicts them. Font identity,
text, exact size and output scale are cache keys. Oversized runs bypass retention.

## Gesture continuity

Winit's integer logical wheel events retain fractional remainders between native
pixel samples. Previously, every small HiDPI delta was rounded separately: 100
samples of 0.4 physical pixels at 2x scale could produce no motion instead of 20
logical pixels. Remainders are per window and reset at gesture boundaries, focus
loss, scale changes and discrete wheel input.

A single-axis ScrollView discriminates direction when acquiring a trackpad
gesture. After motion starts, diagonal jitter no longer drops subsequent samples
on the chosen axis. End/cancel/new-start restore direction discrimination;
discrete wheel input retains its per-event axis behavior.

The application merges consecutive trackpad Moved samples already queued in an
event batch by default. Start/end/cancel, discrete wheels and intervening events
preserve their order. This has no input-rate timer; Winit's optional 16 ms throttle
remains disabled by default. `SCARLET_UI_APP_WHEEL_COALESCE=0` disables application
coalescing for comparisons.

## Validation on 2026-10-05

Core: 403 tests and 23 doctests; SGFX renderer: 65 tests; Winit: 26 tests, all passed.
Global-scale UI tests ran serially, including ignored tests. Cadence's patched
workspace passed 771 tests, macOS release bundling, and Scarlet AArch64/RISC-V64
release builds with ELF audits. Cross builds do not establish Scarlet runtime FPS.

Cadence was actually operated via native GUI scrolling with a copied real catalog
(5,381 tracks / 358 albums), including a 285-track album detail. The music volume
was unavailable, so that session did not validate loaded real covers. At the
user's request, settings were backed up and reset, then replaced with a playable
synthetic catalog of the same size, 358 different PNG covers, and a 383-track album.
The user also operated the final release and reported smooth scrolling. GUI
automation and that observation complement, rather than replace, gesture tests.

The final interactive synthetic session recorded 4,335 presented frames:
median render/backend time 7.863 ms, p95 8.201 ms; input dispatch median 34 us,
p95 172 us. The whole session also contains startup/navigation/resize and cold
resource work, with a maximum frame of 130.804 ms. These are CPU-side wall times
including backend presentation waits, not scanout measurements or an FPS claim.
Do not discard the cold outliers or infer every physical gesture is covered.

`scarlet-ui-core/examples/scroll-performance.rs` benchmarks event dispatch,
virtualization and command preparation with a counting backend. For 50,000 list
rows on the same host, entering-row-only layout changed median preparation from
180 us to 63 us; this excludes GPU execution and image I/O. The native
`scarlet-ui/examples/scroll-stress.rs` exercises dense CJK text and shared artwork,
but advances selection per presented frame: it is not physical trackpad QA.

## Reproduce

```sh
cargo test -p scarlet-ui-core -p scarlet-ui-renderer-sgfx \
  -p scarlet-ui-platform-winit -- --include-ignored --test-threads=1
cargo run --release -p scarlet-ui-core --example scroll-performance
cargo run --release -p scarlet-ui --features platform-winit --example scroll-stress
```

Set `SCARLET_UI_FRAME_LOG=1` before launching a native application to record input,
layout, paint preparation, SGFX lowering/submission, backend and application cycle
times. The flag is read once and is off by default. Backend timing includes
presentation waits; SGFX encode timing alone does not establish GPU completion.
Capture real wheel input and separate startup/navigation from gesture intervals
when analyzing stalls. Cadence's fixture generator refuses an existing catalog:
`python3 scripts/create-scroll-fixture.py --data-dir NEW_DIR --music-dir NEW_MUSIC_DIR`.

## Retained GPU paint and content commits

Native SGFX backends now opt into immutable local-coordinate display lists.
CPU backends keep their existing raster path. A scroll changes transforms and
clips; paint changes replace affected recordings, and virtualization reconciles
mounted children without rerecording clean rows. Virtual rows are grouped into
one recording to preserve batching. Removed rows release their recordings, and
an offscreen content update commits even when it produces no visible damage.
Nested scrollers invalidate their enclosing grouped row. Content, layout and
scroll metadata are reconciled on the UI thread before submission; this is not
an independent compositor thread.

The SGFX encoder retains up to 256 reusable 64 KiB mesh buffers (at most 16 MiB).
Unchanged geometry uses a GPU transform/scissor rather than tessellation and
vertex upload. Cache validity includes display-list identity, output scale,
image identity/revision and glyph-atlas generation. Current-frame resources are
protected before new rows are lowered. Atlas recycling falls back to an atomic
flat frame rebuild. Unsupported clips, extensions, opacity combinations,
position-dependent rasterization and oversized rows use the existing lowering
path. Text/image rounding is preserved separately from shape translation.

Immutable bitmap sources share their pixel buffers with the recordings. Mutable
borrowed sources are snapshotted once when recorded. They must still invalidate
paint when changed; mutating an image without informing the UI is not a content
commit.

`ListView::item_key`, `GridView::item_key` and `LazyVStack::item_key` opt into
scroll anchoring. Supply unique stable model identities, not indices. Inserting
or removing items above the visible anchor preserves its position; explicit
selection scrolling takes precedence, and shrinking content clamps the offset.
If the anchor disappears, the current offset is clamped. These keys identify
scroll anchors; virtual row reconciliation still follows the existing index
model and does not promise preservation of arbitrary keyed row-local state.

On macOS, the WGPU backend also pools its small fixed-pipeline uniform buffers.
Each draw receives a distinct slot within a submission. Later ordered queue
submissions reuse the slots, avoiding one Metal buffer allocation per draw.
The backend change is pinned at SGFX
`a1156b12083017abfd7e64a7caf37a152ace6d74` (branch
`perf/wgpu-draw-uniform-pool`). The SWS frontend pin is unchanged.

### Automatic native scrolling

`scroll-benchmark` creates a native SGFX/wgpu window and sends normal wheel
events through `RenderingPipeline::handle_event`. Its 50,000-row cases include
plain text, shared 256×256 artwork, and rows with multiple texts, artwork,
progress and a button. Scrolling reverses direction periodically. Each case
runs both with static content and with insert/remove updates above the viewport
once every 60 frames. A one-second warmup is reported separately from measured
frames. No selection-state timer drives the scroll.

```sh
cargo build --release -p scarlet-ui --features platform-winit \
  --example scroll-benchmark
python3 tools/measure-scroll.py --seconds 8 --repeats 2 \
  --output artifacts/scroll-retained-pooled
```

The script alternates baseline/retained order between repetitions, records all
raw logs, and writes `results.json` with p50/p95/p99/max, missed frame-budget
fractions, content-update frame times and preparation/lowering/encoding
timings. `encode_us` includes lowering and submission; do not add it to
`lower_us` as though they were independent phases. `SCARLET_UI_RETAINED_PAINT=0` disables the new recording/mesh path for a
controlled comparison; both modes use the same final WGPU backend.

These wall times include synchronous submission and presentation backpressure.
Work rate is the reciprocal dispatch/submission time, excluding OS event
pumping; it is not whole-loop or physical scanout FPS. Native event-queue latency, disk
image decoding and actual hardware trackpad sampling are outside this automated
benchmark. ProMotion and host scheduling can change presentation cadence;
compare both repetitions and phase measurements, not one average FPS. Do not
run other builds or GPU tests concurrently. Cold startup outliers remain in the
raw logs; warm measured results are not a guarantee of cold-frame latency.

Regression checks for this change: 410 core tests, 23 doctests, 74 SGFX renderer
tests and 26 Winit tests passed serially. SGFX WGPU's 42 tests passed on Apple M3
Pro/Metal, including pixel readback after 100 queued submissions with differing
draw uniforms and two render targets. Scarlet AArch64/RISC-V64 release checks
and the RISC-V64 legacy Scarlet std configuration are checked separately;
macOS measurements do not establish Scarlet runtime frame rates.

### Native measurement results (Apple M3 Pro / Metal, 2×)

Two repetitions per mode, 8 measured seconds after a 1-second warmup, wheel
steps of 128 logical pixels, 1,200×800 logical window. Both modes used the pooled
WGPU backend. Full per-run percentiles, phase times and maxima are in
[scroll-retained-20261005.json](scroll-retained-20261005.json).

| Case / update every | Baseline p95 range (ms) | Retained p95 range (ms) | Median frame vertex bytes: baseline → retained |
| --- | ---: | ---: | ---: |
| Plain / none | 16.732–16.972 | 16.738–16.992 | 488,520 → 10,920 |
| Plain / 60 frames | 16.746–16.899 | 16.735–16.858 | 488,520 → 10,920 |
| Artwork / none | 16.939–16.941 | 16.764–16.774 | 499,320 → 11,160 |
| Artwork / 60 frames | 16.925–16.927 | 16.792–16.823 | 499,320 → 11,160 |
| Complex / none | 17.220–17.411 | 17.003–17.183 | 1,028,520 → 22,920 |
| Complex / 60 frames | 17.223–17.519 | 17.000–17.019 | 1,028,520 → 22,920 |

Vertex traffic fell about 97.8%. In complex static rows, median encoding time
fell from 2.509–2.970 ms to 1.196–1.714 ms. Preparation became more expensive
(0.125–0.127 ms → 0.260–0.503 ms), so the improvement is in the total work and
resource reuse, not every phase. Plain rows do not show a meaningful p95 gain.
Presentation backpressure dominated these runs near 60 Hz.

Retained artwork's second static run had a worse p99 (19.077 ms versus 17.312 ms).
Both complex-update second runs had p99 near 33 ms. The worst baseline frame was
1,128.139 ms in plain-update run 1; the worst retained frame was 49.772 ms in
complex-update run 1. No outliers have been discarded or attributed to a specific
cause without evidence. These measurements establish lower geometry traffic and
less complex-row encoding work, not universally perfect frame pacing.

An additional complete 12-run comparison after fixing clear-only damage outside
a rounded fallback clip is included in the JSON report. Stateful recordings
whose opacity or clip changes cross display-list boundaries use a full flat
lowering pass; two geometry-equivalence cases cover that conservative fallback.

Final native Cadence GUI QA used a separate data-directory copy and release
window, leaving the original playback window running. The copied catalog had
10,762 tracks / 716 albums, including the generated cover fixtures. Actual CUA
wheel input moved the album grid from fixture 000 to 031 and back, the 383-track
detail from disc 1 track 1 to 108 and back, and all songs from row 1 to 128 and
back. Screenshots showed the corresponding rows/covers with no missing content.
The entire 1,087-frame session, including cold startup/navigation, recorded
median frame time 3.406 ms, p95 25.312 ms, p99 32.895 ms and max 130.281 ms;
input dispatch median 59 us / p95 493 us. This mixed session is not a gesture-only
FPS or latency claim. The clear-only clip regression found during GUI startup
was fixed and covered by a dedicated test before this successful session.

Grouped recordings also preserve the original shared image `Arc<Buffer>` through
flattening. A resource-identity regression test prevents grouping from turning
shared covers back into copied borrowed snapshots. The recorded comparisons
precede this last sharing fix; it does not change geometry or the cache limits.
