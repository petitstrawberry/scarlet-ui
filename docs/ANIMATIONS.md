# Animations

ScarletUI animates supported presentation properties on existing views. A
`State` write changes the model immediately; the retained view's displayed
values then move toward the new target. Animation ticks update presentation,
layout, and paint as needed without rebuilding application views on every tick.

Import the app-facing APIs from `scarlet_ui::prelude::*`. Applications use the
normal derived `View`, `Application::scenes()`, and `app.run()` path; no animation
loop, platform backend selection, or per-frame state updates are needed.

## Animation Lab

[Open Animation Lab](https://ui.scarlet-os.org/animation-lab/) alongside
[Widget Factory](https://ui.scarlet-os.org/widget-factory/), or run the same
application natively:

```sh
cargo run --release -p scarlet-ui-animation-lab
```

NavigationView shows a sidebar on wide windows and bottom navigation on narrow
ones, matching Widget Factory. Its three pages preserve their models while you explore:

- **Play**: a spinning loader, zigzag gauge, squashing jelly character, and
  star/heart celebration. **Load again** fills the gauges and celebrates at
  completion. **Celebrate!**, **Mix colors**, **Speed up / Slow down**,
  **Pause / Resume**, and **Reset** work during motion.
- **Basics**: explicit `with_animation` and value-triggered `.animation(...)`
  change size, color, offset, and opacity. Reverse during a transition or use
  **Reset to A** to settle immediately.
- **Layout**: composed cards move between Board, Rows, and Spotlight. Shuffle
  preserves card identity; Refresh animates the charts; Theme changes colors.

The [shared application](../examples/animation-lab/src/lab.rs) runs on native
and WebGPU. Only the selected page is mounted. Cards and controls reflow on
narrow viewports; the page scrolls without hiding controls outside the canvas.
The browser needs HTTPS (or localhost) and an available WebGPU adapter.

The [Play page](../examples/animation-lab/src/play.rs) retains `AnimatedValue<f32>`
tracks for phase, progress, and particles. The normal presentation clock
advances them; [drawing code](../examples/animation-lab/src/paint.rs) turns the
displayed values into geometry. Actions write target state, not per-frame
intermediate values. Background browser tabs pause their frame loop.

Native smoke mode exercises all three pages, including pause/resume, reverse,
reset, and returning to Play, then closes after about 23 seconds:

```sh
SCARLET_UI_ANIMATION_LAB_SMOKE=1 cargo run --release -p scarlet-ui-animation-lab
```

This checks application lifecycle and actions, not frame-rate performance.
See the [Lab README](../examples/animation-lab/README.md) for Web build steps.

## Explicit animation around a state write

This complete app keeps the same view structure while changing its properties:

```rust
use core::time::Duration;
use scarlet_ui::{platform::WindowPlacement, prelude::*};
use scarlet_ui_macros::View;

#[derive(View, Clone)]
#[view(body = content)]
struct MotionApp {
    expanded: State<bool>,
}

impl MotionApp {
    fn content(&self) -> impl View + Clone + 'static {
        let expanded = self.expanded.get();
        let toggle = self.expanded.clone();
        let reset = self.expanded.clone();
        let tile = Text::new("Hello").color(Color::WHITE)
            .frame(if expanded { 180.0 } else { 100.0 }, 56.0)
            .background(if expanded { Color::BLUE } else { Color::RED })
            .offset(if expanded { 60.0 } else { 0.0 }, 0.0)
            .opacity(if expanded { 0.5 } else { 1.0 });

        scarlet_ui::vstack! {
            ZStack::new((tile,)).alignment(Alignment::Leading).frame(300.0, 100.0),
            Button::new("Animate / reverse").on_click(move || {
                with_animation(Animation::ease_in_out(Duration::from_millis(600)), || {
                    toggle.update(|value| *value = !*value);
                });
            }),
            Button::new("Reset to A").on_click(move || {
                without_animation(|| reset.set(false));
            }),
        }.spacing(16.0).padding(24.0)
    }
}

impl Application for MotionApp {
    fn scenes(&self) -> impl Scene {
        Window::new("Motion", self.content())
            .scene_key("motion")
            .size(Size::new(400.0, 300.0))
            .placement(WindowPlacement::At { x: 80, y: 80 })
    }
}

fn main() -> scarlet_ui::Result<()> {
    let mut app = MotionApp::default();
    app.run()
}
```

`with_animation(animation, closure)` also accepts `Some(animation)` or `None`.
The closure executes synchronously and returns its ordinary result. Reading
the state immediately after `set` or `update` observes the new model value,
even while the previous value is still being presented.

## Implicit animation scoped by a value

In the app above, replace the tile declaration with this version and remove
`with_animation` from the toggle button:

```rust
let tile = Text::new("Hello").color(Color::WHITE)
    .frame(if expanded { 180.0 } else { 100.0 }, 56.0)
    .background(if expanded { Color::BLUE } else { Color::RED })
    .offset(if expanded { 60.0 } else { 0.0 }, 0.0)
    .opacity(if expanded { 0.5 } else { 1.0 })
    .animation(Animation::ease_in_out(Duration::from_millis(600)), expanded);

let toggle_button = Button::new("Animate / reverse")
    .on_click(move || toggle.update(|value| *value = !*value));
// Use toggle_button in the VStack in place of its explicit-animation button.
```

The trigger must implement `Clone + PartialEq + 'static`. Pass the current
value, such as `state.get()`. The modifier changes the animation policy for
its descendants when that value changes. It can affect every supported
property updated in that subtree during the same rebuild, so put the scope
around the smallest intended region.

Modifier order matters: the final `.animation(...)` above wraps the frame,
background, offset, and opacity. A property modifier placed after it sits
outside that scope. An unchanged trigger inherits the surrounding transaction;
it does not replay the previous implicit animation. The trigger is separate
from view identity and should not be used as a changing view key.

## Immediate changes and transactions

Use `without_animation` to apply an update immediately, even inside an implicit
animation scope:

```rust
without_animation(|| expanded.set(false));
```

`with_animation(None, ...)` removes the inherited animation, but a descendant
whose `.animation(..., value)` trigger changes can still supply one.
`without_animation` additionally sets `disables_animations`, preventing those
descendants from adding animation for the update.

Ordinary duplicate state notifications with equal targets preserve ongoing
transitions. An explicit `without_animation` update also settles existing
active supported properties that reconcile, even when a `State::set`
notification repeats the current target. Thus the reset above snaps a return
transition to A as well. This applies to the properties reached by that update;
an empty closure causes no update and does not cancel animations.

`Transaction` exposes both policy fields when explicit control is useful:

```rust
with_transaction(
    Transaction {
        animation: Some(Animation::ease_out(Duration::from_millis(400))),
        disables_animations: false,
    },
    || expanded.set(true),
);

with_transaction(
    Transaction { animation: None, disables_animations: true },
    || expanded.set(false),
);
```

`Transaction::default()` has no animation and does not disable descendant
animation scopes. Nested synchronous scopes restore the preceding policy when
they return. These policies travel with state invalidation into the deferred
view rebuild; they are not a request to interpolate the `State` itself.

## Timing and interruption

| Constructor | Behavior |
| --- | --- |
| `Animation::linear(duration)` | Constant progress. |
| `Animation::ease_in(duration)` | Quadratic acceleration. |
| `Animation::ease_out(duration)` | Quadratic deceleration. |
| `Animation::ease_in_out(duration)` | Symmetric quadratic acceleration and deceleration. |
| `Animation::spring(duration, damping_ratio)` | Bounded spring motion with an explicit completion deadline. |
| `Animation::default()` | 300 ms ease-in-out. |
| `.repeat_forever(autoreverses)` | Reuse the curve continuously, optionally retracing it backward every other leg. |

All durations are `core::time::Duration`. Zero duration snaps a changed target.
Spring damping must be finite and nonnegative; negative values, NaN, and
infinities panic at construction. Ratios below 1 are underdamped, 1 is critical,
and values above 1 are overdamped. Zero damping is allowed.

Spring progress stays within `[0, 2]`, allowing overshoot. A settling window
`(1 - t * t)^2`, where `t` is normalized time, reduces remaining displacement
to zero at the deadline. This intentionally shapes the physical spring
response so even an undamped spring finishes. Completion assigns the exact
target. Color channels and opacity additionally clamp to their valid ranges;
presented frame dimensions and constraints clamp to nonnegative lengths.

A new target during motion starts from the currently displayed value, avoiding
a positional jump. It starts a fresh curve and duration; spring velocity is
reset rather than preserved. Ordinary duplicate target updates do not restart
the timer. Keep the same view type, structural position, and stable key to
retain its identity and transition.

For a repeating transition, `duration` is the duration of one leg. Without
autoreversing, the value wraps to the starting point at each boundary; use a
periodic visual such as an angle when that wrap should look continuous. With
autoreversing, a round trip takes two legs. Zero duration still snaps and does
not leave a repeating animation active. Finishing a track assigns its exact
target and stops repetition.

## Supported properties and current limits

Arbitrary `View` subtrees can be wrapped in the supported frame, background,
offset, and opacity modifiers. A custom component containing these modifiers
works too: its supported properties animate when their targets change under
an animation policy. This lets a card with text, charts, and buttons move,
resize, change its background, or fade as composed content. Resizing a frame
changes layout; it does not scale the contents or interpolate their font sizes.

| Property | Supported animation | Boundary |
| --- | --- | --- |
| `Frame` / `.frame(...)`, `.frame_width(...)`, `.frame_height(...)` | Finite dimensions and min/max constraints. | Layout changes as the displayed frame changes. Adding/removing an explicit dimension or changing to/from unbounded sizing snaps that field. |
| `Background` / `.background(color)` | Base RGBA color. | Stored channels interpolate independently and clamp to `[0, 1]`; no gamma conversion or alpha premultiplication. This does not animate every color property, such as `Rectangle::fill` or a hover override. |
| `.offset(x, y)` | Both translation coordinates. | Moves presentation and hit-testing coordinates without changing the layout footprint; reserve enough room for travel and overshoot. |
| `.opacity(value)` | Opacity clamped to `[0, 1]`. | Modulates drawing primitives; overlapping descendants are not rendered into an isolated compositing group. |

Initial insertion displays the initial values immediately. Animation currently
applies to updates of these existing properties, not insertion/removal
transitions or automatic layout matching. Scale, rotation, `Text` color or font
size, `Rectangle::fill`, and custom graphics properties are not automatically
animated. Arbitrary `State<T>` fields are not interpolated; a boolean or enum
may choose target property values without itself being interpolated.

The public `Animatable` trait supplies interpolation for `f32`, `f64`,
`[f32; N]`, `Point`, `Size`, and `Color`; implementing it for a custom model does
not automatically make its state or a custom view property animate.

Custom render objects can opt in using `AnimatedValue<T>`. Initialize it from
the view property, then call `retarget(next, Transaction::current().effective_animation())`
in the view updater. Implement `has_visual_animation`,
`advance_visual_animation(delta)`, and `finish_visual_animation` by delegating
to the retained tracks. Render from `value()`, and select layout or paint
invalidation through the ordinary render-object hooks. The playground provides
a complete example, including pausing, repeated clocks, and window resumption.
This is explicit integration for custom properties, rather than automatic
interpolation of arbitrary state fields.

The runner advances presentation on actual frame grants where the backend
supports them, with a 16 ms fallback otherwise. That is a scheduling policy,
not a guaranteed refresh rate. Applications should not add a timer that
repeatedly writes intermediate state values to drive these transitions.

## Async work: wrap the actual write after awaiting

Animation scopes are synchronous and do not cross an `await`, a detached task,
or a thread boundary. Creating a future inside `with_animation` does not apply
the policy to its later execution. Wrap the state write after the work finishes:

```rust
async fn apply_result<F>(result: F, expanded: State<bool>)
where
    F: core::future::Future<Output = bool>,
{
    let next = result.await;
    with_animation(Animation::default(), || expanded.set(next));
}
```

Keep the actual write on the application's usual state-update path. For
conceptual background, Apple's [Transaction documentation](https://developer.apple.com/documentation/swiftui/transaction)
describes animation policy traveling through a view hierarchy, and
[animation(_:value:)](https://developer.apple.com/documentation/swiftui/view/animation%28_%3Avalue%3A%29)
describes value-triggered animation. ScarletUI's supported properties, timing,
and limitations are the ones documented here; SwiftUI's broader API is not a
ScarletUI compatibility contract.
