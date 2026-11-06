# Live preview

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
    CounterApp::default()
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

## Editor integration

For editor-controlled previews, see the [preview server protocol](PREVIEW_PROTOCOL.md)
and the [VS Code extension](../extensions/vscode-scarlet-ui/README.md).
