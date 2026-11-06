# Tabs and text study

TabView defaults to Bubble Glass: a centered, content-sized glass strip with
inset selection tint. HeaderBar and sidebar/bottom navigation use the same
material family. `.chrome_style(ChromeStyle::Simple)` selects the original
solid treatment; `.chrome_style(ChromeStyle::BubbleGlass)` restores the default.

Default density remains a 36-logical-pixel tab bar with 14-pixel labels.
`.material(GlassMaterial::regular().opacity(0.55).blur_radius(12.0))` customizes
its glass. Apply `.style(TabStyle { ... })` after the chrome preset to override
colors or enable an indicator. Preset changes reset appearance colors but
preserve dimensions, placement and selected state.

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
Older images of the solid treatment do not represent the default Bubble Glass appearance.
The integrated CPU/SGFX backends now support actual backdrop blur and cached
soft shadows. See GLASS_MATERIAL.md for limitations, fallback and performance.
Native, physical GPU and dark-mode review remain necessary before adoption.
