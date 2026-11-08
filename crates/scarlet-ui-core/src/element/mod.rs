//! Element module - runtime components for ScarletUI

mod component;
mod dirty;
mod element;
mod focus;
mod hstack;
mod id;
mod reconcile;
mod render;
mod tree;
mod vstack;

pub use component::ComponentElement;
pub use dirty::DirtyFlags;
pub use element::{
    Element, LayoutConstraints, ScrollAnchor, TextInputElementState, UpdateResult, WindowSizeLimits,
};
pub(crate) use focus::{focused_descendant_path, restore_focus_at_path};
pub use hstack::HStackElement;
pub use id::ElementId;
pub(crate) use reconcile::{update_child, update_children};
pub use render::{RenderElement, RenderObject as ElementRenderObject, ScrollOffsetUpdate};
pub use tree::{ElementTree, generate_element_id};
pub use vstack::VStackElement;

// Scroll dependency metadata survives placement-only updates. Build/layout and
// structural notifications invalidate it before the next input/frame pass.
static SCROLL_STRUCTURE_REVISION: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(1);
pub(crate) fn invalidate_scroll_structure() {
    SCROLL_STRUCTURE_REVISION.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}
pub(crate) fn scroll_structure_revision() -> usize {
    SCROLL_STRUCTURE_REVISION.load(core::sync::atomic::Ordering::Relaxed)
}
