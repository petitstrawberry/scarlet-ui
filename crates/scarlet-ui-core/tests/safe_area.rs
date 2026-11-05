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
