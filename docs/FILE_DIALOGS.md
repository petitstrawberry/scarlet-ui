# Asynchronous file dialogs

`scarlet_ui::file_dialog` provides `FileDialog`, `FileDialogMode::{Open,
OpenMultiple, Save}`, extension filters, and a result receipt. This is additive
to the 1.0 contract: existing exhaustive `Event`, `ApplicationCommand` and
`Error` enums are unchanged. `PlatformWindow::begin_file_dialog` has an
unsupported default, so existing custom backends continue to compile.

Obtain the **runtime** `WindowContext.window_id` in `on_window_created`; a scene
key or native surface ID is not an owner. Keep that ID with the window's model.

```rust,ignore
use scarlet_ui::file_dialog::*;
let mut options = FileDialog::new(FileDialogMode::OpenMultiple);
options.title = "Import audio".into();
options.initial_directory = Some(std::env::current_dir()?.into());
options.filters = vec![FileDialogFilter {
    name: "WAV audio".into(), extensions: vec!["wav".into()],
}];
let receipt = options.show(context.window_id);
// In on_idle, rather than a blocking wait or worker-thread AppKit call:
if let Some(result) = receipt.take_result() {
    match result {
        Ok(FileDialogOutcome::Selected(paths)) => { /* start application I/O */ }
        Ok(FileDialogOutcome::Cancelled) => { /* keep the document unchanged */ }
        Err(FileDialogError::Unsupported(reason)) => { /* offer in-app picker */ }
        Err(error) => { /* display failure; allow a new request */ }
        _ => {}
    }
}
```

Options use absolute initial directories, literal extensions without dots or
wildcards, and a default basename (no directory components). Filters form a
union; separate named filter groups are not a promised chooser UI. Native
selection does not replace file-format validation or application I/O errors.
`filter_policy` defaults to `FileDialogFilterPolicy::Required`. Set it to
`Optional` to allow a backend without arbitrary extension filters to show an
unfiltered native chooser, then validate the returned path in your application.
No file is opened or written by this API. `FileDialogPath` converts to/from
`PathBuf` in std builds without losing non-UTF8 filenames; the legacy Scarlet
runtime carries UTF-8 paths. Save overwrite confirmation/default-extension
behavior belongs to the backend.

## Lifecycle and threading

The runner processes queued requests on its UI thread. Start/poll/cancel/drop
of backend sessions run there, and `poll` must not block. One active request is
allowed per runtime owner; another returns `Busy`. Other owners remain
independent. Invalid options complete immediately. A stale/closed owner or
runner shutdown completes with `OwnerClosed`; closing an owner detaches its
session before closing the platform window. Results are published once and
consumed once across all receipt clones. `cancel()` is deferred to the runner;
before receipt completion it wins over a late selection. Dropping a receipt
alone does not cancel. As with scene commands, concurrent independent runners
are unsupported.

After completion/cancellation the runner requests focus on the surviving
owner. Focus remains subject to the window manager. Applications should freeze
operations which could invalidate a pending request, clear continuations on
cancel/error, and restore their own text/keyboard focus as needed. There is no
new application-event enum variant or synchronous callback into user code.

## Backends

| Backend | Single open/save | Multiple open | Extension filters | Ownership/cancel |
| --- | --- | --- | --- | --- |
| macOS/winit | NSOpenPanel/NSSavePanel | Yes | AppKit union | Owner NSWindow sheet, native cancel |
| Scarlet/SWS | Existing Files service | Unsupported | Required: unsupported; Optional: unfiltered | Request ID correlation; local abandon only |
| Linux/winit, other/custom defaults | Unsupported | Unsupported | Unsupported | In-app fallback supplied by application |

macOS uses the winit NSView's actual NSWindow, verifies the main thread, rejects
an already-attached sheet, and uses `beginSheetModalForWindow:completionHandler:`.
It does not use `runModal` or stop the application event loop. The completion
block has weak panel ownership to avoid a retain cycle. URL filesystem
representations preserve native path bytes. AppKit's extension API in the
existing objc2 0.2 bindings is deprecated on newer macOS but remains the native
extension-based route; it can later be migrated to UTType without changing the
common contract.

Scarlet uses the **existing sbus Files service**, not new SWS messages:

- Existing fixed Scarlet revision: `b3d2a55740a3d2ca49daad0ec7baba233f706f7a`.
- Constants from `scarlet-desktop-config`; transport from `sbus-client`.
- `OpenFile(title, initial_folder, filter, allow_multiple, select_directories)`.
- `SaveFile(title, initial_folder, suggested_name, filter)`.
- Immediate `[String request_id]`; later `Response` signal
  `[String request_id, Boolean success, String path]`.
- Match sender, object path, interface, signal name **and request ID**.

One detached worker owns one connection for the call and subsequent signals;
the sbus client's pending queue preserves a Response arriving before the method
reply. The UI thread never waits for IPC. The initial call has a 2-second bound;
ambiguous failures are not automatically resent. A definite `ServiceNotFound`
activates Files through stemd `LaunchOrFocus(DESKTOP_FILES_APP_ID)` and retries
only while the service remains absent, for up to 3 seconds. Response polling is bounded
at 100 ms with a 10-minute overall deadline. Missing/malformed responses and
transport/launch failures are errors, not user cancellation.

The provider accepts `allow_multiple` but returns one selected path; arbitrary
extensions and audio MIME filters are not enforced. Multiple-selection requests and `Required` filter requests return `Unsupported`
before IPC. `Optional` single/open/save requests show Files without a filter.
Applications must validate their chosen extension before starting file I/O. The
protocol has no caller-window identity or remote Cancel. For single/save requests, `cancel()` abandons the receipt and worker but the remote
picker can remain open; late signals cannot start application I/O. This is an
independent desktop window, not an owner-attached modal sheet. It cannot promise
native multiwindow ownership, remote closure, or save overwrite confirmation.

Evidence: Scarlet `user/std-bin/src/filer.rs` (provider and `matches_filter`),
`user/std-bin/src/notepad.rs` (open/save client),
`user/lib/scarlet-desktop-config/src/lib.rs` (constants), and
`user/lib/sbus-client/src/lib.rs` (bounded receive/pending messages).
The current Scarlet dev snapshot `0639a916dfd652e9b2c1ea740cacc1c09743d9eb`
has the same capability limits. No Scarlet protocol or dependency rev change
is required here.

Linux deliberately uses the unsupported default in this change. It does not
claim GTK/portal/native support. An implementation can override the same
platform method later without altering application calls.

## Verification

Core receipt and runner tests cover selection exactly once, cancellation vs
late replies, platform failure, owner teardown, invalid options, two runtime
owners, per-owner Busy, stale IDs, cancellation before start, and focus request
on the completing owner. macOS and Scarlet std/legacy builds check the native
and IPC bindings. The Mac GUI/Scarlet desktop remain separate runtime checks;
compilation and deterministic backend tests do not certify them.
