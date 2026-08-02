# scarlet-ui-platform-sws

SWS backend for ScarletUI.

```text
scarlet-ui-platform-sws
  |-- scarlet-ui-core
  |-- scarlet-ui-renderer-sgfx
  |-- sws-client
  `-- sws-protocol
```

This crate implements `PlatformBackend` and `PlatformWindow` from
`scarlet-ui-core` for Scarlet Window Server. Applications normally enable it
through the `scarlet-ui` facade:

```toml
scarlet-ui = { path = "../lib/scarlet-ui", default-features = false, features = ["legacy-scarlet-std", "platform-sws"] }
```

## Renderer selection

`SCARLET_UI_BACKEND` selects the ScarletUI paint backend for each SWS window:

- `auto` (and an unset variable) negotiates SGFX and falls back to CPU only if
  SGFX initialization is unavailable.
- `cpu` always uses the CPU paint-command renderer.
- `sgfx` requires native SGFX rendering; negotiation or initialization failure
  is returned to the application instead of silently changing renderer.

Any other value is an error. The SWS compositor itself is selected separately
by `SWS_BACKEND=auto|cpu|sgfx`. ScarletUI reports both the renderer it actually
initialized and the compositor backend returned by SWS capability negotiation.

All windows created by one `SwsBackend` share one multiplexed SWS connection.
Window input and SGFX buffer lifecycle events use independent subscriptions, so
synchronous replies and concurrent waiters cannot consume each other's frames.

During Scarlet integration, build only from the Scarlet repository root so its
workspace-level local patches are applied:

```bash
cargo scarlet image --project ./projects/aarch64-limine-full --release
```
