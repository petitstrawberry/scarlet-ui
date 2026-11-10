//! Test tuple-based containers

#[cfg(test)]
mod tests {
    use crate::element::LayoutConstraints;
    use crate::geometry::Size;
    use crate::view::{View, ViewExt};
    use crate::views::containers::{HStack, VStack, ZStack};
    use crate::views::{CanvasView, Text};
    use alloc::rc::Rc;

    // Count work instead of asserting wall-clock timing. The old size-based
    // greedy heuristic repeatedly laid out overflowing intrinsic subtrees.
    struct CountedLeaf {
        calls: Rc<core::cell::Cell<usize>>,
        size: Size,
    }

    impl crate::element::ElementRenderObject for CountedLeaf {
        fn layout(&mut self, constraints: LayoutConstraints) -> Size {
            let calls = self.calls.get() + 1;
            assert!(calls <= 512, "deep intrinsic layout repeated its subtrees");
            self.calls.set(calls);
            self.size = constraints.constrain(Size::new(16.0, 16.0));
            self.size
        }
        fn size(&self) -> Size {
            self.size
        }
        fn render(&mut self) {}
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
            self
        }
    }

    #[test]
    fn deep_intrinsic_stacks_measure_each_leaf_once() {
        use crate::element::{Element, RenderElement};
        use crate::geometry::Alignment;
        use alloc::{boxed::Box, vec};

        for vertical in [true, false] {
            let calls = Rc::new(core::cell::Cell::new(0));
            let leaf = || {
                Box::new(RenderElement::new(
                    Text::new("leaf"),
                    CountedLeaf {
                        calls: calls.clone(),
                        size: Size::ZERO,
                    },
                )) as Box<dyn Element>
            };
            let mut tree = leaf();
            for _ in 0..64 {
                let children = vec![leaf(), tree];
                tree = if vertical {
                    Box::new(RenderElement::with_children(
                        Text::new("stack"),
                        super::super::vstack::VStackRenderObject::new(0.0, Alignment::Leading),
                        children,
                    ))
                } else {
                    Box::new(RenderElement::with_children(
                        Text::new("stack"),
                        super::super::hstack::HStackRenderObject::new(0.0, Alignment::Leading),
                        children,
                    ))
                };
            }
            assert_eq!(
                tree.layout(LayoutConstraints::loose(800.0, 800.0)),
                if vertical {
                    Size::new(16.0, 800.0)
                } else {
                    Size::new(800.0, 16.0)
                }
            );
            assert_eq!(calls.get(), 65);
        }
    }

    #[test]
    fn scroll_view_and_spacer_keep_remaining_space_between_fixed_siblings() {
        use crate::views::{Rectangle, ScrollView, Spacer};
        let mut vertical = VStack::new((
            Text::new("header").frame(20.0, 20.0),
            ScrollView::new(Rectangle::new().frame(80.0, 1000.0))
                .frame_width(80.0)
                .padding(0.0),
            Text::new("footer").frame(20.0, 30.0),
        ))
        .spacing(5.0)
        .create_element();
        vertical.layout(LayoutConstraints::tight(100.0, 200.0));
        assert_eq!(vertical.children()[1].bounds().size.height, 140.0);
        assert_eq!(vertical.children()[2].position().y, 170.0);

        let mut horizontal = HStack::new((
            Text::new("left").frame(20.0, 20.0),
            ScrollView::new(Rectangle::new().frame(1000.0, 80.0))
                .horizontal()
                .frame_height(80.0)
                .padding(0.0),
            Text::new("right").frame(30.0, 20.0),
        ))
        .spacing(5.0)
        .create_element();
        horizontal.layout(LayoutConstraints::tight(200.0, 100.0));
        assert_eq!(horizontal.children()[1].bounds().size.width, 140.0);
        assert_eq!(horizontal.children()[2].position().x, 170.0);

        let mut spacer = VStack::new((
            Text::new("header").frame(20.0, 20.0),
            Spacer::new(),
            Text::new("footer").frame(20.0, 30.0),
        ))
        .spacing(5.0)
        .create_element();
        spacer.layout(LayoutConstraints::tight(100.0, 200.0));
        let allocated = spacer.children()[1].last_layout_constraints().unwrap();
        assert_eq!(allocated.min_height, 140.0);
        assert_eq!(allocated.max_height, 140.0);
        assert_eq!(spacer.children()[2].position().y, 170.0);
    }

    #[test]
    fn test_vstack_tuple() {
        // Test creating a VStack with a 2-tuple
        let stack = VStack::new((Text::new("Hello"), Text::new("World"))).spacing(10.0);

        let mut element = stack.create_element();
        let size = element.layout(LayoutConstraints::loose(200.0, 200.0));
        assert!(!size.is_zero());
    }

    #[test]
    fn test_vstack_empty() {
        // Test creating an empty VStack
        let stack = VStack::new(());
        let mut element = stack.create_element();
        let size = element.layout(LayoutConstraints::loose(200.0, 200.0));
        assert_eq!(size, Size::ZERO);
    }

    #[test]
    fn test_hstack_tuple() {
        // Test creating an HStack with a 3-tuple
        let stack =
            HStack::new((Text::new("Left"), Text::new("Middle"), Text::new("Right"))).spacing(5.0);

        let mut element = stack.create_element();
        let size = element.layout(LayoutConstraints::loose(200.0, 200.0));
        assert!(!size.is_zero());
    }

    #[test]
    fn test_zstack_tuple() {
        // Test creating a ZStack with a 2-tuple
        let stack = ZStack::new((Text::new("Background"), Text::new("Foreground")));

        let mut element = stack.create_element();
        let size = element.layout(LayoutConstraints::loose(200.0, 200.0));
        assert!(!size.is_zero());
    }

    #[test]
    fn nested_same_type_update_preserves_elements_and_render_object() {
        let first =
            VStack::new((CanvasView::new(80.0, 40.0, Rc::new(|_, _, _| {})).frame(80.0, 40.0),));
        let mut element = first.create_element();
        element.layout(LayoutConstraints::loose(200.0, 200.0));

        let root_id = element.id();
        let frame_id = element.children()[0].id();
        let canvas_id = element.children()[0].children()[0].id();
        let render_object_address = element.children()[0].children()[0]
            .render_object()
            .expect("Canvas should own a render object")
            as *const dyn crate::element::ElementRenderObject
            as *const ();

        let next =
            VStack::new((CanvasView::new(80.0, 40.0, Rc::new(|_, _, _| {})).frame(80.0, 40.0),));
        assert!(matches!(
            element.update(&next),
            crate::element::UpdateResult::Updated
        ));

        assert_eq!(element.id(), root_id);
        assert_eq!(element.children()[0].id(), frame_id);
        assert_eq!(element.children()[0].children()[0].id(), canvas_id);
        let next_render_object_address = element.children()[0].children()[0]
            .render_object()
            .expect("Canvas should retain its render object")
            as *const dyn crate::element::ElementRenderObject
            as *const ();
        assert_eq!(next_render_object_address, render_object_address);
    }
}
