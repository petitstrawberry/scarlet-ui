use scarlet_ui_core::{
    Event, MouseEvent, NavigationLink, NavigationView, ScrollSource, WheelPhase,
    pipeline::RenderingPipeline, platform::WindowDecoration, prelude::*, renderer::PresentedFrame,
};

fn pipeline(content: impl View + Clone) -> RenderingPipeline {
    let window = Window::new("Layered content", content)
        .size(Size::new(400., 300.))
        .decoration(WindowDecoration::NONE)
        .content_safe_area(SafeAreaEdges::NONE)
        .background_color(Color::WHITE)
        .shadow(false)
        .corner_radius(0.);
    let mut p = RenderingPipeline::new();
    p.set_root(window.create_element());
    p.layout_initial();
    p
}

fn shell(p: &RenderingPipeline) -> &dyn Element {
    p.element_tree().root().unwrap().children()[0].as_ref()
}

fn wheel(p: &mut RenderingPipeline, amount: i32) {
    p.handle_event(&Event::Mouse(MouseEvent::Moved { x: 150, y: 140 }));
    assert!(p.handle_event(&Event::Mouse(MouseEvent::Wheel {
        delta_x: 0,
        delta_y: -amount,
        x: 150,
        y: 140,
        phase: WheelPhase::Moved,
        source: ScrollSource::Trackpad,
    })));
}

#[test]
fn scroll_viewport_underlaps_both_slots_but_first_and_last_items_remain_safe() {
    let mut p = pipeline(
        ContentShell::scrolling(
            ScrollView::new(Rectangle::new().fill(Color::rgb(0., 1., 0.)))
                .vertical()
                .content_size(0., 1000.)
                .wheel_sensitivity(1.),
        )
        .sidebar(Rectangle::new(), 80.)
        .header(Rectangle::new(), 60.),
    );
    p.set_safe_area_insets(EdgeInsets::new(10., 20., 12., 14.));
    p.render_for_present().unwrap();
    let children = shell(&p).children();
    assert_eq!(children[0].bounds(), Rect::from_xywh(74., 64., 326., 236.));
    assert_eq!(children[0].children()[0].position(), Point::new(16., 16.));
    assert_eq!(children[0].children()[0].bounds().size.width, 298.);
    assert_eq!(children[1].bounds(), Rect::from_xywh(90., 20., 298., 60.));
    assert_eq!(children[2].bounds(), Rect::from_xywh(10., 20., 80., 266.));
    wheel(&mut p, 10000);
    p.render_for_present().unwrap();
    let viewport = &shell(&p).children()[0];
    assert_eq!(
        viewport.position().y + viewport.children()[0].bounds().bottom(),
        286.
    );
    // Resizing and a new OS reservation do not stack the old chrome insets.
    p.resize(Size::new(600., 400.));
    p.set_safe_area_insets(EdgeInsets::ZERO);
    p.render_for_present().unwrap();
    let children = shell(&p).children();
    assert_eq!(children[0].bounds(), Rect::from_xywh(64., 44., 536., 356.));
    assert_eq!(children[0].children()[0].position().x, 16.);
}

#[test]
fn optional_slots_reserve_only_the_space_they_occupy_and_fixed_content_is_safe() {
    for (header, sidebar) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut view = ContentShell::new(Rectangle::new());
        if header {
            view = view.header(Rectangle::new(), 60.);
        }
        if sidebar {
            view = view.sidebar(Rectangle::new(), 80.);
        }
        let mut p = pipeline(view);
        p.set_safe_area_insets(EdgeInsets::new(10., 20., 12., 14.));
        p.render_for_present().unwrap();
        let left = 10. + if sidebar { 80. } else { 0. };
        let top = 20. + if header { 60. } else { 0. };
        assert_eq!(
            shell(&p).children()[0].bounds(),
            Rect::from_xywh(left, top, 388. - left, 286. - top)
        );
    }
}

#[test]
fn scrolling_repaints_the_live_header_backdrop_without_escaping_the_viewport() {
    let mut p = pipeline(
        ContentShell::scrolling(
            ScrollView::new(Rectangle::new().fill(Color::rgb(1., 0., 0.)))
                .vertical()
                .content_size(0., 1000.)
                .wheel_sensitivity(1.)
                .scrollbar_visibility(ScrollbarVisibility::Never),
        )
        .header(
            Surface::new(Rectangle::new().fill(Color::CLEAR), SurfaceRole::Structural)
                .material(GlassMaterial::regular())
                .corner_radius(0.),
            60.,
        ),
    );
    let PresentedFrame::Cpu { buffer: before, .. } = p.render_for_present().unwrap() else {
        panic!()
    };
    let before_overlap = before.get_pixel(150, 52);
    let before_top = before.get_pixel(150, 4);
    wheel(&mut p, 80);
    let PresentedFrame::Cpu { buffer: after, .. } = p.render_for_present().unwrap() else {
        panic!()
    };
    assert_ne!(
        before_overlap,
        after.get_pixel(150, 52),
        "header samples the scrolled content beneath it"
    );
    assert_eq!(
        before_top,
        after.get_pixel(150, 4),
        "underlap stays bounded instead of filling the entire header"
    );
}

#[test]
fn floating_navigation_reserves_underlap_and_disabled_headers_remove_the_slot() {
    let links = (NavigationLink::new("Page", || {
        ScrollView::new(Rectangle::new())
            .vertical()
            .content_size(0., 1000.)
    }),);
    for (floating, underlap) in [(true, 16.), (false, 0.)] {
        let view = NavigationView::new(links.clone())
            .presentation(NavigationPresentation::Sidebar)
            .sidebar_width(80.)
            .sidebar_style(if floating {
                NavigationSidebarStyle::Floating
            } else {
                NavigationSidebarStyle::Simple
            })
            .content_underlap(16.)
            .header(|| Rectangle::new())
            .header_height(0.);
        let mut p = pipeline(view);
        p.render_for_present().unwrap();
        let navigation = &shell(&p).children()[0];
        assert_eq!(
            navigation.children().len(),
            2,
            "disabled header has no child slot"
        );
        let scroll = &navigation.children()[0];
        assert_eq!(
            scroll.bounds(),
            Rect::from_xywh(80. - underlap, 0., 320. + underlap, 300.)
        );
        assert_eq!(scroll.children()[0].position(), Point::new(underlap, 0.));
    }
}

#[test]
fn header_controls_receive_hits_before_scrolled_content_beneath_them() {
    use scarlet_ui_core::MouseButton;
    use std::{cell::Cell, rc::Rc};
    let header_clicks = Rc::new(Cell::new(0));
    let content_clicks = Rc::new(Cell::new(0));
    let header = header_clicks.clone();
    let content = content_clicks.clone();
    let mut p = pipeline(
        ContentShell::scrolling(
            ScrollView::new(Rectangle::new().on_click(move || content.set(content.get() + 1)))
                .vertical()
                .content_size(0., 1000.)
                .wheel_sensitivity(1.),
        )
        .header(
            Button::new("Header").on_click(move || header.set(header.get() + 1)),
            60.,
        ),
    );
    p.render_for_present().unwrap();
    wheel(&mut p, 80);
    p.render_for_present().unwrap();
    p.handle_event(&Event::Mouse(MouseEvent::Moved { x: 150, y: 52 }));
    for event in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 150,
            y: 52,
            click_count: 1,
        },
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 150,
            y: 52,
            click_count: 1,
        },
    ] {
        p.handle_event(&Event::Mouse(event));
    }
    assert_eq!(header_clicks.get(), 1);
    assert_eq!(content_clicks.get(), 0);
}
