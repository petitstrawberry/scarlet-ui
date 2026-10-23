# scarlet-ui-platform-winit

Native desktop backend for ScarletUI preview and host development.

```text
scarlet-ui-platform-winit
  |-- scarlet-ui-core
  |-- winit
  `-- softbuffer
```

This crate implements `PlatformBackend` and `PlatformWindow` from
`scarlet-ui-core` for desktop windows. Applications normally enable it through
the `scarlet-ui` facade:

```toml
scarlet-ui = { path = "../lib/scarlet-ui", default-features = false, features = ["std", "platform-winit"] }
```

Useful check:

```bash
cargo check
```

On macOS, window `MenuBarModel`s are published to the native menu bar. The
focused window owns the menu; titles, enabled state, nested items, separators
and `MenuItemModel::checked(bool)` update when its model changes. `app()` entries
are inserted into the standard application menu, preserving About/Services/Hide.
Callbacks use the existing window-scoped registry and detached execution model.
Retired menus and menus belonging to closed/background windows cannot activate.

App-defined shortcuts are **display hints**, as on Scarlet: their keyboard events
continue through winit and ScarletUI's focused input dispatch. Applications must
keep their existing shortcut handlers. This avoids a native menu Cut/Select All/
Undo action modifying the document while a search `TextField` has focus, and
avoids dispatching an action twice. Single-stroke Cmd/Primary, Ctrl, Option/Alt,
Shift, function and navigation keys have native labels. Multi-stroke chords are
left to the application and have no native key equivalent.

The standard Quit item uses a model's Cmd+Q callback when supplied. Otherwise it
queues normal close requests for the application's windows, allowing
`Application::on_window_close_requested` to veto closing unsaved documents.
It does not call AppKit's unconditional `terminate:`. Standard Hide shortcuts
continue to use AppKit. File-dialog sheets do not activate their parent's menus.
Other platform backends are unchanged.

Interactive regression probe (only counters and temporary text, no file I/O):

```bash
cargo run -p scarlet-ui --example native-menu-smoke --features platform-winit
```

File → Open Other Window verifies ownership. Native menu clicks increment the
native counter; Cmd+I, Shift+Cmd+I, F12 and Ctrl+J increment only the keyboard
counter. Edit shortcuts must operate on the focused field without incrementing
the native counter. View → Enable Action updates checked/enabled state. Quit is
vetoed until “Allow closing probe” is clicked, and increments the close-request
counter instead. The dark input also exercises `TextField::placeholder_color`.
