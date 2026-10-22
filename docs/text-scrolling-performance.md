# Text scrolling CPU benchmark reconciliation

The verified comparison below measures **CPU wheel handling and paint-command
recording**, not completed rendered frames. Every measured scroll moved the
viewport, and all ten baseline/text-optimized samples matched their viewport,
visible rows, command counts and ordered text-position/content hashes.

## Historical figures

The earlier 3,000-line **1,670.460197 ms** baseline was wheel handling alone.
**6,033.635660 ms** additionally included render-object update, layout and
paint-command recording. The earlier **0.041353 ms** result belongs to that
combined workload. Those two baseline scopes must not be interchanged.

The original baseline raw log, compiler identity and exact concurrent optimized
working-tree revision were not preserved. Its harness also did not assert
movement or inspect recorded commands. Historical numbers are therefore
superseded by this controlled comparison, rather than used for a speedup claim.
The old optimized log remains locally at `/tmp/text-benchmarks-final.log`.
Source inspection confirms that the historical combined interval called paint;
it also included command-buffer destruction, which the verified interval excludes.
The exact original **0.041353 ms cannot itself be revalidated** because its
working-tree/compiler provenance is incomplete. The controlled replacement
**0.037585 ms** below proves equivalent active work under recorded conditions.

## Sources and environment

| Case | Exact production-source commit |
| --- | --- |
| Baseline | `ec25336241e70d2fcdad186b3b50f8aa449d303a` |
| Text optimization | `21b75b5ff4e7408154fa2eb9503cda40534bb9ea` |
| Later typography snapshot | `0c09ab24fb90e111fadbf05131a94aacfcab458b` |

Each case used a separate git archive and Cargo target directory. Only the test
module declaration and identical verified benchmark harness were injected;
all other archived blobs were checked against the source commit. All three
Cargo.lock files have SHA-256
`6485d5cc75622256007d3a02031b124de68f1f428957c1393a2e876470afa8fd`.
The injected harness SHA-256 is
`6d0154a85b18a0417a5c9a3465f89001d3043115e9985ee8a47f604daf238549`.

The run used rustc **1.94.0-nightly**, commit
`26d00a8a3a8489d73f5ae3575880e9f915dfd0f2` (2026-10-13), LLVM 21.1.8,
and Cargo 1.94.0-nightly on x86_64 Linux. Cargo artifact metadata confirms release
optimization level 3, debug information 0, debug assertions off, overflow checks
off, and the `default,std` features. This was not a debug build.
The machine reports Intel Xeon Platinum 8573C with a four-CPU container quota.

The explicit font stack was Mplus1-Regular.ttf, NotoSansSymbols2-Regular.ttf,
then NotoEmoji-Variable.ttf from the desktop bundle. Font loading and a short
coverage warm-up preceded timing. Complete font hashes, executable hashes,
compiler metadata, raw sample witnesses and commands are preserved in
[`benchmarks/text-scroll-2026-10-22.json`](benchmarks/text-scroll-2026-10-22.json).

All binaries were compiled first. The final pass ran them back-to-back after
other CPU builds finished; there were no rustc processes at its start. These
remain observations on one shared machine, not portable timing guarantees.
A preliminary shared-target archive run was discarded after Cargo reused an
executable. Accepted results use distinct targets and runtime assertions that
the executable's compiled manifest directory matches the requested snapshot.

## Workload and timing boundary

The fixture has 3,000 numbered lines, 249,000 UTF-8 bytes, containing ASCII,
Japanese, a combining mark, a joined emoji, a tab and a newline. It uses String
state, external scroll state, soft wrapping, line numbers, 15 px font size,
12 px padding and a **640 × 480** viewport. Selection is collapsed at byte zero,
the view is unfocused, and IME is inactive.

Each mode has five samples, with no discarded scroll warm-up iterations. Wheel
`delta_y` alternates -31/+29, producing +31/-29 pixel viewport displacement.
The wheel-only mode moves to y = 31, 2, 33, 4, 35. The combined mode continues
from there to y = 66, 37, 68, 39, 70. Both use the same render object.

- **Wheel only:** direct TextView wheel handler and a black-box observation of
  layout. An untimed paint afterward verifies the resulting visible output.
- **Wheel + update/layout/record:** the same wheel handler, render-object update
  using the existing view, same-size layout and paint-command recording.
  This is not a whole application or element-tree rebuild.

Assertions, witness hashing, command-buffer construction/destruction, font
loading and initial construction/layout are outside both sample intervals.
The initial construction/layout is reported separately as one observation.
A fresh command buffer and actual paint call are used for every combined sample.

**Neither interval includes glyph rasterization, renderer lowering, draw
submission, GPU/compositor work, presentation or input-to-screen latency.**
`PaintContext::draw_text` records strings and positions for a later renderer.
No frame rate or native scrolling-latency claim follows from these timings.

## Verified results

All values are milliseconds. Median is the middle of five sorted samples.
Nearest-rank p95 is the maximum with only five samples; it is not a reliable
tail-latency estimate. Every sample is available in the JSON evidence.

