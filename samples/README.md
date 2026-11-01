# Browser Samples

The static Samples landing launches the existing Widget Factory and Counter
WebGPU examples. It does not replace their ScarletUI views with HTML widgets.
The landing is responsive HTML; each app retains its full-window canvas.
Existing `docs/` content is unchanged.

## Build

Python 3.11+, Rust with edition-2024 support, and the matching wasm-bindgen CLI
are required. From the repository root:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
python3 tools/build-web-samples.py
python3 tools/build-web-samples.py --check target/web-samples
python3 -m http.server 8765 --bind 127.0.0.1 --directory target/web-samples
```

Open `http://localhost:8765/`. The default build uses `--release --locked`,
checks the CLI version against Cargo.lock, generates JavaScript/WASM, copies
only static assets and records file hashes in `build-manifest.json`.
`CARGO_TARGET_DIR` is honored for Rust build artifacts. Use `--profile dev` for
local iteration or `--output PATH` for a different static output directory.
An existing output is replaced only if it carries this script's generated
marker. Generated artifacts stay under ignored `target/` by default.

Routes are relative: `./widget-factory/` and `./counter/`. The output can live
at a site root or under a project prefix such as `/scarlet-ui/`, without
rewriting its URLs. Serve `.wasm` as `application/wasm`, JavaScript as a
JavaScript MIME type, and use HTTPS except on localhost. No credentials,
backend service, browser flags, CDN scripts or cross-origin isolation headers
are required by the page. An available WebGPU adapter is required to run apps.

## GitHub Pages

The deployable payload is the **contents** of `target/web-samples/`, not the
repository or `docs/`. Configure the repository's Pages source as **GitHub
Actions** and use the separately scoped Samples workflow to build/upload this
artifact and deploy it through GitHub Pages. A deployment requires the relevant
Pages settings/environment permissions; building or opening a PR does not by
itself make the site live. Do not replace an existing documentation deployment
without deciding how its routes will coexist.

The package uses no root-absolute asset paths, so a project Pages address under
`/scarlet-ui/` works unchanged. `.nojekyll` is included. Verify the published HTTPS
landing, both sample routes, and JavaScript/WASM content types after deployment.
No browser security flag is needed or recommended for visiting the page; if an
adapter is unavailable, the sample reports that limitation instead.

`build-manifest.json` records the checkout commit/dirty state when Git metadata
is present. Both fields are `null` for a source archive or unavailable Git.

## Static checks

```sh
python3 tools/test_web_samples.py
node --test samples/capability.test.cjs
```

These tests exercise package-path validation and capability/error states without
a browser or GPU. They are not render tests.

## What to verify

- Landing at desktop and narrow mobile widths; keyboard focus and launch links.
- Capability check with an available adapter, no adapter, missing WebGPU API,
  insecure context and JavaScript disabled. A positive adapter check does not
  claim hardware acceleration or guarantee later device initialization.
- Launch each app from a project-prefix path, check relative JS/WASM requests
  and startup error messages, then use browser Back to return to the samples.
- In a compatible environment: Counter clicks and Widget Factory controls,
  menu actions, resize and DPR changes. Compiling a package alone does not
  establish successful GPU execution.

Canvas accessibility, IME composition, clipboard integration and other backend
limits are documented in the individual example READMEs. The landing remains
usable without WebGPU and explains unsupported configurations. It does not
recommend weakening browser security settings.

## CI boundaries

- macOS/Windows: explicit native/common crates (core, scroll motion, icons,
  macros, facade, winit, both renderers and Widget Factory), including optional
  core preview APIs, plus preview-macro
  doctests. These jobs do not select the web platform or counter packages.
- Linux host: existing full workspace suite except target-specific SWS,
  including preview tooling. The preview demo dylib currently exceeds MSVC's
  export limit; Windows native preview support is not claimed.
- Web: the platform's 20 pure menu/damage tests run on a Linux host, followed by
  counter/Factory `wasm32-unknown-unknown` builds. Pure logic tests are not browser
  tests. Static packaging has its own path/capability validation.
- The separate isolated software-WebGPU job renders and clicks actual canvas
  pixels. See `tools/web-smoke/README.md` for its scoped security conditions and
  fail-closed sandbox/adapter assertions; it is not a hardware-GPU benchmark.

This separation is deliberate: sharing a workspace is not a reason to select
browser-only packages in every native platform job. A failing platform-specific
tool should be diagnosed separately rather than quietly reducing common tests.
