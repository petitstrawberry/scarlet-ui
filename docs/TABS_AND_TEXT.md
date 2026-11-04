# Tabs and text study

Widget Factory opts into `TabView::material(GlassMaterial::regular())`.
After review, the trial preserves the original view-switching tab-strip structure:
full-height rectangular selected tab, a visible bar, and the functional selection
indicator at the content edge. The earlier tiny rounded-chip alternative was
rejected. Glass changes the material, not the meaning of the component.

Default density remains a 36-logical-pixel bar with 14-pixel labels. Hover and
press change uniform faces; explicit TabStyle border/indicator overrides remain
available. The material option declares the dependency needed to bypass isolated
raster caches. Non-material custom-color paths are uniform, without old bevel
gradients. The small selected indicator conveys state, not a fixed decorative
specular highlight.

Floating navigation keeps its existing geometry and gains a neutral structural
tint so the panel remains visible on a white canvas. Simple and BottomBar remain
unchanged. This remains a visual trial pending whole-interface review.

Top and bottom placement, caller-owned selection, touch cancellation and active
content remain intact. The selected face follows the same clamped index as the
content when a tab list shrinks. Drawing and hit testing stay inside the view;
labels use the real font line advance for vertical centering.

`Text` has two independent, optional semantic modifiers:

```rust
use scarlet_ui_core::prelude::*;

let title = Text::new("Overview").text_style(TextStyle::Title);
let detail = Text::new("Updated just now")
    .text_style(TextStyle::Callout)
    .tone(TextTone::Secondary);
```

- `Title`: 24 pixels; `Headline`: 18; `Body`: 16; `Callout`: 14; `Caption`: 12.
- `Primary`, `Secondary` and `Tertiary` map to the palette's text colors.
  Tertiary is only for disabled or nonessential information.
- Existing `Text::new` remains 16-pixel primary text. Font selection, glyph
  rasterization, text measurement and drawing are unchanged.
- Explicit `.font_size()` and `.color()` remain available. The last modifier of
  each kind wins. `Text::from_state()` remains reactive after either modifier.

Run the real render-command study at 1x and 2x:

```sh
cargo run --locked -p scarlet-ui-core --example tabs-text-study -- target/tabs-text-study
```

The output is diagnostic PPM imagery, not a native application screenshot.
Older images of the solid treatment do not represent the new opt-in material.
The integrated CPU/SGFX backends now support actual backdrop blur and cached
soft shadows. See GLASS_MATERIAL.md for limitations, fallback and performance.
Native, physical GPU and dark-mode review remain necessary before adoption.
