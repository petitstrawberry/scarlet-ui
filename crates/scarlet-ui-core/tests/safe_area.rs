use scarlet_ui_core::{
    color::Color,
    geometry::{EdgeInsets, Rect, Size},
    pipeline::RenderingPipeline,
    platform::WindowDecoration,
    renderer::PresentedFrame,
    view::{View, ViewExt},
    views::{Rectangle, Window},
};

fn pipeline() -> RenderingPipeline {
    let view = Window::new(
        "Safe area",
        Rectangle::new()
            .fill(Color::rgb(0., 1., 0.))
            .frame(f32::INFINITY, f32::INFINITY),
    )
    .size(Size::new(400., 800.))
    .decoration(WindowDecoration::NONE)
    .shadow(false)
    .corner_radius(0.)
    .background_color(Color::rgb(1., 0., 0.));
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(view.create_element());
    pipeline.layout_initial();
    pipeline
}

fn content_bounds(pipeline: &RenderingPipeline) -> Rect {
    pipeline.element_tree().root().unwrap().children()[0].bounds()
}

#[test]
fn content_moves_with_platform_insets_but_background_and_surface_stay_full_size() {
    let mut pipeline = pipeline();
    let insets = EdgeInsets::new(13., 47., 17., 29.);
    assert!(pipeline.set_safe_area_insets(insets));
    let PresentedFrame::Cpu { buffer, .. } = pipeline.render_for_present().unwrap() else {
        panic!("expected CPU frame")
    };
    assert_eq!(
        (buffer.logical_width(), buffer.logical_height()),
        (400, 800)
    );
    assert_eq!(
        buffer.get_pixel(0, 0),
        Some(Color::rgb(1., 0., 0.).to_bgra())
    );
    assert_eq!(
        buffer.get_pixel(20, 60),
        Some(Color::rgb(0., 1., 0.).to_bgra())
    );
    assert_eq!(
        content_bounds(&pipeline),
        Rect::from_xywh(13., 47., 370., 724.)
    );
    assert!(
        !pipeline.set_safe_area_insets(insets),
        "unchanged OS values must not invalidate layout"
    );

    pipeline.resize(Size::new(800., 400.));
    pipeline.set_safe_area_insets(EdgeInsets::new(49., 0., 51., 19.));
    pipeline.render_for_present().unwrap();
    assert_eq!(
        content_bounds(&pipeline),
        Rect::from_xywh(49., 0., 700., 381.)
    );
    pipeline.set_safe_area_insets(EdgeInsets::ZERO);
    pipeline.render_for_present().unwrap();
    assert_eq!(
        content_bounds(&pipeline),
        Rect::from_xywh(0., 0., 800., 400.)
    );
}

#[test]
fn safe_area_is_per_window_and_survives_root_replacement() {
    let mut first = pipeline();
    let mut second = pipeline();
    first.set_safe_area_insets(EdgeInsets::only_top(37.));
    first.render_for_present().unwrap();
    second.render_for_present().unwrap();
    assert_eq!(content_bounds(&first).origin.y, 37.);
    assert_eq!(content_bounds(&second).origin.y, 0.);

    let view = Window::new(
        "Replacement",
        Rectangle::new().frame(f32::INFINITY, f32::INFINITY),
    )
    .size(Size::new(400., 800.))
    .decoration(WindowDecoration::NONE)
    .shadow(false);
    first.set_root(view.create_element());
    first.layout_initial();
    assert_eq!(content_bounds(&first).origin.y, 37.);
    assert_eq!(first.window_size(), Size::new(400., 800.));
}

#[test]
fn invalid_platform_edges_are_sanitized() {
    let mut pipeline = pipeline();
    pipeline.set_safe_area_insets(EdgeInsets::new(f32::NAN, -10., f32::INFINITY, 23.));
    pipeline.render_for_present().unwrap();
    assert_eq!(pipeline.safe_area_insets(), EdgeInsets::only_bottom(23.));
    assert_eq!(
        content_bounds(&pipeline),
        Rect::from_xywh(0., 0., 400., 777.)
    );
}

fn edge_to_edge(content: impl View + Clone) -> RenderingPipeline {
    let view = Window::new("Edge to edge", content)
        .size(Size::new(400., 800.))
        .decoration(WindowDecoration::NONE)
        .shadow(false)
        .corner_radius(0.)
        .content_safe_area(scarlet_ui_core::SafeAreaEdges::NONE)
        .background_color(Color::rgb(1., 0., 0.));
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(view.create_element());
    pipeline.set_safe_area_insets(EdgeInsets::new(13., 47., 17., 29.));
    pipeline.layout_initial();
    pipeline
}

fn wheel(pipeline: &mut RenderingPipeline, delta_y: i32) {
    use scarlet_ui_core::{Event, MouseEvent, ScrollSource, WheelPhase};
    pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 30, y: 120 }));
    assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
        delta_x: 0,
        delta_y: -delta_y,
        x: 30,
        y: 120,
        phase: WheelPhase::Moved,
        source: ScrollSource::Trackpad,
    })));
}

