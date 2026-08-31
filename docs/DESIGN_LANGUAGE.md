# ScarletUI visual language

ScarletUI uses a quiet, content-first desktop visual language. The semantic
palette and existing widget layout remain authoritative; rendering code adds
polish through consistent geometry, borders, typography, and interaction
states rather than decorative depth.

## Implementation sequence

Visual refinement is deliberately split into two layers:

1. **Foundation:** align geometry roles, spacing and density, typography, and
   hover, pressed, selected, focused, and disabled state semantics.
2. **Effects:** add role-based elevation, gradients, translucency, and material
   treatment only after the foundation is consistent.

Effects are design-system tokens, not one-off widget decorations. A menu and a
popover may share a floating-surface elevation, for example, while an inline
field remains flat. The GPU path may render the full effect; CPU rendering must
retain the same structure and state meaning with a simpler opaque or flat
fallback.

## Surface hierarchy

Tonal surfaces establish containment before effects establish elevation.

| Level | Treatment | Examples |
|---|---|---|
| Canvas | Primary application background | page content, editor canvas |
| Structural | Secondary background plus divider | title bars, sidebars, tab strips |
| Section | Tertiary background without shadow | tracks, inactive switches, grouped regions |
| Raised | Button surface with subtle directional response | buttons, collapsed selects |
| Floating | Opaque foreground surface with shadow | menus, expanded selects, popovers |

Selection is not a surface level. The navigation leading rail and tab underline
remain state indicators and never acquire elevation. Structural and Section
surfaces stay flat; adding radius or shadow cannot be used to manufacture
hierarchy that the tonal structure does not express.

## Shape roles

| Role | Geometry | Examples |
|---|---|---|
| Layout | Square | sidebars, tab strips, split panes, headers |
| Control | 6 px radius | buttons, fields, selects (collapsed or expanded), editors |
| Collection item | 4 px radius | menu hover and pressed highlights |
| Floating surface | 8 px radius | menus, independent popovers |
| Track | Capsule | sliders, progress bars, scrollbars, toggles |

Radii express a component's role. They are not applied to every rectangle.
Caller-defined drawing primitives such as `Rectangle` retain their explicit
geometry. Elevation does not change identity geometry: an expanded select gains
the Floating effect but retains the same 6 px radius it had while collapsed.

## Effect roles

Effects communicate a surface's place in the interface; they are not general
decoration.

| Role | Treatment | Examples |
|---|---|---|
| Inline | Flat semantic fill and hairline border | fields, editors, tracks |
| Raised | Very low-contrast vertical gradient and optional hairline; no drop shadow | buttons, collapsed selects |
| Floating | Opaque surface, hairline, and a two-layer soft shadow | menus, expanded selects, popovers |
| Overlay | Reserved for modal or blocking foreground material | dialogs, sheets |

The raised gradient changes luminance by less than two percent and reverses
subtly while pressed. Floating surfaces combine a close ambient shadow
(`y=1`, `blur=3`) with a wider directional shadow (`y=4`, `blur=10`). Shadow
color comes from the semantic palette. Structural regions, navigation
selection, tab selection, and scroll tracks do not gain shadows.

GPU paint renders the gradient and layered blur approximation. CPU paint uses
the gradient midpoint as a flat fill and omits the shadow while retaining the
same surface, border, and state semantics.

## Runtime input adaptation

Touch support is an input-capability concern, not a second visual theme.
`InputEnvironment` carries a generation, optional tablet and lid states, and
direct-touch, fine-pointer, keyboard, and pen capabilities. Explicit tablet
mode selects Touch. Otherwise touch plus a fine pointer selects Hybrid, touch
alone selects Touch, and remaining configurations select Pointer.

The platform installs its initial snapshot before scene construction and may
send `InputEnvironmentChanged` at any time. ScarletUI then rebuilds, relays out,
and repaints every open pipeline in the process. Pointer mode preserves the
compact desktop metrics. Touch and Hybrid use larger control minimums,
navigation and tab rows, scrollbars, sliders, toggles, and client-side titlebar
hit areas. The current implementation is process-wide so all windows and
runners transition coherently; the public snapshot model is the seam for future
runner- or per-window environments.

On Winit/macOS, `SCARLET_TABLET_MODE=tablet` (also `1`, `true`, `yes`, or `on`)
forces the initial tablet environment. `laptop` (also `0`, `false`, `no`, or
`off`) forces pointer/laptop behavior. Matching is case-insensitive; invalid or
unset values leave tablet state unknown. A native Winit touch event also
advertises direct-touch capability dynamically.

## State roles

- Selection remains persistent while hover or focus is shown.
- Navigation selection uses the existing 3 px leading scarlet rail and label
  color. Hover remains a neutral row surface.
- Tabs use their existing selected surface plus a 2 px scarlet line indicator.
- Fields keep the same bounds in every state. Focus changes the border color
  and stroke weight without moving content.
- Focus and active-outline highlights share the same light primary tint across
  fields, editor borders, slider thumbs, and draggable dividers. Solid primary
  remains reserved for values and persistent selection indicators.
- Control hairlines are opaque semantic colors. Transparency is reserved for
  overlays and shadows so a strong parent surface cannot erase a control edge.
- Text selection uses an opaque, precomposited primary tint. The primary is
  lifted before a 24% surface mix so its hue remains visible without becoming
  dense; repeated painting cannot darken it.
- Pressed and hover colors continue to come from the semantic palette.
- Desktop toggles use a compact 36 x 20 px visual capsule and the primary
  accent. Touch and Hybrid environments use a larger 52 x 32 px target.

## Surfaces and type

- Base content and structural regions stay flat. Dividers are one-pixel
  hairlines.
- Floating surfaces may be rounded, but their structure must remain legible
  without shadows or gradients.
- Translucent or glass-like rendering is reserved for temporary foreground
  surfaces such as menus and popovers. GPU backends may add backdrop treatment;
  CPU rendering keeps the same geometry with an opaque semantic surface.
- macOS uses the installed system UI font first; the bundled font remains the
  cross-platform fallback.
- Window controls and widget positions are compatibility contracts and do not
  move as part of visual refinement.

## Rendering

The retained `PaintCommand` path is the high-fidelity path and tessellates
rounded geometry on the GPU. Text and icon masks carry coverage
antialiasing; full GPU shape antialiasing remains a renderer-quality task.
Legacy canvas rendering may use simpler fallbacks when an equivalent primitive
is unavailable.
