//! Value-triggered animation policy for updates to a child subtree.

use crate::animation::{Animation, Transaction};
use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult,
};
use crate::geometry::{Point, Size};
use crate::view::View;
use alloc::{boxed::Box, vec};
use core::any::Any;

/// Override the child update's animation only when `value` changes.
///
/// The trigger value is independent of stable sibling identity. This wrapper
/// owns no state subscriptions; the wrapped view continues to own its state.
#[derive(Clone)]
pub struct AnimationScope<V: View, K: Clone + PartialEq + 'static> {
    inner: V,
    animation: Option<Animation>,
    value: K,
}

impl<V: View, K: Clone + PartialEq + 'static> AnimationScope<V, K> {
    pub fn new(inner: V, animation: Option<Animation>, value: K) -> Self {
        Self {
            inner,
            animation,
            value,
        }
    }

    pub fn inner(&self) -> &V {
        &self.inner
    }
}

impl<V: View + Clone, K: Clone + PartialEq + 'static> View for AnimationScope<V, K> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| AnimationScopeRenderObject::new(view.value.clone()),
            |render, view| {
                render.update_scope(view.animation, &view.value);
                // The policy has no paint or geometry of its own. Children are
                // still reconciled using child_update_transaction below.
                UpdateResult::NoChange
            },
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct AnimationScopeRenderObject<K: Clone + PartialEq + 'static> {
    previous_value: K,
    // None inherits; Some(None) explicitly removes the inherited animation.
    animation_override: Option<Option<Animation>>,
    size: Size,
}

impl<K: Clone + PartialEq + 'static> AnimationScopeRenderObject<K> {
    fn new(value: K) -> Self {
        Self {
            previous_value: value,
            // Initial mounting never introduces an implicit animation.
            animation_override: None,
            size: Size::ZERO,
        }
    }

    fn update_scope(&mut self, animation: Option<Animation>, value: &K) {
        self.animation_override = if self.previous_value != *value {
            self.previous_value = value.clone();
            Some(animation)
        } else {
            None
        };
    }
}

