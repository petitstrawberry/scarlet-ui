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

The floating bottom capsule shares its 8px outer bottom margin with the platform
bottom reservation: the visible capsule ends `max(8px, bottom inset)` above the
surface edge. Its icons and labels stay within that capsule. The scrollport
still reaches the surface edge; its trailing reservation counts the shared
margin only once, leaving the last item 8px above the capsule. Insets reported
by the host drive this geometry in both browser and standalone modes, including
rotation; no device-model dimensions are assumed.

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

## Home Screen status-bar blur

Widget Factory uses one fixed canvas for both content and the Factory / View /
Tools menu. The GPU paints the menu background; the canvas has no CSS background
color. Moving that canvas down by a guessed 16px did not address WebKit's color
extension detection and has been removed.

The host page now supplies an independent, empty, full-width fixed DOM element
at the top, above the canvas. Its 11px height exceeds WebKit's 10px thin-border
path. It declares the host surface color but clips its background to text;
because it contains no text, it paints no pixels over the menu. Pointer events
are disabled and it is excluded from accessibility. It does not reserve space,
resize/translate the canvas, or alter safe-area values. Only standalone or
fullscreen launch modes enable it (including `navigator.standalone`); ordinary
Safari tabs leave it absent. Rotation does not add or remove layout padding.

The host is deliberately light, like `ColorPalette::default()` used by WebMenu.
`--factory-surface` is shared by the host background, loading/error panel and
status element. OS dark appearance must not turn the strip black while the
canvas menu stays white. A future runtime theme switch must synchronize this
variable with the actual menu surface; do not independently follow media-query
colors or make the declared background transparent. The transparent-color case
in the test is only a negative paint control, not a supported way to suppress
the OS effect. No manifest, service worker or deprecated status-bar meta is added.

### Evidence and limits

- [WebKit LocalFrameView](https://github.com/WebKit/WebKit/blob/e8fb2ea5c8336081c16e4029d830af6a069bc70b/Source/WebCore/page/LocalFrameView.cpp#L2332)
  samples fixed/sticky containers near the viewport edge, checks visibility and
  dimensions, and reads the CSS background color. A GPU-painted canvas is not
  equivalent to a separate element with that CSS color.
- [WKWebView color extension](https://github.com/WebKit/WebKit/blob/e8fb2ea5c8336081c16e4029d830af6a069bc70b/Source/WebKit/UIProcess/API/Cocoa/WKWebView.mm#L3594)
  also requires host support and a positive obscured inset.
  [The iOS scroll-pocket decision](https://github.com/WebKit/WebKit/blob/e8fb2ea5c8336081c16e4029d830af6a069bc70b/Source/WebKit/UIProcess/API/ios/WKWebViewIOS.mm#L2449)
  uses the visible color extension, with additional host override conditions.
  CSS cannot force those host conditions; current upstream source is not proof
  of the exact WebKit build on a user's phone.
- [Falling Nikochan's implementation](https://github.com/na-trium-144/falling-nikochan/blob/1f8caf86839af9387c02a5868e35bd694ec47f6c/frontend/app/%5Blocale%5D/styles/irasutoya-like.css#L44)
  uses an 11px fixed color element with text clipping and no pointer events.
  Its [author's device report](https://qiita.com/na-trium-144/items/0add98a80ca2391e3f17)
  and [Saypip investigation](https://zenn.dev/uakihir0/articles/260920-ios27-pwa-blur)
  describe why an empty text-clipped element can supply the color without
  painting a visible overlay. These reports support a workaround, not an API.
- [MeshMonitor #5328](https://github.com/Yeraze/meshmonitor/pull/5328)
  changed an in-flow header to fixed, but retained an extra gap and explicitly
  left iOS blur validation open. It does not establish that fixed positioning
  alone solves this canvas case.
- [WebKit #317153](https://bugs.webkit.org/show_bug.cgi?id=317153#c2)
  deprecates `black-translucent`; Widget Factory already has no such setting.
  [#301994](https://bugs.webkit.org/show_bug.cgi?id=301994) documents differing
  installed-app viewport/status-bar behavior. Changing install metadata is not
  the proposed fix.

### Verification

Run `node --test tools/web-smoke/status-background.test.mjs` with Playwright's
Chromium installed. This uses the actual host HTML/CSS with only Wasm startup
removed. It checks launch-mode activation, portrait/landscape geometry, light
and dark OS appearance, CSS color changes, pointer pass-through and identical
canvas screenshots with/without the strip. It is a DOM/pixel test, not the
WebGPU smoke and not a simulation of iOS's native scroll-edge effect.

For the affected iPhone, serve the built samples over HTTPS and open Widget
Factory from its existing Home Screen icon. Cold-launch it, navigate to
Controls, scroll to the bottom, rotate both ways and return to portrait. Verify
Factory / View / Tools stay crisp and clickable, popups remain positioned
correctly, the selected tab stays above the home indicator and the final control
can scroll above the capsule. Repeat in ordinary Safari and OS light/dark modes.
Record the iOS build and whether the icon predates an OS update.

For a same-page A/B using Safari Web Inspector, remove
`document.querySelector('#ios-status-background')` with `.remove()`, then reload
to restore it. Removing the node matters: WebKit can retain a previous edge
candidate when it only moves or becomes transparent. If the result is ambiguous,
compare cold launches of baseline and modified builds; do not infer a successful
fix solely from the computed styles. Inspect `navigator.standalone`, the
`display-mode` queries, the canvas rectangle and `/diagnostics/safe-area/` too.
This environment cannot verify the system blur on an iPhone. The workaround is
implemented and its page-level side effects tested; device efficacy remains
unconfirmed.
