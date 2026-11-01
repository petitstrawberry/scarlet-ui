# Website and browser samples

The project website uses the official Astro Starlight theme in `site/`.
It introduces ScarletUI as a cross-platform Rust toolkit and publishes selected
repository documentation without modifying the original Markdown sources.
The homepage embeds a small real ScarletUI/SGFX icon browser. It is not
an HTML recreation: supported browsers load the same Widget Factory Wasm
module and render Tabler icons through IconView, with reactive category, icon
selection and stroke-width controls. A second, independent counter pairs its
compiled Rust view source with the actual rendered result below the hero.

The surrounding documentation remains usable without JavaScript or WebGPU.
A static capture stays in place when GPU support is unavailable or startup
fails. The live runner pauses offscreen and in hidden tabs, preserving state;
page disposal stops it. Document wheel scrolling, touch panning and Tab focus
navigation are not captured by the embedded specimen.

## Build and preview

Use Node 24, Python 3.11+, Rust supporting edition 2024, and the matching
wasm-bindgen CLI. From the repository root:

```sh
npm ci --prefix site --ignore-scripts
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
python3 tools/build-web-samples.py
python3 tools/build-web-samples.py --check target/web-samples
python3 -m http.server 8765 --bind 127.0.0.1 --directory target/web-samples
```

Open `http://localhost:8765/`. The build compiles both examples with
`--release --locked`, imports the existing reference documentation, builds
Starlight, generates matching JavaScript/Wasm bindings, validates local links,
and records file hashes in `build-manifest.json`. Generated files remain under
ignored directories. `CARGO_TARGET_DIR` is honored; `--profile dev` selects a
debug Wasm build. Existing output is replaced only when it carries this tool's
generated marker. Source archives record unknown Git identity as null.

This combined Starlight artifact is configured for a **site root**, not an
arbitrary project prefix. Generated theme assets use root-relative URLs.
The standalone example modules still use relative JS/Wasm paths. Serve Wasm
as `application/wasm` and JavaScript with its JavaScript MIME type. No CDN,
credentials, backend service or browser security flags are required.

Routes: `/` introduction, `/getting-started/`, `/reference/` pages, `/try/`
browser selection, `/counter/`, and `/widget-factory/`. The two full-window
sample URLs are unchanged. `site/README.md` describes the theme and document
import/update path; original `docs/` files remain authoritative.

## GitHub Pages

The approved destination is **https://ui.scarlet-os.org/**. The payload is the
contents of `target/web-samples`, not the repository. Deployment remains
main-only after merge, gated on package and actual browser-smoke success and
the protected `github-pages` environment. PRs never deploy. Manual dispatch
also requires main; this workflow does not change DNS, custom-domain settings,
HTTPS enforcement or environment protection.

The destination guard validates the exact custom host and root path, verifies
DNS/TLS without bypassing certificates, and publishes an HTTPS environment
link. HTTP returned in GitHub API metadata is not permission to serve the site
insecurely. Unrelated hosts and the old project-prefix destination are rejected.
The owner manages DNS/CNAME and Pages settings separately.

## Checks

```sh
python3 tools/test_web_samples.py
python3 tools/test_pages_destination.py
node --test samples/capability.test.cjs
node --test site/scripts/*.test.mjs
```

Static checks cover artifact paths, imported documentation integrity, adapter
messages and the specimen's asynchronous visibility/disposal races. Browser
checks additionally inspect real GPU pixels, icon/category/stroke selection,
independent counter state, full-demo clicks and slider changes,
pause/resume state, wheel scrolling, narrow layouts, links and Back navigation.
The optional `/try/` capability check does not request identifying adapter
information or guarantee later device initialization.

## Browser limitations

Each web application instance uses one canvas/window and WebGPU. The homepage
loads independent Factory and counter modules for its two examples. It has no WebGL or
Canvas2D fallback. Canvas accessibility, IME/dead-key composition, clipboard
integration and device-loss recovery are not implemented. The homepage's
static fallback is an image, not an accessible interactive replacement.
Native toolkit use and reading HTML documentation do not require WebGPU.
No security-weakening flags are recommended for normal visitors.

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
