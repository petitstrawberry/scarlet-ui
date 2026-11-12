//! PipelineOwner - Manages dirty flags and orchestrates render phases
//!
//! PipelineOwner tracks which elements need to be rebuilt, laid out, or repainted,
//! and orchestrates the flush of these dirty phases.
//!
//! PipelineOwner also owns the StateRegistry, ensuring there is only one
//! registry per application.

#![allow(deprecated)]

use crate::element::{ElementId, ElementTree, LayoutConstraints};
use crate::geometry::Size;
use crate::id::IdAllocator;
use crate::os::Mutex;
use crate::pipeline::StateRegistry;
use crate::state::{State, StateId};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use std::println;

// Sparse updates benefit from targeted work. For a large dirty set, repeated
// root searches cost more than a single traversal of the live tree.
const MAX_TARGETED_UPDATES: usize = 64;

/// Stable owner ID for dirty queues belonging to one rendering pipeline.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PipelineId(u64);

impl PipelineId {
    /// Generate a globally unique pipeline ID.
    pub fn generate() -> Self {
        static NEXT_PIPELINE_ID: IdAllocator = IdAllocator::new();
        Self(NEXT_PIPELINE_ID.allocate())
    }

    /// Create a pipeline ID from a raw numeric value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the raw numeric value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Default for PipelineId {
    fn default() -> Self {
        Self(0)
    }
}

/// Context passed while mounting an element tree.
#[derive(Clone, Copy, Debug)]
pub struct MountContext {
    pipeline_id: PipelineId,
}

impl MountContext {
    /// Create a mount context for a rendering pipeline.
    pub const fn new(pipeline_id: PipelineId) -> Self {
        Self { pipeline_id }
    }

    /// Return the owning pipeline ID.
    pub const fn pipeline_id(self) -> PipelineId {
        self.pipeline_id
    }
}

#[derive(Default)]
struct DirtyQueues {
    build: BTreeSet<ElementId>,
    transactions: BTreeMap<ElementId, crate::animation::Transaction>,
    layout: BTreeSet<ElementId>,
    paint: BTreeSet<ElementId>,
    self_paint: BTreeSet<ElementId>,
    children: BTreeSet<ElementId>,
    composite: alloc::vec::Vec<ElementId>,
}

/// Global dirty element IDs for State change callbacks, partitioned by pipeline.
static GLOBAL_DIRTY: Mutex<BTreeMap<PipelineId, DirtyQueues>> = Mutex::new(BTreeMap::new());

/// Mark an element as dirty for rebuild (called from ComponentElement callbacks)
///
/// This function is called from State change callbacks in ComponentElement
/// to notify the PipelineOwner that an element needs to be rebuilt.
pub fn mark_element_dirty(owner: PipelineId, id: ElementId) {
    if crate::debug::is_enabled() {
        println!(
            "[PipelineOwner] mark_element_dirty: owner={}, id={}",
            owner.get(),
            id.get()
        );
    }
    let transaction = crate::animation::current_transaction();
    let mut queues = GLOBAL_DIRTY.lock();
    let queue = queues.entry(owner).or_default();
    queue.build.insert(id);
    if transaction == crate::animation::Transaction::default() {
        queue.transactions.remove(&id);
    } else {
        queue.transactions.insert(id, transaction);
    }
    if crate::debug::is_enabled() {
        println!("[PipelineOwner] Dirty count: {}", queue.build.len());
    }
}

/// Mark an element as needing paint only (no build/layout).
pub fn mark_element_needs_paint(owner: PipelineId, id: ElementId) {
    let mut queues = GLOBAL_DIRTY.lock();
    queues.entry(owner).or_default().paint.insert(id);
}

/// Virtualization changed the mounted children without changing retained rows.
pub fn mark_element_children_changed(owner: PipelineId, id: ElementId) {
    GLOBAL_DIRTY
        .lock()
        .entry(owner)
        .or_default()
        .children
        .insert(id);
}

/// Mark an element as needing retained recomposition only.
pub fn mark_element_needs_composite(owner: PipelineId, id: ElementId) {
    let mut queues = GLOBAL_DIRTY.lock();
    let composite = &mut queues.entry(owner).or_default().composite;
    if !composite.contains(&id) {
        composite.push(id);
    }
}

/// Mark an element as needing layout and paint.
pub fn mark_element_needs_layout(owner: PipelineId, id: ElementId) {
    let mut queues = GLOBAL_DIRTY.lock();
    queues.entry(owner).or_default().layout.insert(id);
}

/// Mark an element as needing paint for its own buffer only.
///
/// This is for render objects whose internal state changed without changing
/// their descendants, such as a window titlebar hover state.
pub fn mark_element_needs_self_paint(owner: PipelineId, id: ElementId) {
    let mut queues = GLOBAL_DIRTY.lock();
    queues.entry(owner).or_default().self_paint.insert(id);
}

pub(crate) fn clear_global_dirty(owner: PipelineId) {
    GLOBAL_DIRTY.lock().remove(&owner);
}

