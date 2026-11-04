# Control study (local trial)

The shared backdrop material direction has resumed after guideline research and
native visual review. Other controls are being brought into the same material
family. This is still a local trial, not an accepted final design for every part.
The initial TabView treatment is not accepted. Closed Select geometry and interaction remain unchanged; its old gradient
material has been replaced after explicit review feedback.

This is an exploratory shared visual language for ScarletUI, shown using actual
stock controls in Widget Factory. It is not a completed redesign of every widget.
The local branch changes Button, TextField, Select, Toggle, Slider, TabView, Text
roles, TextView, Menu/Surface and an opt-in navigation sidebar. CPU contour rendering is antialiased; no
renderer is removed.

## Direction

- Shared real in-app backdrop blur, uniform semantic tint and restrained soft
  elevation. Fixed specular lines and vertical bevel gradients are not used in
  the new material. Foreground labels remain sharp. This is frosted glass, not
  optical refraction or sampling of the operating-system desktop. See
  GLASS_MATERIAL.md for bounds, fallback and performance limitations.
- Shared 36-logical-pixel default height, 14-pixel label size, 10-pixel content
  padding and 12-pixel radius for the three form controls. Explicit frames and
  compact titlebar/menu density remain authoritative.
- Measured glyph width and line advance replace byte-count width and guessed
  baselines. Fonts remain the platform's existing font selection.
- Scarlet marks selection and keyboard focus. Focus uses a soft three-pixel
  rounded outline, following the supplied macOS search-field reference. The
  glass face stays unchanged, without darkening, red wash or underline.
  Foreground text and the caret remain sharp.
- Buttons use a visible neutral glass face and slightly stronger soft elevation.
  Their backdrop blur and translucency remain real; menus, thumbs, tabs and
  structural surfaces do not inherit this button-specific material.
- Neutral surfaces and readable type should carry the interface. Hover and press
  change the same material instead of introducing another color family.
- Toggle and Slider use plain horizontal pill thumbs, without idle gloss or
  specular rims. Toggle's visible track is 36×16 with a 21×14 thumb, inside the
  existing 36×20 hit target. Slider uses a 20×16 thumb and a 6-pixel track.
  While held, the thumbs expand to 31×22 and 25×22 respectively, preserving the
  thumb center. Their face becomes translucent and has a soft shadow, without
  any added rim. These proportions follow the supplied interaction reference,
  not copied assets. Off/on examples appear together in the Factory.
- `NavigationSidebarStyle::Simple` stays the default. `Floating` is opt-in and
  adds an inset panel, rounded selection rows and explicit spacing. Bottom
  navigation behavior is unchanged. Widget Factory opts into the trial style.
- Disabled controls have disabled input and focus behavior, not just faded paint.
  Button and Select gain keyboard activation and focus feedback.
- Select separates Form and Menu styles. Form remains the default input surface.
  Widget Factory's Density uses Menu: right-aligned value and a small up/down
  chevron button. Menu overlaps the trigger: its selected label preserves the
  closed label's X position and baseline unless a real window edge constrains
  the panel. Popup painting and pointer hits escape ancestor clips. Hover and
  selected check are separate; Form keeps its detached input popup. Popup
  material now samples and blurs the actual app scene.
- Tabs use 36-pixel bars and 14-pixel labels. The material trial retains the
  original full-height tab strip and functional content-edge selection indicator,
  rather than the rejected rounded-chip treatment. Explicit TabStyle overrides work.
  Text gains independent semantic size and tone roles, adopted throughout the
  Factory. See TABS_AND_TEXT.md.

The language should extend to tables, scrollbars and window chrome. Those
still need their own state, density and layout review; a palette or radius change
alone is not a full redesign. Automatic navigation headers and scroll-linked
large titles are a separate API design effort.

## Try it

Run Widget Factory and open Controls. The study uses the stock Button, TextField
and Select together, stacks on narrow widths, and includes disabled examples.
Save changes increments a counter and rotates the real color regions beneath
these controls, demonstrating that the material responds to its backdrop. Check pointer hold/cancel, keyboard focus and
Enter/Space, editing, Select navigation/commit/Escape, and disable while captured.
No HTML replica is used.

For the native CPU window:

```sh
SCARLET_UI_WINIT_RENDERER=cpu cargo run --locked -p scarlet-ui-widget-factory
```