impl<K: Clone + PartialEq + 'static> ElementRenderObject for AnimationScopeRenderObject<K> {
    fn hit_test(&self, _point: Point) -> bool {
        // Animation policy has no hit region of its own.
        false
    }

    fn update_needs_layout(&self) -> bool {
        false
    }

    fn child_update_transaction(&self, mut transaction: Transaction) -> Transaction {
        if !transaction.disables_animations {
            if let Some(animation) = self.animation_override {
                transaction.animation = animation;
            }
        }
        transaction
    }

    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = constraints.constrain(Size::ZERO);
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        if let Some(child) = children.first_mut() {
            self.size = child.layout(constraints);
            child.set_position(Point::ZERO);
        } else {
            self.size = constraints.constrain(Size::ZERO);
        }
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn render(&mut self) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{with_animation, with_transaction};
    use crate::state::{State, StateId};
    use crate::view::{ViewExt, ViewKey};
    use crate::views::{Frame, FrameRenderObject, Spacer};
    use core::time::Duration;

    #[test]
    fn scope_only_overrides_changed_values_and_respects_disabled_animations() {
        let inherited = Animation::linear(Duration::from_secs(1));
        let scoped = Animation::linear(Duration::from_secs(2));
        let incoming = Transaction {
            animation: Some(inherited),
            disables_animations: false,
        };
        let mut render = AnimationScopeRenderObject::new(0);
        assert_eq!(render.child_update_transaction(incoming), incoming);
        assert!(!render.has_visual_animation());
        render.update_scope(Some(scoped), &0);
        assert_eq!(render.child_update_transaction(incoming), incoming);
        render.update_scope(Some(scoped), &1);
        assert_eq!(
            render.child_update_transaction(incoming).animation,
            Some(scoped)
        );
        render.update_scope(None, &1);
        assert_eq!(render.child_update_transaction(incoming), incoming);
        render.update_scope(None, &2);
        assert_eq!(render.child_update_transaction(incoming).animation, None);
        render.update_scope(Some(scoped), &3);
        let disabled = Transaction {
            disables_animations: true,
            ..incoming
        };
        assert_eq!(render.child_update_transaction(disabled), disabled);
        render.update_scope(Some(scoped), &3);
        assert_eq!(render.child_update_transaction(incoming), incoming);
    }

    fn child_frame(element: &dyn Element) -> &FrameRenderObject {
        element.children()[0]
            .render_object()
            .unwrap()
            .as_any()
            .downcast_ref::<FrameRenderObject>()
            .unwrap()
    }

    #[test]
    fn render_element_scope_forwards_policy_and_preserves_child_identity() {
        let scoped = Animation::linear(Duration::from_secs(1));
        let make_view = |width, value| {
            AnimationScope::new(Frame::new(Spacer::new(), width, 10.0), Some(scoped), value)
        };
        let mut element = with_animation(scoped, || make_view(10.0, 0).create_element());
        let root_id = element.id();
        let child_id = element.children()[0].id();
        assert!(!child_frame(element.as_ref()).has_visual_animation());
        element.update(&make_view(30.0, 1));
        assert!(child_frame(element.as_ref()).has_visual_animation());
        assert_eq!(child_frame(element.as_ref()).width_value(), Some(10.0));
        element.children_mut()[0]
            .render_object_mut()
            .unwrap()
            .advance_visual_animation(Duration::from_millis(500));
        assert_eq!(child_frame(element.as_ref()).width_value(), Some(20.0));

        // An unchanged trigger inherits the new two-second transaction rather
        // than reusing this scope's prior one-second override.
        with_animation(Animation::linear(Duration::from_secs(2)), || {
            element.update(&make_view(50.0, 1))
        });
        element.children_mut()[0]
            .render_object_mut()
            .unwrap()
            .advance_visual_animation(Duration::from_millis(500));
        assert_eq!(child_frame(element.as_ref()).width_value(), Some(27.5));
        with_transaction(
            Transaction {
                animation: Some(scoped),
                disables_animations: true,
            },
            || element.update(&make_view(80.0, 2)),
        );
        assert_eq!(child_frame(element.as_ref()).width_value(), Some(80.0));
        assert!(!child_frame(element.as_ref()).has_visual_animation());
        assert_eq!(element.id(), root_id);
        assert_eq!(element.children()[0].id(), child_id);
    }

    #[test]
    fn nested_none_scope_suppresses_animation_only_when_its_value_changes() {
        let animation = Some(Animation::linear(Duration::from_secs(1)));
        let make_view = |width, outer_value, inner_value| {
            AnimationScope::new(
                AnimationScope::new(Frame::new(Spacer::new(), width, 10.0), None, inner_value),
                animation,
                outer_value,
            )
        };
        let mut element = make_view(10.0, 0, 0).create_element();
        element.update(&make_view(30.0, 1, 1));
        let frame = child_frame(element.children()[0].as_ref());
        assert_eq!(frame.width_value(), Some(30.0));
        assert!(!frame.has_visual_animation());
        element.update(&make_view(50.0, 2, 1));
        let frame = child_frame(element.children()[0].as_ref());
        assert_eq!(frame.width_value(), Some(30.0));
        assert!(frame.has_visual_animation());
    }

    #[test]
    fn scope_passes_through_layout_and_keeps_trigger_separate_from_keys() {
        let make_view = |value| {
            AnimationScope::new(
                Frame::new(Spacer::new(), 30.0, 40.0).key(99_u64),
                Some(Animation::default()),
                value,
            )
        };
        let view = make_view(0);
        assert!(View::key(&view).is_none());
        let mut element = view.create_element();
        let child_id = element.children()[0].id();
        element.children_mut()[0].set_position(Point::new(11.0, 12.0));
        assert_eq!(
            element.layout(LayoutConstraints::unconstrained()),
            Size::new(30.0, 40.0)
        );
        assert_eq!(element.children()[0].position(), Point::ZERO);
        assert_eq!(element.children()[0].view_key(), Some(&ViewKey::Value(99)));
        element.update(&make_view(1));
        assert_eq!(element.children()[0].id(), child_id);
        assert_eq!(element.children()[0].view_key(), Some(&ViewKey::Value(99)));
        assert!(element.view_key().is_none());
    }

    #[test]
    fn scope_does_not_claim_inner_state_subscriptions() {
        #[derive(Clone)]
        struct ListeningView(State<bool>);

        impl View for ListeningView {
            fn create_element(&self) -> Box<dyn Element> {
                Box::new(RenderElement::new(
                    self.clone(),
                    FrameRenderObject::new(None, None, 0.0, 0.0, f32::INFINITY, f32::INFINITY),
                ))
            }

            fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
                vec![&self.0]
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        let scope = AnimationScope::new(
            ListeningView(State::new(StateId::new(8910), false)),
            Some(Animation::default()),
            0,
        );
        assert_eq!(scope.inner().listenables().len(), 1);
        assert!(scope.listenables().is_empty());
        assert!(
            !scope
                .create_element()
                .render_object()
                .unwrap()
                .owns_view_state()
        );
    }
}