pub(crate) fn has_global_dirty(owner: PipelineId) -> bool {
    GLOBAL_DIRTY.lock().get(&owner).is_some_and(|queue| {
        !queue.build.is_empty()
            || !queue.layout.is_empty()
            || !queue.paint.is_empty()
            || !queue.self_paint.is_empty()
            || !queue.children.is_empty()
            || !queue.composite.is_empty()
    })
}

/// Dirty flags for different render phases
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum DirtyPhase {
    Build,
    Layout,
    Paint,
}

/// PipelineOwner manages dirty flags and orchestrates render phases
///
/// This is inspired by Flutter's PipelineOwner and manages the three
/// main phases of the rendering pipeline:
/// - Build: Rebuild Elements from Views
/// - Layout: Recalculate positions and sizes
/// - Paint: Repaint to buffers
pub struct PipelineOwner {
    /// Dirty queue owner ID.
    pipeline_id: PipelineId,
    /// Elements that need rebuilding (State changed)
    dirty_build: BTreeSet<ElementId>,
    build_transactions: BTreeMap<ElementId, crate::animation::Transaction>,
    /// Elements that need relayouting
    dirty_layout: BTreeSet<ElementId>,
    /// Elements that need repainting
    dirty_paint: BTreeSet<ElementId>,
    /// Elements whose own buffers need repainting without repainting descendants
    dirty_self_paint: BTreeSet<ElementId>,
    dirty_children: BTreeSet<ElementId>,
    last_children_ids: alloc::vec::Vec<ElementId>,
    /// Elements whose retained layers only need recomposition.
    dirty_composite: alloc::vec::Vec<ElementId>,
    /// Elements repainted in the last flush
    last_paint_ids: alloc::vec::Vec<ElementId>,
    /// Elements receiving only a self-paint in the last flush
    last_self_paint_ids: alloc::vec::Vec<ElementId>,
    /// Elements recomposited in the last flush
    last_composite_ids: alloc::vec::Vec<ElementId>,
    /// State registry for managing State instances
    state_registry: StateRegistry,
}

impl PipelineOwner {
    /// Create a new PipelineOwner
    pub fn new() -> Self {
        Self::with_pipeline_id(PipelineId::default())
    }

    /// Create a PipelineOwner for a specific rendering pipeline.
    pub fn with_pipeline_id(pipeline_id: PipelineId) -> Self {
        Self {
            pipeline_id,
            dirty_build: BTreeSet::new(),
            build_transactions: BTreeMap::new(),
            dirty_layout: BTreeSet::new(),
            dirty_paint: BTreeSet::new(),
            dirty_self_paint: BTreeSet::new(),
            dirty_children: BTreeSet::new(),
            last_children_ids: alloc::vec::Vec::new(),
            dirty_composite: alloc::vec::Vec::new(),
            last_paint_ids: alloc::vec::Vec::new(),
            last_self_paint_ids: alloc::vec::Vec::new(),
            last_composite_ids: alloc::vec::Vec::new(),
            state_registry: StateRegistry::new(),
        }
    }

    /// Mark an element as dirty for a specific phase
    pub fn mark_dirty(&mut self, id: ElementId, phase: DirtyPhase) {
        match phase {
            DirtyPhase::Build => {
                self.dirty_build.insert(id);
                let transaction = crate::animation::current_transaction();
                if transaction == crate::animation::Transaction::default() {
                    self.build_transactions.remove(&id);
                } else {
                    self.build_transactions.insert(id, transaction);
                }
            }
            DirtyPhase::Layout => {
                self.dirty_layout.insert(id);
                // Layout implies paint
                self.dirty_paint.insert(id);
            }
            DirtyPhase::Paint => {
                self.dirty_paint.insert(id);
            }
        }
    }

    /// Mark an element as needing a rebuild
    pub fn mark_needs_build(&mut self, id: ElementId) {
        self.mark_dirty(id, DirtyPhase::Build);
    }

    /// Mark an element as needing layout
    pub fn mark_needs_layout(&mut self, id: ElementId) {
        self.mark_dirty(id, DirtyPhase::Layout);
    }

    /// Mark an element as needing paint
    pub fn mark_needs_paint(&mut self, id: ElementId) {
        self.mark_dirty(id, DirtyPhase::Paint);
    }

    /// Mark an element as needing retained recomposition only.
    pub fn mark_needs_composite(&mut self, id: ElementId) {
        if !self.dirty_composite.contains(&id) {
            self.dirty_composite.push(id);
        }
    }

    /// Mark an element as needing only its own buffer repainted.
    pub fn mark_needs_self_paint(&mut self, id: ElementId) {
        self.dirty_self_paint.insert(id);
    }

