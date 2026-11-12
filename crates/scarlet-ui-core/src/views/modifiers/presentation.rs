//! Animated presentation properties that preserve the child's layout size.

use crate::animation::{Animated, Animation, current_transaction};
use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, ScrollOffsetUpdate,
    UpdateResult,
};
use crate::geometry::{Point, Size};
use crate::view::View;
use alloc::{boxed::Box, vec};
use core::{any::Any, time::Duration};

fn finite_offset(dx: f32, dy: f32) -> Point {
    Point::new(
        if dx.is_finite() { dx } else { 0.0 },
        if dy.is_finite() { dy } else { 0.0 },
    )
}

fn bounded_opacity(opacity: f32) -> f32 {
    if opacity.is_nan() {
        1.0
    } else {
        opacity.clamp(0.0, 1.0)
    }
}

fn layout_child(
    constraints: LayoutConstraints,
    children: &mut [Box<dyn Element>],
    position: Point,
) -> Size {
    if let Some(child) = children.first_mut() {
        let size = child.layout(constraints);
        child.set_position(position);
        size
    } else {
        constraints.constrain(Size::ZERO)
    }
}

/// Translate a child without changing the space reserved for it by layout.
#[derive(Clone)]
pub struct OffsetView<V: View> {
    inner: V,
    offset: Point,
}

impl<V: View> OffsetView<V> {
    pub fn new(inner: V, dx: f32, dy: f32) -> Self {
        Self {
            inner,
            offset: finite_offset(dx, dy),
        }
    }

    pub fn inner(&self) -> &V {
        &self.inner
    }

    pub fn offset_value(&self) -> Point {
        self.offset
    }
}

impl<V: View + Clone> View for OffsetView<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| OffsetRenderObject::new(view.offset.x, view.offset.y),
            |render, view| {
                render.update_offset(view.offset, current_transaction().effective_animation())
            },
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Presentation translation, stored in child positions for paint and hit tests.
pub struct OffsetRenderObject {
    offset: Animated<Point>,
    size: Size,
}

impl OffsetRenderObject {
    pub fn new(dx: f32, dy: f32) -> Self {
        Self {
            offset: Animated::new(finite_offset(dx, dy)),
            size: Size::ZERO,
        }
    }

    /// Current presentation offset, including any animation in progress.
    pub fn offset(&self) -> Point {
        self.offset.value()
    }

    fn update_offset(&mut self, offset: Point, animation: Option<Animation>) -> UpdateResult {
        if self
            .offset
            .retarget(finite_offset(offset.x, offset.y), animation)
        {
            UpdateResult::Updated
        } else {
            UpdateResult::NoChange
        }
    }
}

impl ElementRenderObject for OffsetRenderObject {
    // Only translated descendants own a hit region. The unchanged layout
    // footprint must not intercept controls behind the child's old position.
    fn hit_test(&self, _point: Point) -> bool {
        false
    }

    fn presentation_affects_subtree_bounds(&self) -> bool {
        true
    }
    fn update_needs_layout(&self) -> bool {
        false
    }

    fn update_needs_subtree_paint(&self) -> bool {
        false
    }

    fn has_visual_animation(&self) -> bool {
        self.offset.is_active()
    }

    fn advance_visual_animation(&mut self, delta: Duration) -> bool {
        self.offset.advance(delta)
    }

    fn finish_visual_animation(&mut self) -> bool {
        self.offset.finish()
    }

    fn apply_scroll_offset(&mut self, children: &mut [Box<dyn Element>]) -> ScrollOffsetUpdate {
        let offset = self.offset();
        let mut changed = false;
        for child in children {
            if child.position() != offset {
                child.set_position(offset);
                changed = true;
            }
        }
        if changed {
            ScrollOffsetUpdate::NeedsComposite
        } else {
            ScrollOffsetUpdate::None
        }
    }

