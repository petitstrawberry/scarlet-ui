# Scarlet UI file icons

This crate embeds the `extra` folder artwork and `vivid` extension artwork
from [dmhendricks/file-icon-vectors](https://github.com/dmhendricks/file-icon-vectors).
The upstream revision is recorded in `UPSTREAM.SHA1`; the original SVG files
are retained under `source/`, and the upstream license and attribution notes
are retained in `LICENSE` and `UPSTREAM-README.md`.

The build script tessellates the SVG paths into colored triangle meshes. The
runtime crate is `no_std` and does not read files or parse SVG.

The Extra set is used for folders and the Vivid set is selected by file
extension. Unknown extensions use Vivid's generic blank icon.
