# Software-WebGPU CI smoke

This bounded smoke test serves a previously built `target/web-samples` artifact
on an ephemeral `127.0.0.1` port. It does not open the public Pages site.

```sh
cd tools/web-smoke
npm ci --ignore-scripts
test -x /opt/google/chrome/chrome
google-chrome --version
npm test
npm run smoke
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

Chromium is launched with `chromiumSandbox: true`, never `--no-sandbox`.
`chrome://sandbox` must confirm namespace and seccomp-BPF sandbox layers; missing
sandbox support is a failure, not a reason to alter runner sysctls/AppArmor or
turn off the sandbox. SwiftShader uses the specifically approved unsafe-WebGPU
and Vulkan-compositing flags. The GPU subprocess may be unsandboxed; this
exception does **not** authorize disabling the browser/renderer sandbox.

## Assertions and evidence

- Real WebGPU adapter and device acquisition must succeed; no skip on failure.
- Counter title/count pixels must render. Two actual clicks must change the
  count glyph pixels while retaining the title.
- Factory sidebar renders; Controls navigation exposes a red toggle. A click
  removes its red pixels, and a shared View menu action restores them.
- Resize retains toggle state, and another click uses its new rendered position.
- No page error, console error, crash or public `runtime_error()` is accepted.
- `artifacts/` always contains `report.json`; successful/failed frame captures,
  sandbox diagnostics and screenshots are also retained when available.

These are pixel/state-transition assertions, not OCR or a full visual snapshot
suite. They establish software WebGPU rendering and interaction, not exact text
recognition, hardware-GPU coverage, performance or all browser compatibility.
Coordinates derive from the checked-in example layout at DPR 1. View changes
that intentionally move widgets should update these smoke assertions.
