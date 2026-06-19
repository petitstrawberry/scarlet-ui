# ScarletUI

ScarletUI is a cross-platform declarative UI framework for Rust applications.

It lets you write UI in a SwiftUI-like style while staying in ordinary Rust:
views are composed with macros and modifiers, state updates are synchronized with
the rendered UI, and view structure is represented by Rust types. That means the
compiler checks your UI code, and editors can still provide Rust-native
completion, navigation, and refactoring.

```rust
use scarlet_ui::prelude::*;
use scarlet_ui::{hstack, vstack};
use scarlet_ui_macros::View;

#[derive(View, Clone)]
struct CounterApp {
    count: State<i32>,
}

impl CounterApp {
    fn content(&self) -> impl View + Clone + 'static {
        vstack! {
            Text::new("Counter").font_size(24.0),
            Text::new(format!("Count: {}", self.count.get())),
            hstack! {
                Button::new("-").on_click({
                    let count = self.count.clone();
                    move || count.update(|value| *value -= 1)
                }),
                Button::new("+").on_click({
                    let count = self.count.clone();
                    move || count.update(|value| *value += 1)
                }),
            },
        }
        .spacing(12.0)
        .padding(16.0)
    }
}

impl Application for CounterApp {
    fn scenes(&self) -> impl Scene {
        WindowGroup::new("main", Window::new("Counter", self.content()))
    }
}

fn main() -> scarlet_ui::Result<()> {
    let mut app = CounterApp::default();
    app.run()
}
```

Application code declares state, views, and scenes as Rust types. ScarletUI uses
that structure to rebuild, lay out, paint, and present the UI through the
selected platform backend.

## Why ScarletUI

- **Declarative Rust UI**: build interfaces by composing `View` values instead of
  manually updating widgets.
- **State-driven rendering**: `State<T>` drives rebuild, layout, paint, and
  composite, so UI updates follow state changes.
- **Type-safe view structure**: the concrete view tree is built from Rust types,
  so composition errors surface at compile time.
- **IDE-friendly API**: because UI is written as Rust code, normal completion,
  go-to-definition, rename, and diagnostics continue to work.
- **Scene-based applications**: `Application::scenes()` declares top-level
  windows in the same declarative style as views.
- **Cross-platform backend model**: application code does not directly depend on
  SWS, winit, or another windowing backend.

## Application Model

`Application::scenes()` is the application UI entry point. `body()` is still used
by `#[derive(View)]` for reusable view components, but it is not the top-level
application entry point.

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

## Live Preview

ScarletUI preview is a native desktop development tool. It is separate from the
normal application runtime: application binaries still call `app.run()`, and
platform selection remains a build feature.

A preview crate marks one or more view functions with `#[scarlet_ui::preview]`.
The function can return a view fragment directly; a `Window` wrapper is only
needed when the preview wants to exercise window-specific behavior. Stateful
previews should return the state-owning view itself so its `listenables()` stay
mounted; do not return an already-expanded fragment that has read `State::get()`.

For binary crates, a simple `[lib] path = "src/main.rs"` target keeps preview
setup minimal.

```rust
use scarlet_ui::prelude::*;

#[scarlet_ui::preview(width = 420.0, height = 260.0)]
fn counter_preview() -> impl View + Clone + 'static {
    CounterApp::default().content()
}

#[scarlet_ui::preview]
fn button_preview() -> impl View + Clone + 'static {
    Button::new("OK")
}
```

Preview names are generated from function names, such as `counter_preview` to
`Counter Preview`. `#[scarlet_ui::preview(name = "...")]` is available only when
an explicit display name is useful.

Run the native preview host against that crate:

```bash
cargo run --manifest-path tools/preview/Cargo.toml -- \
  --manifest-path examples/preview-demo/Cargo.toml
```

Select a specific preview by generated name or stable ID:

```bash
cargo run --manifest-path tools/preview/Cargo.toml -- \
  --manifest-path examples/preview-demo/Cargo.toml \
  --preview "Button Preview"
```

Use `--build-only` to verify the preview dylib without opening a window:

```bash
cargo run --manifest-path tools/preview/Cargo.toml -- \
  --manifest-path examples/preview-demo/Cargo.toml \
  --build-only
```

The host rebuilds and reloads the preview dylib when Rust source or Cargo files
change. Preview dylibs use Rust ABI and must be built by the same toolchain as
the host.

## Preview Server (IPC)

For editor and IDE integration, `scarlet-ui-preview` also runs as a long-lived
process in `serve` mode. The editor drives it over stdio using newline-delimited
JSON-RPC 2.0: stdout carries protocol messages only, stderr carries
human-readable logs.

```bash
cargo run --manifest-path tools/preview/Cargo.toml -- \
  serve \
  --manifest-path examples/preview-demo/Cargo.toml \
  [--source <file.rs>] [--target <triple>] [--features <features>] \
  [--preview <id-or-name>] [--poll-ms <ms>]
```

`serve` accepts the same project options as the single-shot CLI except
`--build-only` (a single-shot-only flag that `serve` rejects).

### Protocol v1

Requests:

| Method | Params | Behavior |
|---|---|---|
| `initialize` | `{ client?, protocolVersion? }` | Capability handshake. Does not build. |
| `preview/list` | `{ source? }` | Build if needed, return available previews. |
| `preview/open` | `{ source?, preview? }` | Build and open a winit window; reload if already open. |
| `preview/switchSource` | `{ source, preview? }` | Switch to another file; rebuild and reload preserving the window. |
| `preview/switchPreview` | `{ preview }` | Switch to another preview in the same source (requires an open window). |
| `preview/rebuild` | `{}` | Rebuild and reload the current configuration. |
| `preview/closeWindow` | `{}` | Close the window; keep the server alive. |
| `shutdown` | `{}` | Respond, then exit. |

