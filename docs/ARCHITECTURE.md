# ScarletUI Architecture

This document defines ScarletUI's retained runtime model. It is normative for
widget implementations: optimizations may change, but the identity and state
preservation rules below must remain true.

ScarletUI follows the same broad separation as Flutter's Widget, Element, and
RenderObject model, using Rust values and traits:

| ScarletUI | Lifetime | Responsibility |
|---|---|---|
| `View` | Short-lived description | Declares configuration, children, keys, and `State` dependencies. |
| `Element` | Retained while identity matches | Owns tree position, lifecycle, subscriptions, and the mapping from old to new Views. |
| `RenderObject` | Retained by a render Element | Owns layout, paint, hit-testing, and transient interaction state. |

A `View` may contain cloneable `State<T>` handles, but it is still treated as an
immutable description. Building a View must be cheap and must not itself mount,
lay out, paint, or dispatch events.

## Update cycle

Views are not rebuilt unconditionally on every presented frame. The normal
sequence is:

```text
State notification
    -> mark subscribed Element dirty for Build or Paint
    -> build a fresh child View description
    -> reconcile it with the retained child Element
    -> layout, when structure or geometry changed
    -> paint
    -> composite/present
```

An idle frame can reuse the complete Element and RenderObject trees. A build
executes only for a dirty component or another explicitly updated branch. This
keeps declarative View construction cheap while preserving runtime state.

`View::create_element()` is inflation: it is called when a branch is first
mounted or when reconciliation proves that the existing branch is
incompatible. It is not the update path.

## Reconciliation and identity

Reconciliation happens at the parent that owns a child slot. For one child:

1. If the old Element's View type and key are compatible with the new View,
   call `Element::update()` and keep the Element ID.
2. If the Element can update its View, reconcile its descendants in the same
   way.
3. If it returns `UpdateResult::Replaced`, unmount and replace only that child
   branch.

When an existing child returns `UpdateResult::Updated`, reconciliation must
also retain that child's Element ID as an invalidation target. Propagating only
the top-level component's result is insufficient: the top-level component can
sit outside a `RepaintBoundary` while the changed RenderObject sits inside it.
In that case, repainting only the ancestor would permit the nested boundary to
reuse stale pixels. ScarletUI therefore schedules the compatible updated child
locally and lets layout/paint propagation find the owning boundary.

Unkeyed siblings have positional identity. Explicitly keyed siblings have model
identity and can move without losing their retained Elements:

```rust
use scarlet_ui::prelude::*;

let row = Text::new(file_name).key(file_id);
```

Keys must be unique among siblings. Add a key when same-parent children can be
inserted, removed, or reordered and runtime state should follow the model.
Changing a key intentionally starts a new lifetime.

Reconciliation must be centralized. A container must not build a temporary
Element tree and then copy or replace it during an ordinary update. It builds
child Views and passes them to the shared child reconciler.

## Component elements

A component Element owns the current component View and a retained child
Element. Its builder returns a child `View`, not a child `Element`.

On build or parent update it:

1. builds the new child View description;
2. reconciles that description with its existing child Element;
3. refreshes subscriptions when its dependency set changes;
4. requests layout or paint according to the result.

The component Element itself survives while its concrete View type and key are
compatible. A state change therefore does not imply replacing its child tree.

## Render elements and RenderObjects

A render Element owns one RenderObject and zero or more child Elements. View
configuration and child structure are updated independently:

- `RenderObject::update()` copies changed View configuration into the existing
  RenderObject whenever possible.
- Container child builders return `Vec<Box<dyn View>>`; those descriptions are
  reconciled with retained child Elements.
- If a RenderObject cannot update in place, its registered builder may recreate
  that RenderObject locally. The owning Element and compatible descendants are
  still preserved.
- Replacing a whole render Element is reserved for an incompatible View type or
  key.

Widget runtime fields such as hover, pressed, drag, scroll offset, text
selection, cached raster data, and measured geometry belong in an Element,
RenderObject, or explicit controller/state object. Rebuilding a View must not
silently reset them.

## Pointer interaction and capture

Pointer hover and button capture are separate runtime concepts:

- The hover path is always derived from the pointer's current geometric hit
  test. Button capture must not replace or freeze that path.
