# Text editing CPU measurements

Measured on 2026-10-22 in the supplied Nix development environment, starting
from app-pinned revision `ec25336241e70d2fcdad186b3b50f8aa449d303a`.
The baseline contained the same benchmark harness, before the document and
layout changes. The before/after comparison records the text-editing milestone;
a later current-toolchain run after the typography correction is listed
separately below. These are observations from this machine, not timing assertions.

## Reproduction

```sh
/workspace/cloud-setup/bin/dev scarlet-ui cargo test -p scarlet-ui-core --release text_view_cpu_benchmark -- --ignored --nocapture --test-threads=1
```

The text-editing milestone run also included the long-line experiment:

```sh
/workspace/cloud-setup/bin/dev scarlet-ui cargo test -p scarlet-ui-core --release text_view_ -- --ignored --nocapture --test-threads=1
```

The harness lives in `crates/scarlet-ui-core/src/views/text_view/performance.rs`.
`TEXT_BENCH_LINES` overrides the comma-separated line counts;
`TEXT_BENCH_SAMPLES` overrides the default five samples per case. Font discovery
is warmed before timing. Each numbered line contains ASCII, Japanese, a combining
mark, a joined emoji, a tab and a newline (83 UTF-8 bytes per line). The view uses
String state, external scroll state, soft wrapping, line numbers, 15 px text,
12 px padding and a 640 × 480 viewport. Wheel events alternate -31 and +29.

The first mode times direct wheel handling. The second additionally calls the
render object's update, layout and paint-command recording methods. It does not
time a complete application rebuild, GPU rendering, presentation or input latency.

## Results

All values below are milliseconds. Initial layout is one observation; wheel and
update/layout/record values are medians of five samples.

| Lines / bytes | Initial before | Initial after | Wheel before | Wheel after | Update + record before | Update + record after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 / 8,300 | 77.175 | 3.070 | 64.773763 | 0.000200 | 192.712380 | 0.021910 |
| 1,000 / 83,000 | 990.074 | 35.727 | 677.610473 | 0.001461 | 1254.137271 | 0.041718 |
| 3,000 / 249,000 | 2636.438 | 125.866 | 1670.460197 | 0.003800 | 6033.635660 | 0.041353 |

The text-editing milestone run passed both ignored benchmark tests. Its complete local log is
`/tmp/text-benchmarks-final.log`; baseline output was captured in the development
tool transcript, not a persistent log. Earlier after-change measurements also
exist in `/tmp/text-benchmark-after.log`; timing varied during parallel builds,
so the table consistently uses that milestone run. With five samples, the reported
95th percentile in the raw output is simply the largest observation.

The after-only long-line experiment uses document state, no wrapping, mixed
Unicode text and a 640 × 90 viewport. It averages 10,000 hit-test/caret-query
pairs and 20 paint recordings. It has no corresponding baseline measurement.

| UTF-8 bytes | Initial layout | Hit test + caret, mean | Paint recording, mean |
| --- | ---: | ---: | ---: |
| 5,000 | 1.045 | 0.000031 | 0.000488 |
| 50,000 | 29.323 | 0.000048 | 0.001539 |
| 500,000 | 119.198 | 0.000057 | 0.012781 |

## Current source after typography correction

The same two release benchmarks passed again on 2026-10-22 with the current
cached Scarlet compiler and corrected font advances/baselines. Complete output
is in `/tmp/text-benchmarks-post-typography.log`. Font metrics and the compiler
differ from the earlier milestone, so differences between these runs should not
be attributed solely to the text cache implementation. Scope and fixtures remain
as described above; every value is milliseconds.

| Lines / bytes | Initial layout | Wheel median | Update + record median |
| --- | ---: | ---: | ---: |
| 100 / 8,300 | 2.093 | 0.000165 | 0.010900 |
| 1,000 / 83,000 | 22.866 | 0.001277 | 0.015706 |
| 3,000 / 249,000 | 67.518 | 0.003696 | 0.023519 |

| UTF-8 bytes, unwrapped | Initial layout | Hit test + caret, mean | Paint recording, mean |
| --- | ---: | ---: | ---: |
| 5,000 | 0.562 | 0.000025 | 0.000278 |
| 50,000 | 6.811 | 0.000031 | 0.001306 |
| 500,000 | 71.109 | 0.000041 | 0.014771 |

## What changed and what remains

Document clones share immutable snapshots with cached text, line starts and
grapheme boundaries. Layout caches document/font/wrapping inputs and measures
scalar advances once. Scroll changes reuse this layout; selection painting,
caret placement and hit testing use cached prefix widths. String-bound updates
still clone/compare their String, while document-bound updates share the snapshot.

The allocation regression covers a document-bound 1,000-line scroll-state change,
render-object update and unchanged-size layout while the font revision remains
unchanged. That specific path allocates zero times. It does not cover wheel
dispatch, paint recording, a complete application rebuild,
text edits or all widget configurations. A separate mounted-pipeline regression
checks that a real scroll-triggered rebuild retains the document and measured
layout. Concurrent discovery of a new fallback font legitimately invalidates the
layout; allocation checks retry only when that external revision changes and
still require a complete stable-revision run to pass.

The caches trade memory for repeated-work savings: the snapshot retains flattened
text and grapheme/line indices; visual rows retain display text, byte-prefix widths
and caret stops. Cache storage grows with document size and is not a bounded
viewport cache. Edits still rebuild snapshot indices and layout; scrolling still
updates row coordinates in O(visual rows). A visible unwrapped line is submitted
in full to the renderer, so these CPU recordings establish no GPU performance
claim for long horizontally clipped lines. Direct touch scrolling and momentum
are not added by this change.

Regression coverage compares cached and fresh layout coordinates, wrapping and
whitespace preservation, Unicode boundaries, font/width/text invalidation,
external selection reveal versus manual scrolling, IME state retention and
TextField horizontal caret/preedit visibility.
