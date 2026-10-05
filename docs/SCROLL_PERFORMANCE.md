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