- Moving between targets emits `Exited` and `Entered` only for the divergent
  suffixes below their common ancestor. A shared hoverable ancestor therefore
  remains hovered while the pointer moves between its descendants.
- A left-button press captures its target for drag and release dispatch. The
  captured target continues receiving those events outside its bounds, while
  the hover path still exits it normally.
- Button-like controls clear their armed/pressed state on `Exited`. A release
  invokes the click action only while that state is still armed, so pressing
  inside and releasing outside cancels the click.
- `MouseEvent::ButtonCancelled` is a terminal, non-activating event. It clears
  capture and pressed or dragging state but must never invoke a click,
  selection, or release action. Platform adapters use it for interrupted touch
  contacts and equivalent native cancellation paths.

Compatibility note: `ButtonCancelled` extends the public `MouseEvent` enum.
Downstream exhaustive matches must add a cancellation arm; treating it like a
normal `ButtonReleased` is incorrect because release handlers can commit an
action.

The platform adapter must make surface crossings observable to each pipeline.
It may deliver an explicit leave event or a final out-of-bounds pointer motion
to the old surface before routing motion to the new surface. Without that
transition, no client-side hit test can clear hover from the old surface.

## State and dependencies

`State<T>` is a shared, cloneable model-state handle. Cloning a View clones the
handle, not the stored value, so the new description observes the same state.
The View derive macro reports state fields through `listenables()`, and mounted
Elements subscribe to those dependencies.

Use `InvalidationKind::Build` when a value can change View structure or
configuration. Use `InvalidationKind::Paint` only when the existing render
structure can consume the value directly without rebuilding or relayout.

Do not use hand-written `StateId` or `ElementId` values as child identity.
Declare component state through `#[derive(View)]`, and use View keys for dynamic
child identity. Framework internals that create state dynamically must use the
ID generator.

## Persistence rules

The following state must survive a compatible ancestor rebuild:

| Runtime state | Owner | Preserved when |
|---|---|---|
| Hover and pressed state | RenderObject | View type and key remain compatible. |
| Scroll offset | Scroll RenderObject/controller | The `ScrollView` branch remains mounted. |
| Focus and text interaction | Element/RenderObject plus model State | The focused branch remains compatible. |
| Lazy row identity | Lazy container child Element | Its item identity remains in the materialized range. |
| Raster/layout caches | RenderObject/pipeline | Configuration and constraints permit reuse. |

Removing a branch ends its lifetime. For example, the current `TabView` mounts
only the active page, so switching away may discard that page's local runtime
state. State that must survive unmounting belongs in shared `State<T>` or an
explicit controller.

## Widget implementation rules

New widgets must follow these rules:

- Use `ComponentElement::new_with_builder` for a View that expands into another
  View.
- Use `RenderElement::with_view_children` for a RenderObject with declarative
  children.
- Use `RenderElement::with_view_children_and_updater` when generic View data,
  such as a callback type, requires a typed in-place RenderObject updater.
- Implement `RenderObject::update()` for in-place configuration changes and
  report whether layout is required.
- Return `UpdateResult::Replaced` only when the current runtime object cannot
  represent the new View.
- Mount and unmount only branches that are actually inserted or removed.
- Keep build functions free of persistent runtime state and side effects.
- Use `.key(...)` for reorderable dynamic siblings; do not manually assign
  framework IDs.

`RenderElement::with_children` remains a low-level legacy constructor. New
declarative containers must use View child builders so later builds can
reconcile instead of replacing descendants.

## Required regression tests

Changes to reconciliation or container layout must cover these contracts:

- same type and key preserve the Element ID;
- changing the type or key replaces only that branch;
- keyed reordering preserves each model's Element ID;
- a nested state rebuild preserves compatible RenderObject identity;
- an update started outside a `RepaintBoundary` invalidates the changed child
  inside that boundary;
- a `ScrollView` offset survives content updates;
- lazy lists and grids update visible rows without rebuilding on a warm scroll;
- grid/list selection changes repaint visible rows without an unrelated input
  event;
- a retained text field applies declarative focus transitions while preserving
  caret editing, Backspace, and keyboard bubbling;
- unrelated applications such as Clock still lay out and render correctly.

Performance tests should warm caches before measuring. Allocation tests are
separate from identity tests: zero allocation is an optimization goal, while
correct reconciliation and state preservation are semantic requirements.
