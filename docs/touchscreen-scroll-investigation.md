# Touchscreen diagonal scroll investigation

Baseline: ScarletUI `dd2abcf5`, investigated 2026-11-14. This concerns native
touchscreen contacts, not trackpad wheel events.

## Ownership and routing

The application-side gesture and scroll policy is userspace:

- Scarlet SWS `user/std-bin/src/sws/touch.rs::GestureRecognizer::process`
  forwards `TouchSurface::Touchscreen` as `DirectTouch`. Its indirect/trackpad
  recognizer is a separate branch.
- SWS `compositor.rs::handle_direct_touch_frame` retains a contact's window
  grab and sends native contact changes to subscribed surfaces. System edge
  navigation can intercept contacts before application dispatch.
- `scarlet-ui-platform-sws/src/lib.rs` maps `SwsEvent::TouchFrame` to core
  `TouchFrame`, converting physical coordinates to logical coordinates.
- `scarlet-ui-core/src/event/touch.rs` resolves an 8-logical-pixel radial slop,
  cancels presses when scrolling wins, and emits both delta components. It
  does not repeatedly lock the scroll axis. Axis competition with control
  drags occurs when leaving the pending state.
- `event/dispatcher.rs` retains the down hit path, dispatches scroll gestures,
  handles nested consumers, and starts UI-owned momentum on release.
- `views/scroll.rs::handle_event` applies direct-touch deltas without the
  wheel sensitivity or wheel axis-lock filters. `element/render.rs` applies
  the scroll offset and marks the owning pipeline for composition immediately.

This establishes ownership, not that every physical stall originates in UI.
Device delivery, SWS scheduling, and GPU/presentation latency still require
the parent's hardware investigation.

## Confirmed capture defect and fix

Previously the first scroll consumer became the deepest allowed target for
the rest of the contact. If a nested child was initially clamped, or its axis
received no movement, its parent could consume the initial move. Later moves
never reached that child, even after a reversal or a change to diagonal motion.

The dispatcher now captures the deepest touch-scroll-capable element on the
original path after the first scroll is consumed, retaining its ancestor chain.
An ancestor may consume an unhandled move, and the child gets another chance
on the next move. The fallback for custom consumers remains the actual consumer.
This keeps contact capture tied to the original path, without re-hit-testing
at each finger position. Terminal events still go to all recorded consumers;
momentum still starts at the last consumer. Recognition thresholds and control
drag competition are unchanged.

`crates/scarlet-ui-core/tests/touchscreen_scroll_traces.rs` contains deterministic
16 ms input traces with assertions on offsets before any subsequent render:

| Trace | Baseline | Fixed behavior |
| --- | --- | --- |
| Vertical, diagonal, horizontal-dominant zigzag, reversal, and leaving bounds | Pass | Same offsets: 12, 24, 36, 20 px |
| Vertical child in horizontal parent: first delta (-20, 0), then (-1, -20) | Child remains at 0 px | Child reaches 20 px; parent stays at 20 px |
| Child starts at bottom (200 px): initial upward move goes to parent; reverse by 10 px | Child remains at 200 px | Child returns to 190 px; parent stays at 20 px |
| Child consumes first, reaches edge, hands off, reverses, then releases | Regression coverage | Child regains movement; momentum follows last consumer and new down cancels it |
| Slop (5, 5) then (6, 6), followed by a tiny reversal | Pass | One scroll start, full accumulated delta, immediate subsequent reversal |

The single-view traces run both with rendering between moves and with all
renders withheld after setup. Offsets and dirty state update synchronously;
the simple retained scroll path requests no layout. The two failing nested
traces reproduce without GPU rendering, distinguishing this capture defect
from render cost. These tests do not measure hardware input-to-photon latency.

## Reproduction and validation

Use the shared checkout with an isolated target directory:

```sh
export CARGO_HOME=/Users/petitstrawberry/Development/Rust/Scarlet/.work/chromebook-cargo-home
export CARGO_TARGET_DIR=/Users/petitstrawberry/Development/Rust/Scarlet/.work/chromebook-sync/work/validation/gesture-target
nix develop --command cargo test -p scarlet-ui-core --test touchscreen_scroll_traces --offline
nix develop --command cargo test -p scarlet-ui-core --lib event:: --offline
nix develop --command cargo test -p scarlet-ui-core --lib touch_scroll_tests --offline
nix develop --command cargo test -p scarlet-scroll-motion --offline
```

Results: all five trace tests, 53 event tests (including taps, control drags,
multitouch cancellation, and focus), two TextView touch tests, and two
scroll-motion tests pass. The baseline had two failing nested traces out of
the original four tests. An additional `cargo check -p scarlet-ui-core
--no-default-features --offline` was attempted but is not a supported crate
configuration: `src/lib.rs` explicitly requires either `std` or
`legacy-scarlet-std`. It is not a successful target-platform build check.

Baseline and final outputs are preserved in the Chromebook validation directory
as `gesture-baseline.log`, `gesture-fixed.log`, and `gesture-regressions.log`.
Hardware confirmation of the user's specific stall remains outstanding.