This winit/softbuffer path has been launched and inspected on the cloud Linux
desktop. The trial includes antialiased shape contours, cached soft shadows
and genuine in-app backdrop blur on CPU and SGFX. Real SGFX 1x/2x pixel readback
has been checked on SwiftShader Vulkan; this is not a hardware-GPU performance
measurement. Refraction remains unimplemented.

For diagnostic raster sheets only:

```sh
cargo run --locked -p scarlet-ui-core --example control-study -- target/control-study
```

This writes PPM images at 1x/2x and a dimensions record. These are render-command
diagnostics, not screenshots of the native application.

The focus/button revision has a separate real-element diagnostic covering rest,
hover, pointer hold, keyboard focus and focus loss on white, patterned and
explicitly dark glass surfaces:

```sh
cargo run --locked -p scarlet-ui-core --example focus-button-study -- target/focus-button-study
```

This is also CPU render-command output, not a native application screenshot or
a claim that the application has a global dark-theme switch. The main control
study additionally covers disabled controls and form selects; unit tests cover
the focused menu-style chevron and expanded-select behavior.


## Remaining checks before adoption

- SGFX on physical hardware and web, plus dark-mode contrast.
- Actual macOS/Windows font, input and accessibility behavior.
- Mobile touch and physical iOS/Safari keyboard/focus behavior.
- Existing WebGPU smoke coordinates assume the older Controls page and need
  revision if the study layout is adopted.
- Review the whole interface and settle Tab/Sidebar structure before a pull request.

## Native interaction findings

Real winit CPU window checks confirmed Button click/Space activation, text edits,
Select arrow/Enter selection, Toggle changes and Slider dragging. The existing
input infrastructure has separate limitations:

- Winit synthesizes printable characters from logical shortcut keys, so Ctrl+A
  selects all and then erroneously inserts `a`.
- Shared keyboard traversal with Tab/Shift+Tab is not implemented. This trial
  exposes focus feedback and activation but does not claim complete traversal.

These predate this branch and need their own input-policy fix and platform tests.

The trial has not merged, deployed or published anything.

## Trial validation

- Focus-ring/button revision: core library 534 passed, 1 ignored; shared control
  geometry 3 passed; core doctests 23 passed; facade integration tests 32 passed.
  This includes AA, CPU material, popup, form controls, thumbs, tabs and sidebar.
  Real-element CPU diagnostics were inspected at 1x/2x on plain, patterned and
  explicitly dark surfaces. Enabled button label contrast in these diagnostics
  is at least 10.3:1; this is not a whole-application accessibility audit.
- This revision's actual Widget Factory control-study and plain-canvas button
  views were also rendered through the normal pipeline with SGFX/Vulkan at
  1x/2x. SwiftShader readback showed the corrected button material and focused
  TextField ring, with zero backdrop fallbacks. This is software-Vulkan scene
  output, not a physical-GPU result or native-window screenshot.
  A parallel legacy-popup pixel test can race process-global UI scale; it passes
  in isolation and in the serial full suite. This harness limitation remains.
- Shared control geometry: 3 passed.
- Widget Factory native tests: 11 passed, including default button-material,
  responsive-width and touch regressions;
  one manual performance benchmark is ignored in the ordinary suite.
- Native Widget Factory binary builds; wasm32 Widget Factory library checks.
- Real CPU window inspected at 1x, and an actual 2x launch checked text/layout.
- Native overlap Menu kept the selected label aligned, upward hover/click worked,
  hover exit preserved the checkmark, and keyboard selection committed. Toggle
  changed state, Slider moved, and disabled Select did not open.
- Expanded held-thumb geometry, cancellation, release repaint and 1x/2x CPU
  route parity are covered by tests. A native held-state screenshot has not yet
  been captured; bound-window timed clicks did not activate the app, although
  desktop click/drag worked.

Remaining quality work includes the existing text rasterizer's fractional origin
and glyph-cache-size handling, plus GPOS-capable shaping shared by measurement and
painting. Per-widget letter-spacing adjustments were not used to hide this.
Idle thumbs deliberately have no fixed gloss line, outline or specular decoration. Real
CPU shadows make the white slider thumb visible on white; the earlier preview
without shadow rendering is obsolete. The material direction remains under
review rather than accepted as complete.