    /// Check if there's any dirty work
    pub fn has_dirty(&self) -> bool {
        has_global_dirty(self.pipeline_id)
            || !self.dirty_build.is_empty()
            || !self.dirty_layout.is_empty()
            || !self.dirty_paint.is_empty()
            || !self.dirty_self_paint.is_empty()
            || !self.dirty_children.is_empty()
            || !self.dirty_composite.is_empty()
    }

    /// Flush all dirty phases
    ///
    /// This processes build, layout, and paint in order.
    pub fn flush(&mut self, element_tree: &mut ElementTree, window_size: Size) {
        self.flush_with_legacy_paint(element_tree, window_size, true);
    }

    /// Flush all dirty phases, optionally rendering legacy element buffers.
    pub fn flush_with_legacy_paint(
        &mut self,
        element_tree: &mut ElementTree,
        window_size: Size,
        render_legacy_paint: bool,
    ) {
        self.flush_before_paint(element_tree, window_size, render_legacy_paint, |_, _| {});
    }

    /// Reconcile input ownership after rebuilding/layout and before recording
    /// paint, so caret state and event routing describe the same frame.
    pub(crate) fn flush_before_paint(
        &mut self,
        element_tree: &mut ElementTree,
        window_size: Size,
        render_legacy_paint: bool,
        before_paint: impl FnOnce(&mut ElementTree, bool),
    ) {
        self.collect_global_dirty();

        // A composite-only update changes placement, not focus claims or
        // mounted input controls. Reconciliation walks the tree; reserve it for
        // content/geometry changes instead of every scroll animation frame.
        let structure_changed = !self.dirty_build.is_empty()
            || !self.dirty_layout.is_empty()
            || !self.dirty_children.is_empty();
        let content_changed = !self.dirty_build.is_empty()
            || !self.dirty_layout.is_empty()
            || !self.dirty_paint.is_empty()
            || !self.dirty_self_paint.is_empty()
            || !self.dirty_children.is_empty();

        // 1. Build Phase: Rebuild Elements whose State changed
        self.flush_build(element_tree);
        self.collect_global_dirty();

        // 2. Layout Phase: Recalculate layout
        self.flush_layout(element_tree, window_size);

        if structure_changed {
            crate::element::invalidate_scroll_structure();
        }
        before_paint(
            element_tree,
            content_changed || self.dirty_composite.is_empty(),
        );
        self.collect_global_dirty();

        // 3. Paint Phase: Repaint dirty elements
        self.flush_paint(element_tree, render_legacy_paint);
    }

    fn collect_global_dirty(&mut self) {
        let mut queues = GLOBAL_DIRTY.lock();
        let queue = queues.entry(self.pipeline_id).or_default();
        let dirty_build = core::mem::take(&mut queue.build);
        let transactions = core::mem::take(&mut queue.transactions);
        let dirty_layout = core::mem::take(&mut queue.layout);
        let dirty_paint = core::mem::take(&mut queue.paint);
        let dirty_self_paint = core::mem::take(&mut queue.self_paint);
        self.dirty_children.append(&mut queue.children);

        for id in dirty_build {
            self.dirty_build.insert(id);
            self.build_transactions.remove(&id);
            if let Some(transaction) = transactions.get(&id) {
                self.build_transactions.insert(id, *transaction);
            }
        }
        for id in dirty_layout {
            self.mark_needs_layout(id);
        }
        for id in dirty_paint {
            self.mark_needs_paint(id);
        }
        for id in dirty_self_paint {
            self.mark_needs_self_paint(id);
        }
        for id in queue.composite.drain(..) {
            self.mark_needs_composite(id);
        }
    }

    /// Flush the build phase
    fn flush_build(&mut self, element_tree: &mut ElementTree) {
        let dirty_build = core::mem::take(&mut self.dirty_build);
        let transactions = core::mem::take(&mut self.build_transactions);
        let rebuilt = !dirty_build.is_empty();

        crate::animation::pending::with_pending_builds(
            self.pipeline_id,
            &dirty_build,
            &transactions,
            || {
                if dirty_build.len() > MAX_TARGETED_UPDATES {
                    fn rebuild(
                        owner: PipelineId,
                        element: &mut dyn crate::element::Element,
                        dirty: &BTreeSet<ElementId>,
                        transactions: &BTreeMap<ElementId, crate::animation::Transaction>,
                        layout: &mut BTreeSet<ElementId>,
                    ) {
                        if dirty.contains(&element.id())
                            && matches!(
                                crate::animation::pending::with_element_transaction(
                                    owner,
                                    transactions.get(&element.id()).copied().unwrap_or_default(),
                                    || element.rebuild()
                                ),
                                crate::element::UpdateResult::Updated
                            )
                        {
                            layout.insert(element.id());
                        }
                        // Reconciliation can remove or replace children. Visit the
                        // resulting live children, never references saved before rebuild.
                        for child in element.children_mut() {
                            rebuild(owner, child.as_mut(), dirty, transactions, layout);
                        }
                    }
                    if let Some(root) = element_tree.root_mut() {
                        rebuild(
                            self.pipeline_id,
                            root.as_mut(),
                            &dirty_build,
                            &transactions,
                            &mut self.dirty_layout,
                        );
                    }
                } else {
                    for id in dirty_build.iter().copied() {
                        // Find the element in the tree
                        if let Some(element) = element_tree.find_element_mut(id) {
                            // Rebuild a fresh View description and reconcile it with the
                            // retained child Element/RenderObject subtree.
                            if matches!(
                                crate::animation::pending::with_element_transaction(
                                    self.pipeline_id,
                                    transactions.get(&id).copied().unwrap_or_default(),
                                    || element.rebuild()
                                ),
                                crate::element::UpdateResult::Updated
                            ) {
                                self.mark_needs_layout(id);
                            }
                        }
                    }
                }
            },
        );
        if rebuilt && element_tree.safe_area_insets() != crate::geometry::EdgeInsets::ZERO {
            element_tree.apply_safe_area_insets();
        }
    }