#[test]
fn scrollport_is_full_surface_while_first_and_last_content_avoid_system_edges() {
    use scarlet_ui_core::ScrollView;
    let mut pipeline = edge_to_edge(
        ScrollView::new(
            Rectangle::new()
                .fill(Color::rgb(0., 1., 0.))
                .frame(f32::INFINITY, f32::INFINITY),
        )
        .vertical()
        .content_size(0., 1200.)
        .wheel_sensitivity(1.),
    );
    let PresentedFrame::Cpu { buffer, .. } = pipeline.render_for_present().unwrap() else {
        panic!()
    };
    assert_eq!(
        buffer.get_pixel(20, 46),
        Some(Color::rgb(1., 0., 0.).to_bgra())
    );
    assert_eq!(
        buffer.get_pixel(20, 799),
        Some(Color::rgb(0., 1., 0.).to_bgra()),
        "content paints behind bottom safe area"
    );
    assert_eq!(
        content_bounds(&pipeline),
        Rect::from_xywh(0., 0., 400., 800.)
    );
    let scroll = &pipeline.element_tree().root().unwrap().children()[0];
    assert_eq!(
        scroll.children()[0].position(),
        scarlet_ui_core::Point::new(13., 47.)
    );
    wheel(&mut pipeline, 100);
    let PresentedFrame::Cpu { buffer, .. } = pipeline.render_for_present().unwrap() else {
        panic!()
    };
    assert_eq!(
        buffer.get_pixel(20, 0),
        Some(Color::rgb(0., 1., 0.).to_bgra()),
        "scrolling paints through top safe area"
    );
    assert_eq!(
        pipeline.element_tree().root().unwrap().children()[0].children()[0]
            .position()
            .y,
        -53.
    );
    wheel(&mut pipeline, 10000);
    pipeline.render_for_present().unwrap();
    let scroll = &pipeline.element_tree().root().unwrap().children()[0];
    assert_eq!(
        scroll.children()[0].bounds().bottom(),
        771.,
        "last item can stop above home indicator"
    );
    pipeline.set_safe_area_insets(EdgeInsets::new(49., 0., 51., 19.));
    pipeline.resize(Size::new(800., 400.));
    pipeline.render_for_present().unwrap();
    let scroll = &pipeline.element_tree().root().unwrap().children()[0];
    assert_eq!(scroll.bounds(), Rect::from_xywh(0., 0., 800., 400.));
    assert_eq!(scroll.children()[0].position().x, 49.);
    assert_eq!(
        scroll.children()[0].bounds().bottom(),
        724.,
        "rotation preserves the current scroll offset"
    );
    wheel(&mut pipeline, 10000);
    pipeline.render_for_present().unwrap();
    let scroll = &pipeline.element_tree().root().unwrap().children()[0];
    assert_eq!(
        scroll.children()[0].bounds().bottom(),
        381.,
        "last item remains reachable above the rotated safe edge"
    );
}

#[test]
fn nested_safe_area_scopes_consume_each_edge_once_and_geometry_observes_remaining_edges() {
    use scarlet_ui_core::{Point, SafeAreaEdges};
    use std::{cell::RefCell, rc::Rc};
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let content = Rectangle::new()
        .frame(f32::INFINITY, f32::INFINITY)
        .safe_area_padding(SafeAreaEdges::ALL)
        .on_geometry_change(|g| g.safe_area_insets(), move |v| sink.borrow_mut().push(v))
        .safe_area_padding(SafeAreaEdges::TOP | SafeAreaEdges::LEFT);
    let mut pipeline = edge_to_edge(content);
    pipeline.render_for_present().unwrap();
    assert_eq!(&*seen.borrow(), &[EdgeInsets::new(0., 0., 17., 29.)]);
    let outer = &pipeline.element_tree().root().unwrap().children()[0];
    assert_eq!(outer.children()[0].position(), Point::new(13., 47.));
    let inner = &outer.children()[0].children()[0];
    assert_eq!(inner.children()[0].position(), Point::ZERO);
    assert_eq!(inner.children()[0].bounds().size, Size::new(370., 724.));
    pipeline.set_safe_area_insets(EdgeInsets::only_bottom(20.));
    pipeline.render_for_present().unwrap();
    assert_eq!(seen.borrow().last(), Some(&EdgeInsets::only_bottom(20.)));
    pipeline.render_for_present().unwrap();
    assert_eq!(
        seen.borrow().len(),
        2,
        "no duplicate geometry notification on an unchanged frame"
    );
}

