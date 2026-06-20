# ScarletUI Preview for VS Code

ScarletUI Preview controls the `scarlet-ui-preview serve` IPC server from VS Code. It starts the long-running JSON-RPC server, opens native preview windows, switches preview sources, selects previews, rebuilds, and closes the preview window.

## Prerequisites

- A ScarletUI repository or crate checked out locally.
- `scarlet-ui-preview` available on `PATH` (default), or a custom `scarlet-ui.serverCommand` that starts the server.
- `cargo` and `rustc` available on `PATH` as well. Even when you use an installed `scarlet-ui-preview` binary, the preview host still shells out to `cargo build` to compile the preview dylib. On macOS, the extension also queries `rustc --print target-libdir` to mirror Cargo's dynamic library lookup for installed `scarlet-ui-preview` launches.
- A ScarletUI crate with preview functions marked by `#[scarlet_ui::preview]`.

### Nix note

If you use a Nix dev shell, start VS Code from inside `nix develop` so the extension host inherits the same `cargo`/`rustc` toolchain environment. Otherwise `scarlet-ui-preview` may fail when the extension queries `rustc --print target-libdir`, runs `cargo build`, or loads the resulting dylib.

## Configuration

| Setting | Default | Description |
| --- | --- | --- |
| `scarlet-ui.serverCommand` | `["scarlet-ui-preview", "serve"]` | Command array used to start the preview server. The extension appends `--manifest-path <targetManifest>`. Use an installed binary by default, or switch to `cargo run --manifest-path tools/preview/Cargo.toml -- serve` while developing the repo itself. For installed macOS binaries, `rustc` must be on `PATH` so the extension can resolve the Rust target lib directory. |
| `scarlet-ui.targetManifest` | `null` | Cargo manifest to preview. When unset, uses `Cargo.toml` in the first workspace folder. |
| `scarlet-ui.autoFollowActiveEditor` | `true` | When enabled, focusing a Rust file with `#[scarlet_ui::preview]` functions auto-switches the preview source via `preview/switchSource`. |

## Commands

- `ScarletUI: Start Preview Server`
- `ScarletUI: Preview Current File`
- `ScarletUI: Select Preview`
- `ScarletUI: Rebuild Preview`
- `ScarletUI: Close Preview Window`
- `ScarletUI: Stop Preview Server`

Rust functions annotated with `#[scarlet_ui::preview]` also get a `▶ Preview` CodeLens.

## Development Setup

```bash
npm install
npm run compile
```

Open `extensions/vscode-scarlet-ui` as a VS Code workspace and press F5 to launch an Extension Development Host.

## Usage

1. Open a ScarletUI crate in VS Code.
2. Run `ScarletUI: Start Preview Server`.
3. Open a Rust file containing a ScarletUI preview.
4. Run `ScarletUI: Preview Current File`, click the `▶ Preview` CodeLens, or run `ScarletUI: Select Preview`.
5. If the file contains multiple previews, the explicit preview command lets you pick one; auto-follow uses the first preview in the file.

The preview is a native winit window controlled through IPC; it is not embedded inside VS Code.
