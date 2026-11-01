---
title: Getting started
---

ScarletUI is a declarative UI toolkit for Rust. Applications describe views,
state and scenes; the selected platform backend connects them to a window.
Start with the native Widget Factory to explore the existing controls and layouts.

## Run the native example

Clone the repository and run Widget Factory from its root:

```sh
git clone https://github.com/petitstrawberry/scarlet-ui.git
cd scarlet-ui
cargo run --locked -p scarlet-ui-widget-factory
```

Use a Rust toolchain supporting edition 2024. The example selects the winit
backend on native desktop targets and SWS on Scarlet. Native Linux builds
also require the platform development libraries; the repository's CI installs
`libfontconfig-dev` and `pkg-config` for font discovery.

To build the desktop facade without launching a window:

```sh
cargo build --locked -p scarlet-ui --no-default-features --features std,platform-winit
```

These commands build native code. They do not select the WebAssembly backend.
For platform feature selection, application examples and development commands,
see the [project reference](../reference/project/).

## Understand the application model

- `View` values describe UI structure; `State` handles notify the retained tree
  when application state changes.
- `Application::scenes()` declares windows. Platform features select the backend
  rather than introducing a separate application model.
- The [architecture reference](../reference/architecture/) describes view,
  element and render-object lifetimes.
- The [application and extension contract](../reference/application-contract/)
  documents public API boundaries and platform responsibilities.

The reference pages are imported from the repository during the site build, so
the original documentation remains the source of truth.

## Try the browser examples

The browser version runs the existing widgets through WebGPU. It requires an
available WebGPU adapter; it is not required to read this documentation or use
the native toolkit. The [browser build and limitations guide](../reference/browser-samples/)
contains the commands, compatibility notes and input limitations.

For the executable example itself, see
[Widget Factory source](https://github.com/petitstrawberry/scarlet-ui/tree/main/examples/widget-factory).
