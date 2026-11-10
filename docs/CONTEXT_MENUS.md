# Context menus

Any view can provide a right-click menu using the existing `Menu` command model:

```rust
use scarlet_ui_core::prelude::*;
use scarlet_ui_core::views::{Menu, MenuItemContent};

let view = Text::new("Right-click me").context_menu(Menu::new(vec![
    MenuItemContent::new("Open").callback(|| println!("Open")),
    MenuItemContent::separator(),
    MenuItemContent::new("Unavailable").enabled(false),
]));
```

The modifier does not change layout or keyboard focus. Menus are painted at the
window level, outside scroll clips, and positioned inside the window when they
fit. The closest explicit provider on the hit path wins, including over a text
editor's built-in edit menu. Menu contents are a snapshot when opened; rebuilding
the view updates the menu used on the next opening.

Click and release a command to activate it. Up/Down skip disabled commands and
separators; Enter activates the highlighted command. Escape, Tab, an outside
click, scrolling, window resizing or focus loss dismiss the popup. An outside
click is consumed rather than activating content behind it. Menu shortcuts are
labels, not newly registered global keyboard shortcuts. Hierarchical submenus
and touch long-press invocation are not supported by this context-menu API.

Custom render objects can implement `ElementRenderObject::context_menu` to build
commands from their latest state. Advanced hosts can also call
`RenderingPipeline::open_context_menu_at(menu, position)` with window-local
logical coordinates. Menus taller than the window are clipped by the window;
keep command lists short.

## Standard text editing and the platform clipboard

`ApplicationRunner` automatically connects `TextField` and `TextView` to the
window's `TextClipboard` on winit and SWS. No callback setup is needed. Ctrl+C,
Ctrl+X and Ctrl+V (Command on macOS) and the built-in right-click Cut/Copy/Paste/
Select All menu use the same editing path. A right-click preserves the current
selection. Explicit `TextView::on_copy` and `on_paste` callbacks still take
precedence. TextField pastes retain its existing single-line normalization;
TextView preserves newlines.

Hosts with their own undo or asynchronous edit transactions can use
`TextView::on_edit_action` to handle Copy, Cut, Paste and Select All from both
shortcuts and the standard context menu. This overrides `on_copy`/`on_paste`;
ScarletUI leaves clipboard access and all text mutations to the callback.

Clipboard access is asynchronous and limited to 256 KiB of UTF-8 text. Cut
removes the selected text only after the clipboard write succeeds. Pending cuts
and pastes are discarded when the editor is replaced, disabled, loses focus,
starts composing, or changes its document/selection. Intervening editing input
also invalidates a pending transaction, even if selection later returns to its
original position. Clipboard writes already sent to the OS cannot be undone.
Repeated operations while one is pending report `ClipboardError::Busy`.

Override `Application::on_clipboard_error` to show access-denied, size-limit,
timeout or unavailable errors in your application. The default ignores the
error and leaves text intact. Native access requires a focused, live window.
The winit backend's [clipboard notes](../crates/scarlet-ui-platform-winit/README.md)
describe OS support and manual verification.

Custom runners install `RenderingPipeline::set_text_clipboard` and call
`poll_text_clipboard` from idle ticks, then retrieve errors with
`take_text_clipboard_error`. These APIs do not substitute an in-memory clipboard
when the system service is unavailable.

## Browser text editing

Custom context menus also work on the Web canvas and take keyboard ownership
while open. The canvas prevents the browser context menu only when a custom
popup actually opens. Native desktop editors provide Cut/Copy/Paste menus; Web
editors retain browser keyboard clipboard editing instead. The hidden textarea
is not a pointer target and does not provide a native right-click edit menu.
