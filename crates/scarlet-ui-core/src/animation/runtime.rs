use crate::{
    element::{Element, ElementId, ElementTree},
    os::Mutex,
    pipeline::PipelineId,
};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::time::Duration;

static ACTIVE: Mutex<BTreeMap<PipelineId, BTreeSet<ElementId>>> = Mutex::new(BTreeMap::new());

pub(crate) fn set_active(owner: PipelineId, id: ElementId, active: bool) {
    let mut owners = ACTIVE.lock();
    if active {
        owners.entry(owner).or_default().insert(id);
    } else if let Some(ids) = owners.get_mut(&owner) {
        ids.remove(&id);
        if ids.is_empty() {
            owners.remove(&owner);
        }
    }
}

/// Only active targets are visited. Paths contain IDs, never element pointers;
/// every descent validates the current children before using a cached path.
#[derive(Default)]
pub(crate) struct VisualAnimations {
    paths: BTreeMap<ElementId, Vec<ElementId>>,
}

fn at_path<'a>(element: &'a mut dyn Element, path: &[ElementId]) -> Option<&'a mut dyn Element> {
    if path.first().copied() != Some(element.id()) {
        return None;
    }
    if path.len() == 1 {
        return Some(element);
    }
    let child = element
        .children_mut()
        .iter_mut()
        .find(|child| child.id() == path[1])?;
    at_path(child.as_mut(), &path[1..])
}

impl VisualAnimations {
    pub fn is_active(&self, owner: PipelineId) -> bool {
        ACTIVE.lock().get(&owner).is_some_and(|ids| !ids.is_empty())
    }

    pub fn advance(
        &mut self,
        owner: PipelineId,
        tree: &mut ElementTree,
        elapsed: Duration,
        finish: bool,
    ) -> bool {
        let ids = ACTIVE.lock().get(&owner).cloned().unwrap_or_default();
        self.paths.retain(|id, _| ids.contains(id));
        if ids.len() > 64 {
            // Dense animations must not repeat a root/path search for every
            // sibling. Walk the current tree once, preserving event-safe IDs.
            fn walk(
                element: &mut dyn Element,
                ids: &BTreeSet<ElementId>,
                elapsed: Duration,
                finish: bool,
            ) -> bool {
                let mut changed = false;
                if ids.contains(&element.id()) {
                    changed |= if finish {
                        element.finish_visual_animation()
                    } else {
                        element.advance_visual_animation(elapsed)
                    };
                }
                for child in element.children_mut() {
                    changed |= walk(child.as_mut(), ids, elapsed, finish);
                }
                changed
            }
            self.paths.clear();
            return tree
                .root_mut()
                .is_some_and(|root| walk(root.as_mut(), &ids, elapsed, finish));
        }
        let mut changed = false;
        for id in ids {
            let valid = self.paths.get(&id).is_some_and(|path| {
                tree.root_mut()
                    .is_some_and(|root| at_path(root.as_mut(), path).is_some())
            });
            if !valid {
                let Some(path) = tree.find_path_ids(id) else {
                    set_active(owner, id, false);
                    self.paths.remove(&id);
                    continue;
                };
                self.paths.insert(id, path);
            }
            if let Some(element) = tree
                .root_mut()
                .and_then(|root| at_path(root.as_mut(), &self.paths[&id]))
            {
                changed |= if finish {
                    element.finish_visual_animation()
                } else {
                    element.advance_visual_animation(elapsed)
                };
            }
        }
        changed
    }

    pub fn clear(&mut self, owner: PipelineId) {
        self.paths.clear();
        ACTIVE.lock().remove(&owner);
    }
}
