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

## Shape roles

| Role | Geometry | Examples |
|---|---|---|
| Layout | Square | sidebars, tab strips, split panes, headers |
| Control | 6 px radius | buttons, fields, selects, editors |
| Collection item | 4 px radius | menu hover and pressed highlights |
| Floating surface | 8 px radius | menus, select popovers |
| Track | Capsule | sliders, progress bars, scrollbars, toggles |

Radii express a component's role. They are not applied to every rectangle.
Caller-defined drawing primitives such as `Rectangle` retain their explicit
geometry.

## Future input adaptation

Touch support is an input-capability concern, not a second visual theme. A
convertible device may gain or lose a mouse, keyboard, pen, or touchscreen
while the application is running, so adaptation must be a live, per-window
view-environment value rather than a process-wide setting.

Adaptation must also be selective. It must not uniformly enlarge every visible
widget. A touch-capable configuration may expand invisible hit slop, increase
spacing only where adjacent targets would otherwise be ambiguous, expose
gesture affordances, or adjust transient scroll controls while retaining the
same compact visual geometry where it remains usable. Mixed-input devices must
continue to work well with pointer and keyboard interaction.

The current foundation intentionally exposes no static Desktop/Touch switch.
The dynamic environment and hit-target model should be designed with the input
event system before a public API is added.

## State roles

- Selection remains persistent while hover or focus is shown.
- Navigation selection uses the existing 3 px leading scarlet rail and label
  color. Hover remains a neutral row surface.
- Tabs use their existing selected surface plus a 2 px scarlet line indicator.
- Fields keep the same bounds in every state. Focus changes the border color
  and stroke weight without moving content.
- Pressed and hover colors continue to come from the semantic palette.

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

The retained `PaintCommand` path is the high-fidelity path and uses antialiased
rounded geometry on the GPU. Legacy canvas rendering may use simpler square
fallbacks when an equivalent primitive is unavailable.
