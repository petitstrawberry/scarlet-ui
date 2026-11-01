# ScarletUI documentation site

The site uses the official Astro Starlight template. The toolkit homepage and
Getting started guide are authored here; technical reference pages are generated
from the existing repository documentation. The original root `README.md`,
`docs/` and browser build guide remain authoritative and are not edited by the
import process.

## Reference imports

Run from the repository root:

```sh
node site/scripts/sync-docs.mjs
node --test site/scripts/sync-docs.test.mjs
```

The site's build runs the import script before Astro. It generates these ignored
files under `site/src/content/docs/reference/`:

| Reference page | Original source |
| --- | --- |
| `project.md` | `README.md` |
| `architecture.md` | `docs/ARCHITECTURE.md` |
| `application-contract.md` | `docs/1.0-contract.md` |
| `browser-samples.md` | `samples/README.md` |

Do not maintain or commit the generated copies. Edit their originals instead.
Each imported page gets Starlight title frontmatter from its first H1 and a
source attribution. Fenced examples and inline code are retained; relative
Markdown links resolve to the corresponding GitHub `main` source location so
links do not accidentally depend on the site's deployment prefix. External
links and same-page anchors stay unchanged. The importer does not rewrite API
examples or silently correct source documentation.

## Local development

Use the Node version required by `site/package.json` (Node 24.22.1 or newer).
From `site/`, install the checked-in dependency versions with
`npm ci --ignore-scripts`, then run `npm run dev` (which imports references first). Use `npm run build` to build the
static site and `npm run preview` to inspect that output. Consult the repository's
build workflow for how the standalone browser samples are added to the final
artifact. A local build does not publish the site.

The native toolkit is the primary entry point. Browser requirements and current
backend limitations belong in the documentation and browser demo entry, not in
the root homepage. Existing `/counter/` and `/widget-factory/` applications remain
separate canvas samples.

The starter was taken from the official `withastro/starlight` repository's
`examples/basics` at `6c9ca431629bbd37d969876f5f4281b5f5a34d38`.
Layout, navigation and theme switching are supplied by the installed
Starlight package; the project customizes the accent, a live specimen
in the documented hero HTML slot, and the code/result example below it. The original template license is retained.

For the complete working specimen and full demos, build/serve
`target/web-samples` with `python3 tools/build-web-samples.py` from the repository
root. Astro's standalone dev/preview commands do not add the Rust/Wasm artifacts;
without them the homepage retains its static image. The production package
is root-hosted at `https://ui.scarlet-os.org/`, not built for a project subpath.

The counter code is imported from `examples/web-counter/src/lib.rs` as raw source
and extracted between its `docs:counter-controls` markers at build time. The displayed excerpt is the compiled view body, excluding its method signature
and ownership setup. Edit the compiled body, not a second handwritten copy in
the site. Each canvas has
its own lifecycle controller and Wasm module; scrolling between them preserves
independent state.

Search is disabled with Starlight’s official `pagefind: false` configuration.
This omits the default search component (including its dialog and keyboard
shortcut) and search index instead of hiding an active UI with CSS.