#[test]
fn safe_area_padding_and_nested_scroll_do_not_double_inset() {
    use scarlet_ui_core::{SafeAreaEdges, ScrollView};
    let mut pipeline = edge_to_edge(
        ScrollView::new(Rectangle::new().frame(f32::INFINITY, f32::INFINITY))
            .vertical()
            .safe_area_padding(SafeAreaEdges::ALL)
            .safe_area_padding(SafeAreaEdges::ALL),
    );
    pipeline.render_for_present().unwrap();
    let outer = &pipeline.element_tree().root().unwrap().children()[0];
    let inner = &outer.children()[0];
    assert_eq!(inner.position(), scarlet_ui_core::Point::new(13., 47.));
    let scroll = &inner.children()[0];
    assert_eq!(scroll.position(), scarlet_ui_core::Point::ZERO);
    assert_eq!(
        scroll.children()[0].position(),
        scarlet_ui_core::Point::ZERO
    );
    assert_eq!(scroll.bounds().size, Size::new(370., 724.));
}

#[test]
fn safe_area_context_is_retained_through_components_and_child_reconciliation() {
    use scarlet_ui_core::{SafeAreaEdges, State, state::generate_state_id, views::Either};
    #[derive(Clone)]
    struct Rebuild {
        changed: State<bool>,
    }
    impl View for Rebuild {
        fn create_element(&self) -> Box<dyn scarlet_ui_core::Element> {
            Box::new(
                scarlet_ui_core::element::ComponentElement::new_with_builder(
                    self.clone(),
                    |view| {
                        if view.changed.get() {
                            Box::new(Either::<_, Rectangle>::A(
                                Rectangle::new().safe_area_padding(SafeAreaEdges::ALL),
                            ))
                        } else {
                            Box::new(Either::<scarlet_ui_core::SafeArea<Rectangle>, _>::B(
                                Rectangle::new(),
                            ))
                        }
                    },
                ),
            )
        }
        fn listenables(&self) -> Vec<&dyn scarlet_ui_core::Listenable> {
            vec![&self.changed]
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    let changed = State::new(generate_state_id(), false);
    let mut pipeline = edge_to_edge(Rebuild {
        changed: changed.clone(),
    });
    pipeline.render_for_present().unwrap();
    changed.set(true);
    pipeline.render_for_present().unwrap();
    fn find_padding(
        element: &dyn scarlet_ui_core::Element,
    ) -> Option<&dyn scarlet_ui_core::Element> {
        if element.render_object().is_some_and(|r| {
            r.as_any()
                .is::<scarlet_ui_core::views::modifiers::SafeAreaRenderObject>()
        }) {
            return Some(element);
        }
        element
            .children()
            .iter()
            .find_map(|c| find_padding(c.as_ref()))
    }
    let padding = find_padding(pipeline.element_tree().root().unwrap()).unwrap();
    assert_eq!(
        padding.children()[0].position(),
        scarlet_ui_core::Point::new(13., 47.)
    );
}

#[test]
fn artwork_can_opt_out_of_scroll_content_safe_area() {
    use scarlet_ui_core::{Point, SafeAreaEdges, ScrollView};
    let mut pipeline = edge_to_edge(
        ScrollView::new(Rectangle::new().frame(f32::INFINITY, f32::INFINITY))
            .vertical()
            .content_size(0., 1200.)
            .content_safe_area(SafeAreaEdges::NONE),
    );
    pipeline.render_for_present().unwrap();
    let scroll = &pipeline.element_tree().root().unwrap().children()[0];
    assert_eq!(scroll.children()[0].position(), Point::ZERO);
    assert_eq!(scroll.children()[0].bounds().size, Size::new(400., 1200.));
}

#[test]
fn safe_area_scrolling_does_not_relayout_content() {
    use scarlet_ui_core::element::RenderElement;
    use scarlet_ui_core::{Element, ElementRenderObject, LayoutConstraints, Point, ScrollView};
    use std::{any::Any, cell::Cell, rc::Rc};
    #[derive(Clone)]
    struct CountLayout(Rc<Cell<usize>>);
    struct Counter {
        count: Rc<Cell<usize>>,
        size: Size,
    }
    impl View for CountLayout {
        fn create_element(&self) -> Box<dyn Element> {
            Box::new(RenderElement::new(
                self.clone(),
                Counter {
                    count: self.0.clone(),
                    size: Size::ZERO,
                },
            ))
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }
    impl ElementRenderObject for Counter {
        fn layout(&mut self, constraints: LayoutConstraints) -> Size {
            self.count.set(self.count.get() + 1);
            self.size = constraints.constrain(Size::new(10., 10.));
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
    let calls = Rc::new(Cell::new(0));
    let mut pipeline = edge_to_edge(
        ScrollView::new(CountLayout(calls.clone()))
            .vertical()
            .content_size(0., 1200.)
            .wheel_sensitivity(1.),
    );
    pipeline.render_for_present().unwrap();
    let before = calls.get();
    for _ in 0..20 {
        wheel(&mut pipeline, 5);
        pipeline.render_for_present().unwrap();
    }
    assert_eq!(
        calls.get(),
        before,
        "safe-area layout must not add layout work to scrolling"
    );
    assert_eq!(
        pipeline.element_tree().root().unwrap().children()[0].children()[0].position(),
        Point::new(13., -53.)
    );
}
