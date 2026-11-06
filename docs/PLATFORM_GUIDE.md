# Platform and backend guide

Choose build features and understand the renderer and backend boundaries.
Run commands from the repository root.

## Platform Features

Applications explicitly select one platform feature per build. The feature pulls
in the matching backend crate; `scarlet-ui-core` itself never depends on
`sws-client`, `sws-protocol`, `winit`, or `softbuffer`.

| Feature | Target | Notes |
|---------|--------|-------|
| `platform-sws` | Scarlet OS / SWS | Pulls in `scarlet-ui-platform-sws`, which uses `sws-client` and `sws-protocol`. |
| `platform-winit` | native desktop | Pulls in `scarlet-ui-platform-winit`, which uses `winit` and `softbuffer`; requires `std`. |

The default feature set enables the portable API without selecting a platform:

```toml
default = ["std"]
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

### SGFX dependency

The SGFX renderer and platform backends use the standalone
[SGFX repository](https://github.com/petitstrawberry/sgfx). Applications that also
use SGFX directly should depend on that repository:

```toml
[dependencies]
sgfx = { git = "https://github.com/petitstrawberry/sgfx" }
```

`Cargo.lock` records the resolved commits; no `rev` pin or compatibility patch
for the old Scarlet SGFX source is needed.

When migrating an existing lockfile, also update the external Adreno backend
so it no longer imports SGFX core from Scarlet:

```bash
cargo update -p sgfx-backend-scarlet-adreno
cargo tree --locked --invert sgfx-core
```

The second command fails if the graph still contains multiple SGFX core
sources. CI repeats this check for the host and both Scarlet targets.

SGFX is the internal execution IR used by this renderer. Its Rust interfaces
are updated and rebuilt with the consuming renderer/platform adapters using a
compatible source and lockfile set; SGFX does not promise a separate 1.x Rust
API freeze. See the [SGFX execution contract](https://github.com/petitstrawberry/sgfx/blob/main/docs/1.0-contract.md).
ScarletUI's application-facing API and frame-failure contract remain separate.
The direct SGFX renderer can remain in use, while a future Vulkan renderer can
use the application's Vulkan graphics boundary instead.

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

## Browser Samples

The experimental WebGPU backend runs the counter and the shared Widget Factory
in a browser, including the SGFX canvas and shared menus. See the
[Samples build and run guide](../samples/README.md) for a release-mode static
package, browser requirements and current limitations. The existing native
Widget Factory uses the same view definitions.
