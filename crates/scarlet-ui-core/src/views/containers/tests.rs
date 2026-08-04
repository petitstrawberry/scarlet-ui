//! Test tuple-based containers

#[cfg(test)]
mod tests {
    use crate::element::LayoutConstraints;
    use crate::geometry::Size;
    use crate::view::{View, ViewExt};
    use crate::views::containers::{HStack, VStack, ZStack};
    use crate::views::{CanvasView, Text};
    use alloc::rc::Rc;

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
