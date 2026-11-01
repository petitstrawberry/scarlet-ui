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

## Link previews

`npm run build` generates static, page-specific sharing PNGs and metadata after
Astro renders the pages. `tools/build-web-samples.py` repeats the same step after
adding the standalone samples, so their routes receive cards too. There is no
runtime image service, API key, or network font dependency.

The pipeline reads each rendered title, description and canonical URL rather
than maintaining a second page catalog. For imported reference pages that only
have the generic site description, it uses a bounded excerpt of the first real
source paragraph. Explicit page descriptions take precedence. The homepage
keeps its visible `ScarletUI` heading while using a descriptive browser/social
title. Error/noindex pages are excluded.

Artwork uses the shared `scripts/social-image.mjs` template and repository
fonts. Each route uses its own title and source description. The homepage adds
a few genuine stock-control crops, not an icon catalog or a new logo; see
`assets/sharing/PROVENANCE.md` for capture/update details. Images are 1200×630 PNGs;
long text wraps and scales within fixed safe margins or fails the build instead
of being silently clipped. The font license is in `assets/fonts/LICENSE`.

Run `node --test scripts/*.test.mjs` for controller, source and image regressions.
The build additionally runs the Python metadata parser tests and generated-HTML
checks, including unique tags, absolute URLs, all card paths and PNG dimensions.
For a sandbox with an unwritable default font cache, set `XDG_CACHE_HOME` to a
writable local build-cache directory; this does not affect shipped assets.

## Drawing specimen

The homepage Sketch is the real Rust `Specimen` in Widget Factory, rendered by
SGFX. Its paths use normalized paper coordinates so resizing preserves the
composition. Undo removes the latest stroke; Clear empties the drawing (it is
not undoable). The bounded model accepts up to 128 strokes / 4096 total points;
Clear recovers that budget. A stable mesh handle with revisions avoids creating
a GPU resource for each pointer move.

The invisible `data-drawing-input` region matches only the paper bounds. It
forwards pointer events to the existing platform-web canvas handler and applies
`touch-action: none` there; the rest of the specimen retains page pan/zoom.
Wheel and keyboard events are not intercepted. Test this boundary when changing
paper geometry; JavaScript unit tests and the browser smoke cover the mapping.
