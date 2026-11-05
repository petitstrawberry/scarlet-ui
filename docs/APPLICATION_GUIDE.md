# Application guide

Practical application, window and input behavior. The [architecture](ARCHITECTURE.md)
and [application contract](1.0-contract.md) remain the normative references.

## Application Model


Window decoration defaults are selected by the backend: winit uses system
frames and titlebars, while SWS uses ScarletUI decorations. The selection is
resolved when a scene is built, so windows created before `app.run()` also
receive the backend policy. Explicit `WindowDecoration::CUSTOM`, `SYSTEM`,
and `NONE` choices remain authoritative.

Window children receive the content area's layout constraints. To track the
available viewport, expand the content with `.frame(f32::INFINITY, f32::INFINITY)`
and observe it with `.on_geometry_change(|geometry| geometry.size(), ...)`.
Geometry callbacks run after layout and report only changed values. Do not
subtract titlebar heights from `PlatformWindow::size()`: that is the platform
surface size, including client-rendered chrome and any shadow outsets, whereas
the child geometry already excludes those. Native decorations are outside the
winit client surface. Min/max sizes and resizability are forwarded to winit.

Platform resize notifications update layout and presentation through
`PlatformWindow::apply_resize`; winit does not echo their size back to the OS.
`PlatformWindow::resize` remains the explicit application request. SWS keeps
its existing backing-buffer negotiation through the default implementation.
The runner also reconciles the native extent with the rendered extent each
tick, covering native restoration or resize changes whose notification was
not delivered. Losing window focus cancels stale view capture without synthesizing a click.

On macOS, the application runner stays inside winit's native event loop and
finishes resize layout and presentation before the native redraw callback
returns. This preserves AppKit's live-resize tracking and avoids stretching a
stale frame after the mouse is released. Other backends retain the polling
runner. New scenes created during callbacks use the active event loop.

SWS windows support native gamepad events and optional menu navigation.
See [gamepad input](GAMEPAD_INPUT.md) for game controls, reset handling,
and per-window policy.

`Application::scenes()` is the application UI entry point. `#[derive(View)]`
collects the application's State dependencies; scene-only applications do not
need a component body method. Reusable View components select their child
builder explicitly with `#[view(body = method_name)]`, as the counter in the [root README](../README.md) does
with `content()`. Mounting a derived View without that attribute is a
configuration error, not an alternate way to enter the application's scenes.

Each scene declaration produces a top-level `Window`. At runtime, ScarletUI
creates one `WindowSlot` per opened window:

```text
Application::scenes()
  -> Scene declarations
  -> ApplicationRunner
  -> Box<dyn PlatformBackend>
  -> WindowSlot { WindowId, PipelineId, RenderingPipeline, PlatformWindow }
```

Application code should not choose or name a platform implementation. It imports
the normal ScarletUI prelude, declares views and scenes, and calls `app.run()`.

The normative runtime and reconciliation model is documented in
[ScarletUI Architecture](ARCHITECTURE.md).
The [1.0 application and extension contract](1.0-contract.md) specifies
public features, Scene/Window lifetime, launch/open/close behavior and platform
extension responsibilities using the current implementation.

Use `ViewExt::on_event_capture` on a container to handle shortcuts before a
focused descendant, such as a `TextField`, receives them. Its callback runs
only during capture, from the root to the target's parent. Returning `true`
stops target and bubble dispatch; returning `false` lets the control handle
the event normally. Events dispatched only to the target, such as IME commits
and focus changes, do not invoke capture handlers. `on_event` keeps its target
and bubble behavior.

Frame admission, safe discard, and application recovery follow the
[frame-failure contract](FRAME_FAILURES.md). `Application::on_render_error`
reports transient capacity pressure, rejected frames, and recovery-required GPU
failures separately; a discarded frame is never reported as presented.

## Common Views

- `Text`
- `Button`
- `TextField`
- `Toggle`
- `Slider`
- `Select`
- `CanvasView`
- `Image`
- `Rectangle`
- `Spacer`
- `Divider`
- `Window`
- `VStack`, `HStack`, `ZStack`
- `NavigationView`, `NavigationLink`