    /// Flush the layout phase
    fn flush_layout(&mut self, element_tree: &mut ElementTree, window_size: Size) {
        let dirty_layout = core::mem::take(&mut self.dirty_layout);
        if dirty_layout.is_empty() {
            return;
        }

        // Create constraints from window size
        let constraints = LayoutConstraints::loose(window_size.width, window_size.height);

        // One conservative layout also coalesces overlapping dirty subtrees.
        // Finding an ancestor path separately for every node is quadratic for
        // dense updates, before any actual layout has even taken place.
        if dirty_layout.len() > MAX_TARGETED_UPDATES {
            element_tree.layout(constraints);
            if let Some(root) = element_tree.root() {
                self.dirty_paint.insert(root.id());
            }
            return;
        }

        let root_id = match element_tree.root() {
            Some(root) => root.id(),
            None => return,
        };

        let mut layout_roots = BTreeSet::new();
        let mut full_layout = false;

        for id in dirty_layout {
            if id == root_id {
                full_layout = true;
                break;
            }

            let path = match element_tree.find_path_ids(id) {
                Some(path) => path,
                None => {
                    full_layout = true;
                    break;
                }
            };

            if path.len() < 2 {
                full_layout = true;
                break;
            }

            let mut chosen_id = path[path.len() - 2];
            let mut has_constraints = false;

            for ancestor_id in path[..path.len() - 1].iter().rev() {
                let constraints_opt = element_tree
                    .find_element_mut(*ancestor_id)
                    .and_then(|element| element.last_layout_constraints());
                let Some(ancestor_constraints) = constraints_opt else {
                    has_constraints = false;
                    break;
                };

                chosen_id = *ancestor_id;
                has_constraints = true;

                // Virtual content deliberately exceeds tight viewport
                // constraints. Its size changes must reach ScrollView so the
                // content extent and offsets are remeasured and clamped.
                let size = element_tree
                    .find_element(*ancestor_id)
                    .map(|element| element.bounds().size);
                if ancestor_constraints.is_tight()
                    && size.is_some_and(|size| {
                        size.width == ancestor_constraints.max_width
                            && size.height == ancestor_constraints.max_height
                    })
                {
                    break;
                }
            }

            if !has_constraints {
                full_layout = true;
                break;
            }

            layout_roots.insert(chosen_id);
        }

        if full_layout || layout_roots.is_empty() {
            element_tree.layout(constraints);
            if let Some(root) = element_tree.root() {
                self.dirty_paint.insert(root.id());
            }
            return;
        }

        let mut fallback_full = false;
        for id in layout_roots.iter().copied() {
            if let Some(element) = element_tree.find_element_mut(id) {
                if let Some(local_constraints) = element.last_layout_constraints() {
                    let before = element.bounds().size;
                    let after = element.layout(local_constraints);
                    if before != after
                        && local_constraints.is_tight()
                        && (after.width != local_constraints.max_width
                            || after.height != local_constraints.max_height)
                    {
                        // Content that previously fit may have grown. A full
                        // layout propagates this new extent to its scroll owner.
                        fallback_full = true;
                        break;
                    }
                    self.dirty_paint.insert(id);
                } else {
                    fallback_full = true;
                    break;
                }
            } else {
                fallback_full = true;
                break;
            }
        }

        if fallback_full {
            element_tree.layout(constraints);
            if let Some(root) = element_tree.root() {
                self.dirty_paint.insert(root.id());
            }
        } else {
            for id in layout_roots {
                element_tree.flush_layout_notifications_for(id);
            }
        }
    }