    fn apply_view_scroll_offset(
        &mut self,
        children: &mut [Box<dyn Element>],
    ) -> ScrollOffsetUpdate {
        self.apply_scroll_offset(children)
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
        self.size = layout_child(constraints, children, self.offset());
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

/// Modulate each drawing primitive's alpha in the child's subtree.
///
/// Nested modifiers multiply their opacity. This preserves layout and uses
/// primitive-alpha modulation without creating an isolated offscreen group.
#[derive(Clone)]
pub struct Opacity<V: View> {
    inner: V,
    opacity: f32,
}

impl<V: View> Opacity<V> {
    pub fn new(inner: V, opacity: f32) -> Self {
        Self {
            inner,
            opacity: bounded_opacity(opacity),
        }
    }

    pub fn inner(&self) -> &V {
        &self.inner
    }

    pub fn opacity_value(&self) -> f32 {
        self.opacity
    }
}

impl<V: View + Clone> View for Opacity<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| OpacityRenderObject::new(view.opacity),
            |render, view| {
                render.update_opacity(view.opacity, current_transaction().effective_animation())
            },
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Supplies subtree opacity to the pipeline; emits no paint commands itself.
pub struct OpacityRenderObject {
    opacity: Animated<f32>,
    size: Size,
}

impl OpacityRenderObject {
    pub fn new(opacity: f32) -> Self {
        Self {
            opacity: Animated::new(bounded_opacity(opacity)),
            size: Size::ZERO,
        }
    }

    /// Current presentation opacity, including any animation in progress.
    pub fn opacity(&self) -> f32 {
        bounded_opacity(self.opacity.value())
    }

    fn update_opacity(&mut self, opacity: f32, animation: Option<Animation>) -> UpdateResult {
        let target = bounded_opacity(opacity);
        if *self.opacity.target() == target {
            return UpdateResult::NoChange;
        }
        let displayed = self.opacity();
        if self.opacity.value() != displayed {
            // A spring can overshoot the alpha range. A new target starts at
            // the visible alpha, while duplicate targets keep their timeline.
            self.opacity = Animated::new(displayed);
        }
        self.opacity.retarget(target, animation);
        UpdateResult::Updated
    }
}

impl ElementRenderObject for OpacityRenderObject {
    // Descendants keep their hit behavior, including offsets outside this box.
    fn hit_test(&self, _point: Point) -> bool {
        false
    }

    fn presentation_affects_subtree_bounds(&self) -> bool {
        true
    }
    fn has_opacity_scope(&self) -> bool {
        true
    }
    fn update_needs_composite(&self) -> bool {
        true
    }

    fn update_needs_layout(&self) -> bool {
        false
    }

    fn update_needs_subtree_paint(&self) -> bool {
        false
    }

    fn subtree_opacity(&self) -> f32 {
        self.opacity()
    }

    fn has_visual_animation(&self) -> bool {
        self.opacity.is_active()
    }

    fn advance_visual_animation(&mut self, delta: Duration) -> bool {
        self.opacity.advance(delta)
    }

    fn finish_visual_animation(&mut self) -> bool {
        self.opacity.finish()
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
        self.size = layout_child(constraints, children, Point::ZERO);
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
    use crate::animation::{with_animation, without_animation};
    use crate::element::ElementTree;
    use crate::event::EventDispatcher;
    use crate::renderer::PaintContext;
    use crate::testing::layout_probe::LayoutProbe;
    use crate::view::ViewExt;
    use crate::views::{Frame, Spacer};
    use alloc::rc::Rc;
    use core::cell::Cell;

    #[test]
    fn opacity_retarget_starts_at_the_clamped_spring_presentation() {
        for (start, target, displayed) in [(0.0, 1.0, 1.0), (1.0, 0.0, 0.0)] {
            let mut render = OpacityRenderObject::new(start);
            render.update_opacity(target, Some(Animation::spring(Duration::from_secs(1), 0.1)));
            render.advance_visual_animation(Duration::from_millis(300));
            assert_eq!(render.opacity(), displayed);
            assert_ne!(render.opacity.value(), displayed);
            assert!(matches!(
                render.update_opacity(target, None),
                UpdateResult::NoChange
            ));
            assert!(render.has_visual_animation());

            render.update_opacity(0.5, Some(Animation::linear(Duration::from_secs(1))));
            assert_eq!(render.opacity(), displayed);
            render.advance_visual_animation(Duration::from_millis(500));
            assert_eq!(render.opacity(), (displayed + 0.5) / 2.0);
        }
    }

    #[test]
    fn presentation_modifiers_can_be_chained_without_getter_collisions() {
        let view = Spacer::new().opacity(0.5).opacity(0.5);
        assert_eq!(view.opacity_value(), 0.5);
        assert_eq!(view.inner().opacity_value(), 0.5);
        let element = view.create_element();
        assert_eq!(element.render_object().unwrap().subtree_opacity(), 0.5);
        assert_eq!(
            element.children()[0]
                .render_object()
                .unwrap()
                .subtree_opacity(),
            0.5
        );

        let view = Spacer::new().offset(1.0, 2.0).offset(3.0, 4.0);
        assert_eq!(view.offset_value(), Point::new(3.0, 4.0));
        assert_eq!(view.inner().offset_value(), Point::new(1.0, 2.0));
        let mut element = view.create_element();
        element.layout(LayoutConstraints::tight(10.0, 20.0));
        assert_eq!(element.children()[0].position(), Point::new(3.0, 4.0));
        assert_eq!(
            element.children()[0].children()[0].position(),
            Point::new(1.0, 2.0)
        );
    }

    #[test]
    fn offset_ticks_move_children_without_repeating_layout() {
        let layouts = Rc::new(Cell::new(0));
        let mut children = vec![LayoutProbe(layouts.clone()).create_element()];
        let mut render = OffsetRenderObject::new(0.0, 0.0);
        assert_eq!(
            render.layout_with_children(LayoutConstraints::loose(100.0, 100.0), &mut children),
            Size::new(10.0, 10.0)
        );
        assert_eq!(layouts.get(), 1);
        assert!(!render.update_needs_layout());
        assert!(!render.update_needs_subtree_paint());
        let target = Point::new(80.0, -40.0);
        assert!(matches!(
            render.update_offset(target, Some(Animation::linear(Duration::from_secs(1)))),
            UpdateResult::Updated
        ));
        assert_eq!(render.offset(), Point::ZERO);
        assert_eq!(
            render.apply_view_scroll_offset(&mut children),
            ScrollOffsetUpdate::None
        );
        assert!(render.advance_visual_animation(Duration::from_millis(500)));
        assert_eq!(
            render.apply_view_scroll_offset(&mut children),
            ScrollOffsetUpdate::NeedsComposite
        );
        assert_eq!(children[0].position(), Point::new(40.0, -20.0));
        assert!(matches!(
            render.update_offset(target, None),
            UpdateResult::NoChange
        ));
        assert!(render.has_visual_animation());
        assert!(render.finish_visual_animation());
        assert_eq!(
            render.apply_scroll_offset(&mut children),
            ScrollOffsetUpdate::NeedsComposite
        );
        assert_eq!(children[0].position(), target);
        assert_eq!(
            render.apply_view_scroll_offset(&mut children),
            ScrollOffsetUpdate::None
        );
        assert!(!render.finish_visual_animation());
        assert_eq!(render.size(), Size::new(10.0, 10.0));
        assert_eq!(layouts.get(), 1);
    }

    #[test]
    fn offset_view_updates_keep_hit_tests_in_translated_child_coordinates() {
        let child = Frame::new(Spacer::new(), 20.0, 10.0);
        let mut tree = ElementTree::new();
        tree.set_root(OffsetView::new(child.clone(), -30.0, 40.0).create_element());
        assert_eq!(
            tree.layout(LayoutConstraints::loose(100.0, 100.0)),
            Size::new(20.0, 10.0)
        );
        let child_id = tree.root().unwrap().children()[0].id();
        let dispatcher = EventDispatcher::new();
        assert_eq!(
            dispatcher
                .hit_test(&tree, Point::new(-29.0, 41.0))
                .unwrap()
                .id(),
            child_id
        );

        // The compatible view update uses apply_view_scroll_offset directly.
        tree.root_mut()
            .unwrap()
            .update(&OffsetView::new(child, 80.0, -20.0));
        let root = tree.root().unwrap();
        assert_eq!(root.children()[0].id(), child_id);
        assert_eq!(root.children()[0].position(), Point::new(80.0, -20.0));
        assert_eq!(root.render_object().unwrap().size(), Size::new(20.0, 10.0));
        assert_eq!(
            dispatcher
                .hit_test(&tree, Point::new(81.0, -19.0))
                .unwrap()
                .id(),
            child_id
        );
        assert!(
            dispatcher
                .hit_test(&tree, Point::new(-29.0, 41.0))
                .is_none()
        );

        // Animation ticks use the same hook and hit-test transform.
        with_animation(Animation::linear(Duration::from_secs(1)), || {
            tree.root_mut().unwrap().update(&OffsetView::new(
                Frame::new(Spacer::new(), 20.0, 10.0),
                40.0,
                20.0,
            ));
        });
        tree.root_mut()
            .unwrap()
            .advance_visual_animation(Duration::from_millis(500));
        assert_eq!(
            tree.root().unwrap().children()[0].position(),
            Point::new(60.0, 0.0)
        );
        assert_eq!(
            dispatcher
                .hit_test(&tree, Point::new(61.0, 1.0))
                .unwrap()
                .id(),
            child_id
        );
        tree.clear_root();
    }

    #[test]
    fn translated_layer_does_not_block_buttons_in_its_original_layout_footprint() {
        use crate::event::{Event, MouseButton, MouseEvent};
        use crate::views::{Button, ZStack};

        let first_clicks = Rc::new(Cell::new(0));
        let second_clicks = Rc::new(Cell::new(0));
        let first = first_clicks.clone();
        let second = second_clicks.clone();
        let view = ZStack::new((
            Button::new("First")
                .on_click(move || first.set(first.get() + 1))
                .frame(80.0, 40.0),
            Button::new("Second")
                .on_click(move || second.set(second.get() + 1))
                .frame(80.0, 40.0)
                .offset(0.0, 80.0)
                .opacity(0.7)
                .animation(Animation::default(), false)
                .key(1u64),
        ))
        .alignment(crate::geometry::Alignment::TopLeading)
        .frame(100.0, 140.0);
        let mut tree = ElementTree::new();
        tree.set_root(view.create_element());
        tree.layout(LayoutConstraints::tight(100.0, 140.0));
        let mut dispatcher = EventDispatcher::new();
        dispatcher.set_root(tree.root().unwrap().id());
        for y in [20, 100] {
            dispatcher.dispatch(
                &mut tree,
                &Event::Mouse(MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x: 30,
                    y,
                    click_count: 1,
                }),
            );
            dispatcher.dispatch(
                &mut tree,
                &Event::Mouse(MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x: 30,
                    y,
                    click_count: 1,
                }),
            );
        }
        assert_eq!(
            first_clicks.get(),
            1,
            "the lower button remains clickable at the vacated position"
        );
        assert_eq!(
            second_clicks.get(),
            1,
            "the translated button receives its own click"
        );
        tree.clear_root();
    }

    #[test]
    fn opacity_animates_subtree_metadata_and_emits_no_commands() {
        let layouts = Rc::new(Cell::new(0));
        let mut children = vec![LayoutProbe(layouts.clone()).create_element()];
        children[0].set_position(Point::new(10.0, 20.0));
        let mut render = OpacityRenderObject::new(0.25);
        assert_eq!(
            render.layout_with_children(LayoutConstraints::loose(100.0, 100.0), &mut children),
            Size::new(10.0, 10.0)
        );
        assert_eq!(children[0].position(), Point::ZERO);
        assert!(!render.has_visual_animation());
        assert!(!render.update_needs_layout());
        assert!(!render.update_needs_subtree_paint());
        let animation = Some(Animation::linear(Duration::from_secs(1)));
        render.update_opacity(0.75, animation);
        assert_eq!(render.subtree_opacity(), 0.25);
        render.advance_visual_animation(Duration::from_millis(500));
        assert_eq!(render.subtree_opacity(), 0.5);
        assert!(matches!(
            render.update_opacity(0.75, None),
            UpdateResult::NoChange
        ));
        render.update_opacity(0.0, animation);
        assert_eq!(render.subtree_opacity(), 0.5);
        render.advance_visual_animation(Duration::from_millis(500));
        assert_eq!(render.subtree_opacity(), 0.25);
        assert!(render.finish_visual_animation());
        assert_eq!(render.subtree_opacity(), 0.0);
        assert!(!render.finish_visual_animation());
        assert_eq!(render.size(), Size::new(10.0, 10.0));
        assert_eq!(layouts.get(), 1);
        let mut context = PaintContext::new();
        render.paint(&mut context, Point::ZERO);
        assert!(context.is_empty());
    }

    #[test]
    fn presentation_view_updaters_honor_effective_animation() {
        let animation = Animation::linear(Duration::from_secs(1));
        let mut offset = with_animation(animation, || {
            OffsetView::new(Spacer::new(), 10.0, 20.0).create_element()
        });
        let mut opacity = with_animation(animation, || {
            Opacity::new(Spacer::new(), 0.25).create_element()
        });
        assert!(!offset.render_object().unwrap().has_visual_animation());
        assert!(!opacity.render_object().unwrap().has_visual_animation());
        with_animation(animation, || {
            offset.update(&OffsetView::new(Spacer::new(), 30.0, 40.0));
            opacity.update(&Opacity::new(Spacer::new(), 0.75));
        });
        assert!(offset.render_object().unwrap().has_visual_animation());
        assert!(opacity.render_object().unwrap().has_visual_animation());
        without_animation(|| {
            offset.update(&OffsetView::new(Spacer::new(), 50.0, 60.0));
            opacity.update(&Opacity::new(Spacer::new(), 0.0));
        });
        assert!(!offset.render_object().unwrap().has_visual_animation());
        assert!(!opacity.render_object().unwrap().has_visual_animation());
        assert_eq!(offset.children()[0].position(), Point::new(50.0, 60.0));
        assert_eq!(opacity.render_object().unwrap().subtree_opacity(), 0.0);
    }

    #[test]
    fn nonfinite_presentation_inputs_are_canonical_and_opacity_is_bounded() {
        assert_eq!(
            OffsetRenderObject::new(f32::NAN, f32::INFINITY).offset(),
            Point::ZERO
        );
        assert_eq!(
            OffsetRenderObject::new(f32::NEG_INFINITY, 4.0).offset(),
            Point::new(0.0, 4.0)
        );
        for (input, expected) in [
            (f32::NAN, 1.0),
            (f32::INFINITY, 1.0),
            (f32::NEG_INFINITY, 0.0),
            (-0.5, 0.0),
            (1.5, 1.0),
        ] {
            assert_eq!(OpacityRenderObject::new(input).subtree_opacity(), expected);
        }
        let mut render = OpacityRenderObject::new(0.0);
        render.update_opacity(1.0, Some(Animation::spring(Duration::from_secs(1), 0.5)));
        render.advance_visual_animation(Duration::from_millis(250));
        assert_eq!(render.subtree_opacity(), 1.0);
        assert!(render.has_visual_animation());
    }
}
