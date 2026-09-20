# Gamepad input

The SWS backend selects standard menu navigation for ordinary ScarletUI
windows when SWS advertises `GAMEPAD_INPUT`. Raw snapshots are opt-in, so a
confirm button does not arrive as both a keyboard action and gamepad event.
Menu navigation follows SWS distribution configuration. Other backends do not
yet provide physical gamepad acquisition.

`Event::Gamepad(GamepadEvent)` routes through capture, target, and bubble to
the focused element, falling back to the window root. Unconsumed events reach
`Application::on_gamepad(&WindowContext, GamepadEvent)`. `GamepadButton` names
physical positions rather than Nintendo/Xbox labels. X/Y sticks use
−32767…32767, positive Y down; triggers use 0…32767 and hats −1…1.
`device_id` identifies one reader instance, including across `/dev/gamepadN`
reconnects; `time_ns` is monotonic nanoseconds, and `reset`
invalidates cached state. Each event is a complete snapshot rather than a
button transition. Detect presses by comparing with the previous snapshot.

Games can receive native buttons and axes without menu-key conversion:

```rust
use scarlet_ui::{GamepadButton, GamepadEvent, PlatformWindow, WindowContext};

// Inside an Application implementation:
fn on_window_created(&mut self, _ctx: &WindowContext, window: &mut dyn PlatformWindow) {
    window.set_gamepad_input(true, false).expect("select gamepad policy");
}

fn on_gamepad(&mut self, _ctx: &WindowContext, event: GamepadEvent) {
    if event.reset {
        // Clear cached buttons, axes, and any ongoing action for this device.
        return;
    }
    if event.pressed(GamepadButton::East) {
        // Nintendo A, identified by its physical position.
    }
}
```

HOME remains a SWS system action. Focus loss, unsubscription and dropped input
produce reset snapshots. Applications must also clear input on window teardown
or transport failure. `InputEnvironment::has_gamepad()` reports availability
independently of `has_keyboard()`.

This adds the exhaustive `Event::Gamepad` variant and the public
`InputEnvironment::gamepad` field to the coordinated, unreleased API baseline.
Exhaustive event matches require a new arm; direct input-environment struct
literals require the new field. `InputEnvironment::new` retains its signature
and starts with `gamepad = false`; `.with_gamepad(true)` adds availability.
The platform trait method has a default implementation for existing backends.