| Case | Initial construction/layout, once | Wheel median | Wheel p95 (= max) | Combined median | Combined p95 (= max) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Baseline | 2820.959 | 1822.955422 | 1832.785192 | 6902.559349 | 7008.990224 |
| Text optimization | 84.448 | 0.003652 | 0.004827 | 0.037585 | 0.183547 |
| Later typography snapshot | 75.333 | 0.003644 | 0.009257 | 0.030148 | 0.241199 |

For the baseline/text-optimization pair, all ten samples have identical scroll
positions, visible ranges, 3,001 visual rows, 71–73 total commands, 64–66 DrawText
commands and 32–33 document-body rows. The ordered hashes include every DrawText
position, font size and UTF-8 string. Each sample's hash changes from the previous
viewport. Handler success, exact unclamped movement, external-state agreement,
row-coordinate displacement and nonempty body-text recording are asserted.
The optimized recording measurement is therefore active viewport work, not an
unchanged viewport, skipped paint call or empty command buffer.

The typography snapshot deliberately changes font metrics and displays only
22–23 body rows in this viewport, with 44–46 DrawText commands. Its output is
**not identical** to the earlier pair, so its lower recording time must not be
attributed solely to the text-cache optimization.

## Reproduction commands

The verified section of
`crates/scarlet-ui-core/src/views/text_view/performance.rs` is shared by all three
archives. To reproduce the current checkout's guarded workload:

```sh
/workspace/cloud-setup/bin/dev scarlet-ui env \
  TEXT_BENCH_LINES=3000 TEXT_BENCH_SAMPLES=5 \
  TEXT_BENCH_FONT_FILES=/workspace/Scarlet/bundles/desktop/fs/share/fonts/Mplus1-Regular.ttf:/workspace/Scarlet/bundles/desktop/fs/share/fonts/NotoSansSymbols2-Regular.ttf:/workspace/Scarlet/bundles/desktop/fs/share/fonts/NotoEmoji-Variable.ttf \
  cargo test -p scarlet-ui-core --release text_view_verified_scroll_benchmark \
  -- --ignored --nocapture --test-threads=1
```

For the recorded comparison, `case` was `before`, `after`, or `current`; each
archive lived under `/workspace/scratch/text-scroll-reconciliation/$case`.
The following command compiled each case, saving Cargo's JSON artifact output
so the exact executable and profile could be checked before execution:

```sh
/workspace/cloud-setup/bin/dev scarlet-ui env \
  CARGO_TARGET_DIR=/workspace/cloud-setup/targets/scarlet-ui/text-scroll-reconciliation-$case \
  cargo test --locked \
  --manifest-path /workspace/scratch/text-scroll-reconciliation/$case/Cargo.toml \
  -p scarlet-ui-core --release text_view_verified_scroll_benchmark \
  --no-run --message-format=json
```

After all compilation finished, the final runner executed the following for
`before`, `after`, then `current`, with the same font variable shown above:

```sh
/workspace/cloud-setup/bin/dev scarlet-ui env \
  TEXT_BENCH_LINES=3000 TEXT_BENCH_SAMPLES=5 \
  TEXT_BENCH_EXPECT_MANIFEST_DIR=/workspace/scratch/text-scroll-reconciliation/$case/crates/scarlet-ui-core \
  TEXT_BENCH_FONT_FILES=/workspace/Scarlet/bundles/desktop/fs/share/fonts/Mplus1-Regular.ttf:/workspace/Scarlet/bundles/desktop/fs/share/fonts/NotoSansSymbols2-Regular.ttf:/workspace/Scarlet/bundles/desktop/fs/share/fonts/NotoEmoji-Variable.ttf \
  /workspace/cloud-setup/targets/scarlet-ui/text-scroll-reconciliation-$case/release/deps/scarlet_ui_core-dcda43713be14269 \
  text_view_verified_scroll_benchmark --ignored --nocapture --test-threads=1
```

Executable names are compiler-specific; use the path reported by Cargo's
`compiler-artifact` message for another environment. The recorded full commands
and SHA-256 identities are in the JSON. Local final logs are
`/workspace/scratch/text-scroll-reconciliation/{before,after,current}-final.log`.

## Coverage and remaining limits

This timing comparison covers only 640 × 480. Existing regressions separately
exercise fractional 180.5 × 96.25 viewports, 140 × 90 to 140 × 230 height changes,
soft/unwrapped layouts, Unicode boundaries, whitespace preservation, cached
versus fresh geometry, caret/selection and IME behavior. No small/large viewport
performance matrix, DPI matrix or native GPU/presentation timing was measured.

Document snapshots and measured rows avoid repeated parsing/measurement during
scrolling. String-bound updates still clone/compare the String; document-bound
updates share a snapshot. The zero-allocation regression covers a 1,000-line
document-bound scroll-state change, update and unchanged-size layout with an
unchanged font revision—not wheel dispatch, paint recording or application
rebuilds. A separate mounted-pipeline test covers real wheel-triggered rebuilding.

Caches remain proportional to document size. Edits rebuild indices and layout;
scrolling updates row coordinates in O(visual rows). A visible unwrapped line is
submitted whole to the renderer, so these recordings establish no GPU performance
claim for long clipped lines. Direct-touch scrolling and momentum were not added.
