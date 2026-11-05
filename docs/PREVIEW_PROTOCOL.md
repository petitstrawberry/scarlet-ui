# Preview Server Protocol

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
