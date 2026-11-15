//! Cached scene/input facts. Paint invalidations refresh only their live branch;
//! layout and external tree access retain a conservative full-tree fallback.
use super::owner::PrepaintChanges;
use crate::element::{Element, ElementId, ElementTree};
use alloc::vec::Vec;
use hashbrown::HashMap;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Facts {
    backdrop: bool,
    extension: bool,
    opacity: bool,
    focusable: bool,
    focused: bool,
    popup: bool,
    scroll: (bool, bool, bool),
}
impl Facts {
    fn read(element: &dyn Element) -> Self {
        let object = element.render_object();
        Self {
            backdrop: object.is_some_and(|r| r.reads_backdrop()),
            extension: object.is_some_and(|r| r.emits_paint_extension()),
            opacity: object.is_some_and(|r| r.has_opacity_scope()),
            focusable: element.accepts_keyboard_focus(),
            focused: element.wants_keyboard_focus(),
            scroll: (
                element.view_key().is_some(),
                object.is_some_and(|r| r.depends_on_viewport()),
                object.is_some_and(|r| r.owns_child_viewport()),
            ),
            popup: object
                .and_then(|r| {
                    r.as_any()
                        .downcast_ref::<crate::views::SelectRenderObject>()
                })
                .is_some_and(|r| r.is_expanded()),
        }
    }
}
struct Node {
    parent: Option<ElementId>,
    children: Vec<ElementId>,
    facts: Facts,
}
#[derive(Default)]
pub(super) struct TreeMetadata {
    root: Option<ElementId>,
    nodes: HashMap<ElementId, Node>,
    counts: [usize; 3],
    path_scratch: Vec<ElementId>,
    branch_scratch: Vec<ElementId>,
    refreshed_scratch: Vec<ElementId>,
    #[cfg(test)]
    pub visits: usize,
}
#[derive(Default)]
pub(super) struct Changes {
    pub input: bool,
    pub topology: bool,
    pub popup: bool,
    pub scroll: bool,
}
impl TreeMetadata {
    pub fn clear(&mut self) {
        self.root = None;
        self.nodes.clear();
        self.counts = [0; 3];
    }
    pub fn dependencies(&self) -> (bool, bool, bool) {
        (
            self.counts[0] != 0,
            self.counts[1] != 0,
            self.counts[2] != 0,
        )
    }
    fn count(&mut self, facts: Facts, add: bool) {
        for (total, present) in
            self.counts
                .iter_mut()
                .zip([facts.backdrop, facts.extension, facts.opacity])
        {
            if present {
                if add {
                    *total += 1;
                } else {
                    *total -= 1;
                }
            }
        }
    }
    fn remove(&mut self, id: ElementId, changes: &mut Changes) {
        if let Some(node) = self.nodes.remove(&id) {
            self.count(node.facts, false);
            changes.input |= node.facts.focusable || node.facts.focused;
            changes.popup |= node.facts.popup;
            changes.topology = true;
            for child in node.children {
                if self
                    .nodes
                    .get(&child)
                    .is_some_and(|node| node.parent == Some(id))
                {
                    self.remove(child, changes);
                }
            }
        }
    }
    fn sync_branch(
        &mut self,
        element: &dyn Element,
        parent: Option<ElementId>,
        descend: bool,
        changes: &mut Changes,
    ) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        let id = element.id();
        let facts = Facts::read(element);
        let mut previous = self.nodes.remove(&id);
        let mut topology = previous.as_ref().is_none_or(|node| {
            node.parent != parent
                || !node
                    .children
                    .iter()
                    .copied()
                    .eq(element.children().iter().map(|child| child.id()))
        });
        if let Some(old) = &previous {
            self.count(old.facts, false);
            changes.input |=
                old.facts.focusable != facts.focusable || old.facts.focused != facts.focused;
            changes.popup |= old.facts.popup != facts.popup;
            changes.scroll |= old.facts.scroll != facts.scroll;
            if topology {
                for old_child in &old.children {
                    if !element
                        .children()
                        .iter()
                        .any(|child| child.id() == *old_child)
                        && self
                            .nodes
                            .get(old_child)
                            .is_some_and(|node| node.parent == Some(id))
                    {
                        self.remove(*old_child, changes);
                    }
                }
            }
        } else {
            changes.input |= facts.focusable || facts.focused;
            changes.popup |= facts.popup;
        }
        self.count(facts, true);
        changes.topology |= topology;
        // Self-paint does not promise topology changes, but detect changed direct
        // children anyway so a custom element cannot leave stale scene flags.
        topology |= previous.is_none();
        let mut children = previous
            .as_mut()
            .map_or_else(Vec::new, |node| core::mem::take(&mut node.children));
        if topology {
            children.clear();
            children.extend(element.children().iter().map(|child| child.id()));
        }
        self.nodes.insert(
            id,
            Node {
                parent,
                children,
                facts,
            },
        );
        if descend || topology {
            for child in element.children() {
                self.sync_branch(child.as_ref(), Some(id), true, changes);
            }
        }
    }
    fn depth(&self, id: ElementId) -> usize {
        let mut depth = 0;
        let mut current = Some(id);
        while let Some(id) = current {
            depth += 1;
            let Some(node) = self.nodes.get(&id) else {
                return usize::MAX;
            };
            current = node.parent;
        }
        depth
    }
    fn path_into(&self, tree: &ElementTree, id: ElementId, path: &mut Vec<ElementId>) -> bool {
        path.clear();
        let mut current = Some(id);
        while let Some(id) = current {
            path.push(id);
            let Some(node) = self.nodes.get(&id) else {
                return tree.find_path_ids_into(*path.first().unwrap(), path);
            };
            current = node.parent;
        }
        path.reverse();
        true
    }
    fn rebuild(&mut self, root: &dyn Element) -> Changes {
        if self.root != Some(root.id()) {
            self.clear();
        }
        self.root = Some(root.id());
        let mut changes = Changes::default();
        // A conservative metadata refresh is not itself a structural change.
        // Compare existing facts even on dense paint and ancestry recovery.
        self.sync_branch(root, None, true, &mut changes);
        changes.input |= changes.topology;
        // Do not retain a formerly large virtual page indefinitely after removal.
        if self.nodes.capacity() > self.nodes.len().saturating_mul(4).max(256) {
            self.nodes.shrink_to(self.nodes.len().saturating_mul(2));
        }
        changes
    }
    pub fn sync(&mut self, tree: &ElementTree, scope: &PrepaintChanges) -> Changes {
        let mut changes = Changes::default();
        let Some(root) = tree.root() else {
            changes.topology = self.root.is_some();
            changes.input = changes.topology;
            self.clear();
            return changes;
        };
        if self.root != Some(root.id()) || scope.geometry || scope.branches.len() > 64 {
            return self.rebuild(root);
        }
        // Shallow branches first coalesce parent/child invalidations. Reuse
        // scratch even for ordinary paint, so a motion burst does not replace
        // the removed tree walks with per-frame ancestry allocations.
        let mut branches = core::mem::take(&mut self.branch_scratch);
        branches.clear();
        branches.extend_from_slice(&scope.branches);
        branches.sort_unstable_by_key(|id| self.depth(*id));
        let mut refreshed = core::mem::take(&mut self.refreshed_scratch);
        refreshed.clear();
        let mut path = core::mem::take(&mut self.path_scratch);
        let mut invalid = false;
        for id in &branches {
            if !self.path_into(tree, *id, &mut path) {
                continue;
            }
            if path.iter().any(|id| refreshed.contains(id)) {
                continue;
            }
            if let Some((element, _)) = tree.element_and_absolute_origin_for_path(&path) {
                self.sync_branch(
                    element,
                    path.iter().rev().nth(1).copied(),
                    true,
                    &mut changes,
                );
                refreshed.push(*id);
            } else {
                invalid = true;
                break;
            }
        }
        if !invalid {
            for id in &scope.self_paint {
                if !self.path_into(tree, *id, &mut path) {
                    continue;
                }
                if path.iter().any(|id| refreshed.contains(id)) {
                    continue;
                }
                if let Some((element, _)) = tree.element_and_absolute_origin_for_path(&path) {
                    self.sync_branch(
                        element,
                        path.iter().rev().nth(1).copied(),
                        false,
                        &mut changes,
                    );
                } else {
                    invalid = true;
                    break;
                }
            }
        }
        self.path_scratch = path;
        self.branch_scratch = branches;
        self.refreshed_scratch = refreshed;
        // Reparenting/custom reconciliation invalidates cached ancestry. Do one
        // conservative refresh rather than silently trusting an obsolete route.
        if invalid {
            return self.rebuild(root);
        }
        changes.input |= changes.topology;
        changes
    }
}
