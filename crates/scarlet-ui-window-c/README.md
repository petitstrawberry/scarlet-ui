# ScarletUI window C ABI

`libscarlet_ui_window.so` renders the standard ScarletUI `Window` frame for
applications with an external content renderer. The C interface is in
`include/scarlet_ui_window.h`; ABI version 1 uses physical pixel coordinates.

Create, configure, render, dispatch pointer input, and destroy a frame on the
same UI thread. Copy the returned premultiplied BGRA pixels and clip rectangles
before the next call. Configure uses the outer extent; the returned insets
identify the application content area. Undecorated frames have zero insets.

The host creates its platform window, processes the returned close/move/state
actions, and composes the frame above the external image. This library owns no
SWS connection, Vulkan resource, or event loop.

Build the native library with the Scarlet Rust toolchain:

```sh
cargo build --locked -p scarlet-ui-window-c --release --target aarch64-unknown-scarlet
```

Native consumers must use a toolchain with DSO-local Scarlet runtime startup
and TLS namespaces, as pinned by this repository's CI.
