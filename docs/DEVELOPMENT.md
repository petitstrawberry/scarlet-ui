# Development

Run these commands from the repository root. For feature selection and backend
boundaries, see the [platform guide](PLATFORM_GUIDE.md). Browser package commands
and requirements are in the [samples guide](../samples/README.md).

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
