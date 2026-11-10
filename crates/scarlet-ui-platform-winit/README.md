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

### macOS unified titlebar

`Window::decoration(WindowDecoration::SYSTEM_OVERLAY)` keeps the native traffic
lights and system frame, hides the native title text, and lets application
content fill the titlebar. The window title remains available to accessibility
and window menus. Ordinary `SYSTEM` decorations are unchanged.

The app must reserve its own title strip (at least 28 logical pixels) and keep
interactive content clear of the traffic lights; a 32px strip with a 90px
leading reservation is suitable for a compact toolbar. Initiate dragging only
from that strip via `PlatformWindow::request_move`. Do not make the entire
editor/content view draggable. ScarletUI does not add custom titlebar height or
custom window buttons in overlay mode.

This option requires a system frame and is currently supported only by macOS
Winit. Other Winit hosts return `WindowDecorationUnsupported`; applications
should select normal platform decorations there.

## Native text clipboard

`PlatformWindow::text_clipboard()` returns a cloneable asynchronous service backed
by arboard, with text-only support on macOS, Windows, X11, and Wayland compositors
supporting a data-control protocol. Arboard falls back to X11 when Wayland data
control is unavailable; a session without either reports an unavailable error.
No process-local clipboard is substituted. Images and rich text are not exposed.

Access requires a live, focused owner window. Focus changes invalidate in-flight
results, and closing the window revokes all retained service clones. Calls are
revalidated on the native worker immediately before OS access. Text is limited to
256 KiB (UTF-8 bytes); native reads are checked after the OS returns the text.

One process-wide worker retains native clipboard ownership, including X11 text
when a window closes. Callers wait at most two seconds on their background task,
never on the event loop. If an OS call stalls, further requests return `Busy`
until it finishes rather than creating additional native threads or queued
writes. A write already submitted to the OS cannot be cancelled or rolled back,
even if the caller times out or focus changes. Linux clipboard persistence after
process exit depends on a desktop clipboard manager.

Run the headless authorization and worker-limit tests with:

```sh
cargo test -p scarlet-ui-platform-winit --lib clipboard::tests
```

For desktop verification, copy Unicode and multiline text to/from another native
application, test cut/paste across two ScarletUI windows, and verify that a saved
service clone is denied after its owner loses focus or closes. Check empty and
non-text clipboards, oversized text, and sessions without a clipboard server.
