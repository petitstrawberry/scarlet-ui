# Documentation

Start with the [project overview](../README.md) and the [getting started guide](https://ui.scarlet-os.org/getting-started/).
The repository files below are the documentation sources; the website imports the main guides during its build.

## Application and platform guides

- [Application guide](APPLICATION_GUIDE.md): scenes, windows, layout, input, and common views.
- [Platforms and backends](PLATFORM_GUIDE.md): feature selection, SGFX dependencies, crate layout, and platform integration.
- [Live preview](LIVE_PREVIEW.md): preview declarations, host commands, state ownership, and reload.
- [Preview server protocol](PREVIEW_PROTOCOL.md): editor integration and the complete Protocol v1 wire format.
- [Browser samples](../samples/README.md): WebGPU builds, browser requirements, and limitations.
- [Development](DEVELOPMENT.md): test, check, and smoke-test commands.

## Runtime contracts

- [Architecture](ARCHITECTURE.md): normative retained-runtime and reconciliation model.
- [Application and extension contract](1.0-contract.md): public API and Scene/Window lifetime boundaries.
- [Frame failure and recovery](FRAME_FAILURES.md): classified failures, safe discard, and recovery.

## Focused guides and implementation studies

- [Safe-area layout](SAFE_AREA.md)
- [Asynchronous file dialogs](FILE_DIALOGS.md)
- [Gamepad input](GAMEPAD_INPUT.md)
- [Scroll performance](SCROLL_PERFORMANCE.md)
- [Visual language](DESIGN_LANGUAGE.md)
- [Control study](CONTROL_STUDY.md)
- [Shared glass material trial](GLASS_MATERIAL.md)
- [Tabs and text study](TABS_AND_TEXT.md)

The visual studies describe their own trial status; consult the current implementation and recorded limitations alongside them.

- [Context menus and clipboard editing](CONTEXT_MENUS.md)
