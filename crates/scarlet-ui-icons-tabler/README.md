# Scarlet UI Tabler Icons

This crate vendors all 5,075 outline and 1,053 filled SVG files from the
Tabler Icons v3.43.0 release. They are available under the MIT license included
in [`LICENSE`](LICENSE). The release archive checksum is recorded in
[`UPSTREAM.SHA256`](UPSTREAM.SHA256). Original assets are retained under
`source/outline` and `source/filled`.

Only the 118 standard Scarlet UI names listed in `selected.toml` and their
available filled counterparts are compiled. `build.rs` converts their SVG
paths into typed, allocation-free Rust drawing data, so the remaining source
assets do not increase application binaries.

Upstream: https://github.com/tabler/tabler-icons