    /// Flush the paint phase
    fn flush_paint(&mut self, element_tree: &mut ElementTree, render_legacy_paint: bool) {
        let mut dirty_paint = core::mem::take(&mut self.dirty_paint);
        self.last_children_ids.clear();
        let mut children_changed = core::mem::take(&mut self.dirty_children);
        let owners: alloc::vec::Vec<_> = children_changed.iter().copied().collect();
        for id in owners {
            if let Some(element) = element_tree.find_element(id) {
                children_changed.extend(element.children().iter().map(|child| child.id()));
            }
        }
        self.last_children_ids.extend(
            children_changed
                .iter()
                .copied()
                .filter(|id| !dirty_paint.contains(id) && !self.dirty_self_paint.contains(id)),
        );
        dirty_paint.append(&mut children_changed);
        let dirty_self_paint = core::mem::take(&mut self.dirty_self_paint);
        self.last_paint_ids.clear();
        self.last_self_paint_ids.clear();
        self.last_composite_ids.clear();
        self.last_paint_ids.extend(dirty_paint.iter().copied());
        self.last_paint_ids.extend(dirty_self_paint.iter().copied());
        self.last_self_paint_ids.extend(
            dirty_self_paint
                .iter()
                .copied()
                .filter(|id| !dirty_paint.contains(id)),
        );
        self.last_composite_ids.extend(
            self.dirty_composite
                .iter()
                .copied()
                .filter(|id| !dirty_paint.contains(id) && !dirty_self_paint.contains(id)),
        );
        self.dirty_composite.clear();

        if !render_legacy_paint {
            if dirty_paint.len() + dirty_self_paint.len() > MAX_TARGETED_UPDATES {
                fn render_dirty(
                    element: &mut dyn crate::element::Element,
                    subtree_ids: &BTreeSet<ElementId>,
                    self_ids: &BTreeSet<ElementId>,
                    inherited: bool,
                ) {
                    let subtree = inherited || subtree_ids.contains(&element.id());
                    let own = subtree || self_ids.contains(&element.id());
                    for child in element.children_mut() {
                        render_dirty(child.as_mut(), subtree_ids, self_ids, subtree);
                    }
                    if own {
                        let requires_buffer = element
                            .render_object()
                            .is_some_and(|object| object.requires_buffer_render_for_paint());
                        if requires_buffer {
                            if subtree {
                                if let Some(object) = element.render_object_mut() {
                                    object.render();
                                }
                            } else {
                                element.render();
                            }
                        }
                    }
                }
                if let Some(root) = element_tree.root_mut() {
                    render_dirty(root.as_mut(), &dirty_paint, &dirty_self_paint, false);
                }
                return;
            }
            for id in dirty_paint.iter().copied() {
                if let Some(element) = element_tree.find_element_mut(id) {
                    Self::render_paint_buffers_recursive(element);
                }
            }
            for id in dirty_self_paint.iter().copied() {
                if dirty_paint.contains(&id) {
                    continue;
                }
                if let Some(element) = element_tree.find_element_mut(id) {
                    let requires_buffer = element.render_object().is_some_and(|render_object| {
                        render_object.requires_buffer_render_for_paint()
                    });
                    if requires_buffer {
                        element.render();
                    }
                }
            }
            return;
        }

        if crate::debug::is_enabled() {
            crate::logln!(
                "[PipelineOwner] flush_paint: {} dirty elements",
                dirty_paint.len()
            );
        }

        for id in dirty_paint {
            if crate::debug::is_enabled() {
                crate::logln!(
                    "[PipelineOwner] flush_paint: rendering element id={}",
                    id.get()
                );
            }
            // Find the element and call render()
            if let Some(element) = element_tree.find_element_mut(id) {
                // Render this element and all its descendants
                // Containers like VStack/HStack don't have buffers, but their children do
                Self::render_recursive(element, 0);
            }
        }

        for id in dirty_self_paint {
            if crate::debug::is_enabled() {
                crate::logln!(
                    "[PipelineOwner] flush_paint: rendering self element id={}",
                    id.get()
                );
            }
            if let Some(element) = element_tree.find_element_mut(id) {
                element.render();
            }
        }
    }

    /// Recursively render an element and all its descendants
    fn render_recursive(element: &mut Box<dyn crate::element::Element>, depth: usize) {
        let indent = "  ".repeat(depth);
        let type_name = element.type_name_debug();
        let has_buffer = element.get_buffer().is_some();

        if crate::debug::is_enabled() {
            crate::logln!(
                "[PipelineOwner] {}render: {} (buffer={})",
                indent,
                type_name,
                has_buffer
            );
        }

        // Render children before the parent so parent renderers that composite
        // child buffers, such as WindowRenderElement, see fresh child content.
        let children = element.children();
        if crate::debug::is_enabled() {
            crate::logln!("[PipelineOwner] {}has {} children", indent, children.len());
        }

        for (i, child) in element.children_mut().iter_mut().enumerate() {
            if crate::debug::is_enabled() {
                crate::logln!(
                    "[PipelineOwner] {}child #{}: {}",
                    indent,
                    i,
                    child.type_name_debug()
                );
            }
            Self::render_recursive(child, depth + 1);
        }

        // Render this element
        element.render();
    }