Server notifications:

| Method | Params | When |
|---|---|---|
| `preview/buildStarted` | `{ source }` | Before a cargo build starts. |
| `preview/buildFinished` | `{ previews[] }` | After a successful build. |
| `preview/reloadFinished` | `{ preview }` | After the host applied a reload or switch. |
| `preview/buildFailed` | `{ message }` | Build/load failed; the previous window is preserved. |
| `preview/windowClosed` | `{}` | The winit window closed (user close or `preview/closeWindow`). |

Client notification:

| Method | Params | Behavior |
|---|---|---|
| `workspace/didChangeActiveFile` | `{ path }` | Record the active editor file. No auto-follow by default. |

Error codes are standard JSON-RPC (`-32700` parse, `-32600` invalid request,
`-32601` method not found, `-32602` invalid params) plus
`-32001` build failed, `-32002` preview not found,
`-32003` preview library load failed, `-32004` preview host/window error.
On any build failure the server stays alive and the existing preview window is
preserved.

Minimal stdin example (the server runs until `shutdown` or stdin closes):

```text
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"preview/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"preview/open","params":{"preview":"Button Preview"}}
{"jsonrpc":"2.0","id":4,"method":"shutdown","params":{}}
```

The server-side source/Cargo mtime watch stays active after the first successful
build, so live reload works with or without an IDE driving the server.

## Platform Features

`scarlet-ui` currently selects one platform feature per build. The feature pulls
in the matching backend crate; `scarlet-ui-core` itself never depends on
`sws-client`, `sws-protocol`, `winit`, or `softbuffer`.

| Feature | Target | Notes |
|---------|--------|-------|
| `platform-sws` | Scarlet OS / SWS | Default platform feature. Pulls in `scarlet-ui-platform-sws`, which uses `sws-client` and `sws-protocol`. |
| `platform-winit` | native desktop | Pulls in `scarlet-ui-platform-winit`, which uses `winit` and `softbuffer`; requires `std`. |

The default feature set is:

```toml
default = ["std", "platform-sws"]
```

For native desktop builds, depend on ScarletUI with `platform-winit`:

```toml
[target.'cfg(not(target_os = "scarlet"))'.dependencies]
scarlet-ui = { path = "crates/scarlet-ui", default-features = false, features = ["std", "platform-winit"] }
```

For Scarlet OS `std`-compatible builds, use `platform-sws`:

```toml
[target.'cfg(target_os = "scarlet")'.dependencies]
scarlet-ui = { path = "crates/scarlet-ui", default-features = false, features = ["std", "platform-sws"] }
```

For no-std user binaries using Scarlet's legacy std shim, use:

```toml
scarlet-ui = { path = "crates/scarlet-ui", default-features = false, features = ["legacy-scarlet-std", "platform-sws"] }
```

`platform-sws` and `platform-winit` are mutually exclusive. `std` and
`legacy-scarlet-std` are also mutually exclusive.

## Crate Layout

```text
apps / user bins
  |
  v
scarlet-ui
  |-- re-exports scarlet-ui-core
  |-- ApplicationRunExt::run()
  |-- scarlet-ui-platform-sws    [feature: platform-sws]
  |   |-- scarlet-ui-core
  |   |-- sws-client
  |   `-- sws-protocol
  `-- scarlet-ui-platform-winit  [feature: platform-winit]
      |-- scarlet-ui-core
      |-- winit
      `-- softbuffer

scarlet-ui-core
  |-- View / Element / Scene / Pipeline
  |-- rendering and preview core
  |-- PlatformBackend trait
  |-- PlatformWindow trait
  `-- WindowCreateRequest
```

Dependency direction is intentionally one-way:

```text
scarlet-ui-platform-sws  ----.
                             v
apps --> scarlet-ui ---> scarlet-ui-core
                             ^
scarlet-ui-platform-winit ---'
```

`scarlet-ui-core` owns the UI implementation and backend traits. Platform crates
only implement those traits. The `scarlet-ui` facade preserves the app-facing
`scarlet_ui` crate name and selects the backend crate from features.

## Platform Integration

Platform code lives behind `PlatformBackend` and `PlatformWindow`. These traits
are public so backend crates can implement them, but they are runner boundaries,
not normal application APIs. They create windows, poll events, present buffers,
route text input, and handle window controls.

Most applications should not construct a `PlatformWindow` directly. Use
`Window`, `WindowGroup`, `open_window`, `dismiss_window`, and `app.run()`.

SWS-specific applications may use lifecycle hooks and downcast the
`dyn PlatformWindow` only when they intentionally depend on SWS-specific
capabilities.

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

## Development Commands

Run facade tests:

```bash
cargo test -p scarlet-ui --lib --tests
```

Run core tests and doctests:

```bash
cargo test -p scarlet-ui-core
```

Check native desktop support and preview tooling:

```bash
cargo check -p scarlet-ui --no-default-features --features std,platform-winit,preview
cargo check -p scarlet-ui-platform-winit
cargo check -p scarlet-ui-preview
```

Run the preview demo:

```bash
cargo run -p scarlet-ui-preview -- \
  --manifest-path examples/preview-demo/Cargo.toml \
  --preview "Counter Preview"
```

Smoke-test the IPC server without an IDE (initialize, list previews, shut down):

```bash
printf '%s\n%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","id":2,"method":"preview/list","params":{}}' \
  '{"jsonrpc":"2.0","id":3,"method":"shutdown","params":{}}' \
  | cargo run -p scarlet-ui-preview -- serve \
    --manifest-path examples/preview-demo/Cargo.toml
```
