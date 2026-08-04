//! Child reconciliation shared by component and render elements.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::pipeline::MountContext;
use crate::view::{View, ViewKey};

use super::{Element, UpdateResult, focused_descendant_path, restore_focus_at_path};

fn keys_match(element: &dyn Element, view: &dyn View) -> bool {
    element.view_key() == view.key()
}

fn replace_child(
    child: &mut Option<Box<dyn Element>>,
    new_view: &dyn View,
    mount_context: Option<MountContext>,
    preserve_focus: bool,
) {
    let focused_path = preserve_focus
        .then(|| {
            child
                .as_ref()
                .and_then(|element| focused_descendant_path(element.as_ref()))
        })
        .flatten();

    if let Some(mut old_child) = child.take()
        && mount_context.is_some()
    {
        old_child.unmount();
    }

    let mut new_child = new_view.create_element();
    if let Some(context) = mount_context {
        new_child.mount(&context);
        if let Some(path) = focused_path.as_deref() {
            restore_focus_at_path(new_child.as_mut(), path);
        }
    }
    *child = Some(new_child);
}

/// Reconcile one optional Element with one optional View description.
pub(crate) fn update_child(
    child: &mut Option<Box<dyn Element>>,
    new_view: Option<&dyn View>,
    mount_context: Option<MountContext>,
) -> UpdateResult {
    match (child.as_mut(), new_view) {
        (None, None) => UpdateResult::NoChange,
        (Some(_), None) => {
            if let Some(mut old_child) = child.take()
                && mount_context.is_some()
            {
                old_child.unmount();
            }
            UpdateResult::Updated
        }
        (None, Some(view)) => {
            replace_child(child, view, mount_context, false);
            UpdateResult::Updated
        }
        (Some(old_child), Some(view)) if keys_match(old_child.as_ref(), view) => {
            match old_child.update(view) {
                UpdateResult::Replaced => {
                    replace_child(child, view, mount_context, true);
                    UpdateResult::Updated
                }
                UpdateResult::Updated => {
                    // Reconciliation can begin at an ancestor outside a
                    // RepaintBoundary while the actual changed RenderObject is
                    // nested inside it. Keep the changed child's identity in
                    // the dirty queues; otherwise only the ancestor is painted
                    // and a retained descendant boundary can legally reuse
                    // stale content.
                    if let Some(context) = mount_context {
                        crate::pipeline::mark_element_needs_layout(
                            context.pipeline_id(),
                            old_child.id(),
                        );
                    }
                    UpdateResult::Updated
                }
                UpdateResult::NoChange => UpdateResult::NoChange,
            }
        }
        (Some(_), Some(view)) => {
            replace_child(child, view, mount_context, false);
            UpdateResult::Updated
        }
    }
}

fn matching_old_child(
    old_children: &[Box<dyn Element>],
    new_key: Option<&ViewKey>,
) -> Option<usize> {
    match new_key {
        Some(key) => old_children
            .iter()
            .position(|child| child.view_key() == Some(key)),
        None => old_children
            .iter()
            .position(|child| child.view_key().is_none()),
    }
}

/// Reconcile sibling Elements with a newly built list of View descriptions.
pub(crate) fn update_children(
    children: &mut Vec<Box<dyn Element>>,
    new_views: Vec<Box<dyn View>>,
    mount_context: Option<MountContext>,
) -> UpdateResult {
    let mut old_children = core::mem::take(children);
    let mut new_children = Vec::with_capacity(new_views.len());
    let mut result = UpdateResult::NoChange;

    for view in new_views {
        let old_index = matching_old_child(&old_children, view.key());
        if old_index.is_some_and(|index| index != 0) {
            result = UpdateResult::Updated;
        }
        let mut child = old_index.map(|index| old_children.remove(index));
        let child_result = update_child(&mut child, Some(view.as_ref()), mount_context);
        if !matches!(child_result, UpdateResult::NoChange) {
            result = UpdateResult::Updated;
        }
        if let Some(child) = child {
            new_children.push(child);
        }
    }

    if !old_children.is_empty() {
        result = UpdateResult::Updated;
    }
    if mount_context.is_some() {
        for mut old_child in old_children {
            old_child.unmount();
        }
    }

    *children = new_children;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::ViewExt;
    use crate::views::Text;

    #[test]
    fn matching_key_preserves_element_identity() {
        let first = Text::new("before").key(7usize);
        let mut child = Some(first.create_element());
        let original_id = child.as_ref().expect("child should exist").id();

        let next = Text::new("after").key(7usize);
        assert!(matches!(
            update_child(&mut child, Some(&next), None),
            UpdateResult::Updated
        ));
        assert_eq!(
            child.as_ref().expect("child should exist").id(),
            original_id
        );
    }

    #[test]
    fn changed_key_replaces_element_identity() {
        let first = Text::new("before").key(7usize);
        let mut child = Some(first.create_element());
        let original_id = child.as_ref().expect("child should exist").id();

        let next = Text::new("after").key(8usize);
        update_child(&mut child, Some(&next), None);

        assert_ne!(
            child.as_ref().expect("child should exist").id(),
            original_id
        );
    }

    #[test]
    fn keyed_sibling_move_preserves_each_identity() {
        let first = Text::new("one").key(1usize);
        let second = Text::new("two").key(2usize);
        let mut children = alloc::vec![first.create_element(), second.create_element()];
        let first_id = children[0].id();
        let second_id = children[1].id();

        let result = update_children(
            &mut children,
            alloc::vec![
                Box::new(Text::new("two updated").key(2usize)),
                Box::new(Text::new("one updated").key(1usize)),
            ],
            None,
        );

        assert!(matches!(result, UpdateResult::Updated));
        assert_eq!(children[0].id(), second_id);
        assert_eq!(children[1].id(), first_id);
    }
}
