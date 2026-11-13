# Software-WebGPU CI smoke

The separate `npm run test:host` check uses Playwright's headless Chromium
(`npx playwright install chromium --only-shell`) to test the Widget Factory
host page without Wasm or WebGPU. It compares canvas pixels and geometry with
and without the standalone status-background element, including rotation,
appearance and input. It cannot validate iOS's native status-bar blur. It does
not change the Chrome/sandbox requirements of the GPU smoke below.

This bounded smoke test serves a previously built `target/web-samples` artifact
on an ephemeral `127.0.0.1` port. It does not open the public Pages site.
Animation Lab's source-reader check opens the rendered code button, switches to
the full Rust file, copies it, and returns through the header. Clipboard read/write
permissions are granted only to that ephemeral localhost context for this check,
then cleared; the copied value is the bundled public example source.

```sh
cd tools/web-smoke
npm ci --ignore-scripts
test -x /opt/google/chrome/chrome
google-chrome --version
npm test
xvfb-run --auto-servernum npm run smoke
```

Optional arguments: `node smoke.mjs /absolute/site /absolute/artifacts`.
Node 22+ and a Linux runner with Google Chrome stable already installed at its
standard package path are required. The Ubuntu runner supplies the Chrome
package and its existing AppArmor policy; this test neither installs a browser
nor modifies that policy. The Playwright client is pinned; the runner Chrome
version is logged by CI. Dependency setup network access is separate from runtime.

## Isolation

Run in a dedicated ephemeral job, without repository secrets or write
permissions, with checkout `persist-credentials: false`. Browser child processes
receive only an explicit environment allowlist, not GitHub Actions tokens or
arbitrary workflow variables. Pages, workers and WebSockets must not contact
external services: context routing allows only the exact local HTTP origin and
service workers are blocked. Do not reuse this profile for everyday browsing.

Chrome runs in headed mode under the runner-provided Xvfb display, with
`chromiumSandbox: true`, never `--no-sandbox`. There is no automatic
fallback or sandbox-policy change.
`chrome://sandbox` must confirm namespace and seccomp-BPF sandbox layers; missing
sandbox support is a failure, not a reason to alter runner sysctls/AppArmor or
turn off the sandbox. SwiftShader uses the specifically approved unsafe-WebGPU
and Vulkan-compositing flags. The GPU subprocess may be unsandboxed; this
exception does **not** authorize disabling the browser/renderer sandbox.

The software backend is selected explicitly for all three consumers:
ANGLE (`--use-angle=swiftshader`), Chromium's Vulkan compositor
(`--use-vulkan=swiftshader`) and Dawn WebGPU
(`--use-webgpu-adapter=swiftshader`). Selecting ANGLE alone left the runner's
Vulkan compositor unable to initialize (`vkCreateInstance failed: -9`), followed
by a missing shared-image backing factory and device loss during canvas
presentation. The standalone mapped-buffer allocation itself succeeded.
These driver-selection flags do not change sandbox settings. See Chromium's
[SwiftShader driver documentation](https://chromium.googlesource.com/chromium/src/+/main/docs/gpu/swiftshader.md)
and [Dawn adapter-selection explanation](https://chromium.googlesource.com/chromium/src/gpu/+/963206a961a7992b048d07f756f87f11fa2e5420).
The report records actual WebGPU adapter information as well as compositor data.

## Assertions and evidence

- Real WebGPU adapter and device acquisition must succeed; no skip on failure.
- A mapped 4096-byte buffer must allocate, and a WebGPU clear-color canvas must
  actually appear in a screenshot before any ScarletUI app starts.
- Counter title/count pixels must render against a white background (black or
  blank frames do not pass). Two actual clicks must change the
  count glyph pixels while retaining the title.
- Animation Lab must present colored artwork that changes over time. Its public
  `pause()` export must freeze the presented frame; `resume()` must restore motion.
  The canvas must remain inside a narrow viewport, without a runtime error,
  and `stop()` must complete. These checks use no fixed artwork snapshots or
  tab/control coordinates; they do not assert every Lab interaction.
- Factory sidebar renders; Controls navigation exposes a red toggle. A click
  removes its red pixels, and a shared View menu action restores them.
- Resize retains toggle state, and another click uses its new rendered position.
- No page error, console error, crash or public `runtime_error()` is accepted.
- `artifacts/` always contains `report.json`; successful/failed frame captures,
  sandbox diagnostics and screenshots are also retained when available. Timestamped
  phases and full error stacks distinguish primary failures from teardown errors.

These are pixel/state-transition assertions, not OCR or a full visual snapshot
suite. They establish software WebGPU rendering and interaction, not exact text
recognition, hardware-GPU coverage, performance or all browser compatibility.
Coordinates derive from the checked-in example layout at DPR 1. View changes
that intentionally move widgets should update these smoke assertions.