    /// Render only legacy buffers that are still referenced by PaintCommand output.
    fn render_paint_buffers_recursive(element: &mut Box<dyn crate::element::Element>) {
        for child in element.children_mut().iter_mut() {
            Self::render_paint_buffers_recursive(child);
        }

        let requires_buffer = element
            .render_object()
            .is_some_and(|render_object| render_object.requires_buffer_render_for_paint());
        if requires_buffer {
            if let Some(render_object) = element.render_object_mut() {
                render_object.render();
            }
        }
    }

    /// Get the StateRegistry
    pub fn state_registry(&self) -> &StateRegistry {
        &self.state_registry
    }

    /// Get mutable reference to the StateRegistry
    pub fn state_registry_mut(&mut self) -> &mut StateRegistry {
        &mut self.state_registry
    }

    /// Check if there are any dirty build elements
    pub fn has_dirty_build(&self) -> bool {
        !self.dirty_build.is_empty()
    }

    /// Check if there are any dirty layout elements
    pub fn has_dirty_layout(&self) -> bool {
        !self.dirty_layout.is_empty()
    }

    /// Check if there are any dirty paint elements
    pub fn has_dirty_paint(&self) -> bool {
        !self.dirty_paint.is_empty()
            || !self.dirty_self_paint.is_empty()
            || !self.dirty_children.is_empty()
    }

    /// Check if there are any composite-only dirty elements.
    pub fn has_dirty_composite(&self) -> bool {
        !self.dirty_composite.is_empty()
    }

    /// Structural updates that can reuse existing children's display lists.
    pub fn last_children_ids(&self) -> &[ElementId] {
        &self.last_children_ids
    }

    /// Get the IDs repainted in the last flush.
    pub fn last_paint_ids(&self) -> &[ElementId] {
        &self.last_paint_ids
    }

    /// Get the IDs whose last repaint affected only their own rendered pixels.
    ///
    /// # Returns
    ///
    /// Element IDs that were self-painted without also receiving a regular
    /// subtree paint in the last flush.
    pub fn last_self_paint_ids(&self) -> &[ElementId] {
        &self.last_self_paint_ids
    }

    /// Get the IDs recomposited in the last flush.
    pub fn last_composite_ids(&self) -> &[ElementId] {
        &self.last_composite_ids
    }

    /// Register a State instance
    ///
    /// This is a convenience method that forwards to the StateRegistry.
    /// Use this to register States when they are first created.
    pub fn register_state<T: 'static + Send + Sync>(&mut self, state: State<T>) -> StateId {
        self.state_registry.register(state)
    }

    /// Get a State from the registry by ID
    ///
    /// This is a convenience method that forwards to the StateRegistry.
    /// Returns a cloned State that shares data with the original.
    pub fn get_state<T: 'static + Clone>(&self, id: StateId) -> Option<State<T>> {
        self.state_registry.get(id)
    }

    /// Get a State reference from the registry by ID
    ///
    /// This is a convenience method that forwards to the StateRegistry.
    pub fn get_state_ref<T: 'static>(&self, id: StateId) -> Option<&State<T>> {
        self.state_registry.get_ref(id)
    }
}

impl Default for PipelineOwner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod dense_update_tests {
    use super::*;
    use crate::element::{Element, ElementRenderObject as RenderObject, UpdateResult};
    use crate::view::View;
    use alloc::{rc::Rc, vec::Vec};
    use core::{any::Any, cell::RefCell};

    #[derive(Clone)]
    struct AnimatedChild {
        expanded: State<bool>,
        key: crate::view::ViewKey,
        local_animation: Option<crate::animation::Animation>,
    }

    impl AnimatedChild {
        fn body(&self) -> Box<dyn View> {
            use crate::view::ViewExt;
            let expanded = self.expanded.get();
            let frame =
                crate::views::Spacer::new().frame(if expanded { 100.0 } else { 20.0 }, 20.0);
            match self.local_animation {
                Some(animation) => Box::new(frame.animation(animation, expanded)),
                None => Box::new(frame),
            }
        }
    }

