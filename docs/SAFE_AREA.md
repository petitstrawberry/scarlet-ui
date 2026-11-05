# Safe-area layout

ScarletUI receives a per-window `EdgeInsets` snapshot from the platform. Native
hosts use the OS API; Web uses CSS `env(safe-area-inset-*)`. The canvas/backing
surface and window background retain their full size. System edges are layout
reservations, not a translation of the canvas or a device-model constant.

## Ownership

`Window` handles all edges by default, preserving existing application layout.
To allow content to paint beneath system edges, defer handling to the content:

```rust
use scarlet_ui::prelude::*;

let window = Window::new(
    "Library",
    ScrollView::new(Text::new("Content")).vertical(),
)
.content_safe_area(SafeAreaEdges::NONE);
```

An outer `ScrollView` handles inherited reservations by adding content insets.
Its viewport, clip and background remain full-size. At rest the first item is
safe; scrolling can carry it beneath the system edge. Extra trailing space lets
the final item stop above the home indicator. Scrollbar tracks and caret reveal
also respect those reservations. Scroll motion continues to use retained
composition and does not relayout content to apply safe-area padding.

`NavigationView` places its fixed navigation/header chrome within the safe
rectangle. Floating bottom navigation leaves the content scrollport full-height
and combines the bar's overlap clearance with the platform bottom reservation.
Switching to a sidebar removes the previous bar clearance. Static content is
laid out inside the usable area instead of beneath the bar.

For artwork that should start at the viewport edge, opt out on that scrollport:
`ScrollView::new(artwork).content_safe_area(SafeAreaEdges::NONE)`.
Unhandled reservations remain available to descendants that need safe controls.

## Explicit safe layout

Static branches can own their safe-area padding explicitly:

```rust
use scarlet_ui::prelude::*;

let content = Text::new("Safe controls")
    .padding(16.0)
    .safe_area_padding(SafeAreaEdges::TOP | SafeAreaEdges::HORIZONTAL);
let window = Window::new("Controls", content)
    .content_safe_area(SafeAreaEdges::NONE);
```

`SafeArea::new(content).edges(...)` is the equivalent container API. Handled
edges are removed from the inherited value, so nested containers and scrollports
do not add the same reservation again. Different siblings inherit their own
copy. Ordinary visual padding does not consume system reservations.

These values describe **unconsumed reservations in a layout scope**, rather than
the geometric distance of every descendant from a screen edge. Apply SafeArea
at the container that owns the boundary; an arbitrary inset frame does not
implicitly consume it. Choose edges explicitly for separate branches.

## Observation and lifecycle

`GeometryProxy::safe_area_insets()` exposes the scope's unconsumed insets through
`on_geometry_change`, including changes that leave the view's size unchanged.
`RenderingPipeline::safe_area_insets()` remains the window's raw platform
snapshot. There is no process-wide current-safe-area variable. Component
reconciliation retains the inherited scope, and rotation updates both layout
and scroll extents. Keyboard occlusion remains a separate caret-reveal concern.

Custom Elements that create children during layout must retain the value passed
to `Element::set_safe_area_insets` and pass it to new children before layout.
Render objects can consume edges with `child_safe_area_insets`; ordinary render
objects forward them unchanged. Built-in elements handle this automatically.

Widget Factory uses this layout ownership and derives its responsive content
width from geometry after subtracting the inherited system edges. Standalone
Web sample canvases are fixed to the viewport because ScarletUI owns scrolling;
embedded homepage specimens continue to use their DOM container. CSS fixed
positioning is not a promise that every iOS version suppresses its system edge
blur; that behavior needs separate device validation.