    impl View for AnimatedChild {
        fn create_element(&self) -> Box<dyn Element> {
            Box::new(crate::element::ComponentElement::new_with_builder(
                self.clone(),
                Self::body,
            ))
        }
        fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
            alloc::vec![&self.expanded]
        }
        fn key(&self) -> Option<&crate::view::ViewKey> {
            Some(&self.key)
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum ChildReconciliation {
        Single,
        StableSiblings,
        KeyedMove,
    }

    #[derive(Clone)]
    struct AnimatedParent {
        changed: State<bool>,
        child: AnimatedChild,
        sibling: AnimatedChild,
        mode: ChildReconciliation,
    }

    impl AnimatedParent {
        fn body(&self) -> Box<dyn View> {
            match self.mode {
                ChildReconciliation::Single => Box::new(self.child.clone()),
                ChildReconciliation::KeyedMove if self.changed.get() => Box::new(
                    crate::views::HStack::new((self.sibling.clone(), self.child.clone())),
                ),
                _ => Box::new(crate::views::HStack::new((
                    self.child.clone(),
                    self.sibling.clone(),
                ))),
            }
        }
    }

    impl View for AnimatedParent {
        fn create_element(&self) -> Box<dyn Element> {
            Box::new(crate::element::ComponentElement::new_with_builder(
                self.clone(),
                Self::body,
            ))
        }
        fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
            alloc::vec![&self.changed]
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn animated_child_id(element: &dyn Element, state: StateId) -> Option<ElementId> {
        if let Some(component) = element
            .as_any()
            .downcast_ref::<crate::element::ComponentElement<AnimatedChild>>()
            && component.view().expanded.id() == state
        {
            return Some(element.id());
        }
        element
            .children()
            .iter()
            .find_map(|child| animated_child_id(child.as_ref(), state))
    }

    fn animated_frame(
        element: &mut dyn Element,
    ) -> Option<&mut crate::views::modifiers::FrameRenderObject> {
        if element.render_object().is_some_and(|object| {
            object
                .as_any()
                .is::<crate::views::modifiers::FrameRenderObject>()
        }) {
            return element.render_object_mut()?.as_any_mut().downcast_mut();
        }
        element
            .children_mut()
            .iter_mut()
            .find_map(|child| animated_frame(child.as_mut()))
    }

    #[test]
    fn queued_descendant_policy_is_used_on_first_update_in_sparse_and_dense_builds() {
        use crate::animation::{Animation, Transaction, with_animation, with_transaction};
        use crate::state::generate_state_id;
        use core::time::Duration;

        let parent_animation = Animation::linear(Duration::from_millis(400));
        let child_animation = Animation::linear(Duration::from_millis(100));
        let local_animation = Animation::linear(Duration::from_millis(200));
        let policies = [
            Transaction {
                animation: Some(child_animation),
                disables_animations: false,
            },
            Transaction::default(),
            Transaction {
                animation: None,
                disables_animations: true,
            },
        ];
        for dense in [false, true] {
            for mode in [
                ChildReconciliation::Single,
                ChildReconciliation::StableSiblings,
                ChildReconciliation::KeyedMove,
            ] {
                for parent_policy in [None, Some(parent_animation)] {
                    for local in [None, Some(local_animation)] {
                        for policy in policies {
                            let child = AnimatedChild {
                                expanded: State::new(generate_state_id(), false),
                                key: 1usize.into(),
                                local_animation: local,
                            };
                            let parent = AnimatedParent {
                                changed: State::new(generate_state_id(), false),
                                child: child.clone(),
                                sibling: AnimatedChild {
                                    expanded: State::new(generate_state_id(), false),
                                    key: 2usize.into(),
                                    local_animation: None,
                                },
                                mode,
                            };
                            let owner_id = PipelineId::generate();
                            let mut owner = PipelineOwner::with_pipeline_id(owner_id);
                            let mut tree = ElementTree::with_pipeline_id(owner_id);
                            tree.set_root(parent.create_element());
                            let root = tree.root().unwrap();
                            let child_id = animated_child_id(root, child.expanded.id()).unwrap();
                            assert!(
                                root.id() < child_id,
                                "sparse traversal must rebuild the ancestor first"
                            );

                            with_animation(parent_policy, || parent.changed.set(true));
                            with_animation(Animation::linear(Duration::from_millis(800)), || {
                                child.expanded.set(true)
                            });
                            owner.collect_global_dirty();
                            // A later notification, even with the same value,
                            // must replace an already-collected animation policy.
                            with_transaction(policy, || child.expanded.set(true));
                            owner.collect_global_dirty();
                            if dense {
                                // Stale IDs force the live-tree traversal branch.
                                for _ in 0..MAX_TARGETED_UPDATES {
                                    owner.mark_needs_build(ElementId::generate());
                                }
                            }
                            owner.flush_build(&mut tree);

                            let frame =
                                animated_frame(tree.find_element_mut(child_id).unwrap().as_mut())
                                    .unwrap();
                            let expected = if policy.disables_animations {
                                100.0
                            } else if local.is_some() {
                                40.0
                            } else if policy.animation.is_some() {
                                60.0
                            } else {
                                100.0
                            };
                            frame.advance_visual_animation(Duration::from_millis(50));
                            assert_eq!(
                                frame.width_value(),
                                Some(expected),
                                "dense={dense} mode={mode:?} parent={parent_policy:?} child={policy:?} local={local:?}"
                            );
                            // The frame ignores identical targets, so a late
                            // child rebuild cannot repair a wrong first policy.
                            tree.clear_root();
                            clear_global_dirty(owner_id);
                        }
                    }
                }
            }
        }
    }

    #[derive(Default)]
    struct Calls {
        visits: usize,
        rebuilt: Vec<ElementId>,
        rendered: Vec<ElementId>,
        layouts: usize,
    }
    struct Probe {
        id: ElementId,
        children: Vec<Box<dyn Element>>,
        calls: Rc<RefCell<Calls>>,
        remove_children_on_rebuild: bool,
    }
    impl Probe {
        fn new(calls: &Rc<RefCell<Calls>>) -> Self {
            Self {
                id: ElementId::generate(),
                children: Vec::new(),
                calls: calls.clone(),
                remove_children_on_rebuild: false,
            }
        }
    }
    impl Element for Probe {
        fn id(&self) -> ElementId {
            self.calls.borrow_mut().visits += 1;
            self.id
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn children(&self) -> &[Box<dyn Element>] {
            &self.children
        }
        fn children_mut(&mut self) -> &mut [Box<dyn Element>] {
            &mut self.children
        }
        fn update(&mut self, _: &dyn View) -> UpdateResult {
            UpdateResult::NoChange
        }
        fn rebuild(&mut self) -> UpdateResult {
            self.calls.borrow_mut().rebuilt.push(self.id);
            if self.remove_children_on_rebuild {
                self.children.clear();
            }
            UpdateResult::Updated
        }
        fn layout(&mut self, c: LayoutConstraints) -> Size {
            self.calls.borrow_mut().layouts += 1;
            for child in &mut self.children {
                child.layout(c);
            }
            Size::new(10., 10.)
        }
        fn render(&mut self) {
            self.calls.borrow_mut().rendered.push(self.id);
        }
        fn render_object(&self) -> Option<&dyn RenderObject> {
            Some(self)
        }
        fn render_object_mut(&mut self) -> Option<&mut dyn RenderObject> {
            Some(self)
        }
    }
    impl RenderObject for Probe {
        fn layout(&mut self, c: LayoutConstraints) -> Size {
            Element::layout(self, c)
        }
        fn size(&self) -> Size {
            Size::new(10., 10.)
        }
        fn render(&mut self) {
            Element::render(self);
        }
        fn requires_buffer_render_for_paint(&self) -> bool {
            true
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    #[test]
    fn dense_build_visits_live_tree_once_and_skips_removed_dirty_children() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut root = Probe::new(&calls);
        let mut removed_parent = Probe::new(&calls);
        let removed = Probe::new(&calls);
        let removed_id = removed.id;
        removed_parent.children.push(Box::new(removed));
        removed_parent.remove_children_on_rebuild = true;
        let mut owner = PipelineOwner::new();
        owner.mark_needs_build(removed_parent.id);
        owner.mark_needs_build(removed_id);
        root.children.push(Box::new(removed_parent));
        for _ in 0..128 {
            let child = Probe::new(&calls);
            owner.mark_needs_build(child.id);
            root.children.push(Box::new(child));
        }
        let mut tree = ElementTree::new();
        tree.set_root(Box::new(root));
        owner.flush_build(&mut tree);
        let calls = calls.borrow();
        assert_eq!(calls.rebuilt.len(), 129);
        assert!(!calls.rebuilt.contains(&removed_id));
        assert_eq!(owner.dirty_layout.len(), 129);
        assert!(
            calls.visits < 520,
            "dense rebuild must not search from the root per ID: {}",
            calls.visits
        );
    }

    #[test]
    fn dense_paint_renders_overlapping_subtrees_once_and_preserves_self_only_scope() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut root = Probe::new(&calls);
        let mut owner = PipelineOwner::new();
        let mut expected = BTreeSet::new();
        for _ in 0..128 {
            let mut parent = Probe::new(&calls);
            let child = Probe::new(&calls);
            owner.mark_needs_paint(parent.id);
            owner.mark_needs_paint(child.id);
            expected.insert(parent.id);
            expected.insert(child.id);
            parent.children.push(Box::new(child));
            root.children.push(Box::new(parent));
        }
        let mut self_only = Probe::new(&calls);
        owner.mark_needs_self_paint(self_only.id);
        expected.insert(self_only.id);
        self_only.children.push(Box::new(Probe::new(&calls)));
        root.children.push(Box::new(self_only));
        root.children.push(Box::new(Probe::new(&calls)));
        let mut tree = ElementTree::new();
        tree.set_root(Box::new(root));
        owner.flush_paint(&mut tree, false);
        let calls = calls.borrow();
        assert_eq!(calls.rendered.len(), expected.len());
        assert_eq!(
            calls.rendered.iter().copied().collect::<BTreeSet<_>>(),
            expected
        );
        assert!(
            calls.visits < 1040,
            "dense paint must not search from the root per ID: {}",
            calls.visits
        );
    }

    #[test]
    fn dense_layout_coalesces_overlapping_roots() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut root = Probe::new(&calls);
        let root_id = root.id;
        let mut owner = PipelineOwner::new();
        for _ in 0..128 {
            let child = Probe::new(&calls);
            owner.mark_needs_layout(child.id);
            root.children.push(Box::new(child));
        }
        let mut tree = ElementTree::new();
        tree.set_root(Box::new(root));
        owner.flush_layout(&mut tree, Size::new(100., 100.));
        assert_eq!(calls.borrow().layouts, 129);
        assert!(owner.dirty_paint.contains(&root_id));
    }
}
