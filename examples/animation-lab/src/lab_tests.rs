use super::*;
use scarlet_ui::{
    RenderingPipeline,
    event::{Event, MouseEvent, ScrollSource, WheelPhase},
    renderer::{BackendFrame, PaintBackend},
    views::HeaderBarRenderObject,
};

struct HeadlessPaint;
impl PaintBackend for HeadlessPaint {
    fn supports_retained_display_lists(&self) -> bool {
        true
    }
    fn resize(&mut self, _: Size, _: u32) {}
    fn render<'a>(
        &'a mut self,
        _: &PaintContext<'_>,
        _: Color,
        _: Option<&[Rect]>,
        _: Option<&[scarlet_ui::compositor::DamageRect]>,
    ) -> scarlet_ui::Result<BackendFrame<'a>> {
        Ok(BackendFrame::External)
    }
}

fn bounds<T: 'static>(element: &dyn Element, parent: Point) -> Option<Rect> {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if element
        .render_object()
        .is_some_and(|render| render.as_any().is::<T>())
    {
        return Some(Rect::new(origin, element.bounds().size));
    }
    element
        .children()
        .iter()
        .find_map(|child| bounds::<T>(child.as_ref(), origin))
}

fn page_bounds(pipeline: &RenderingPipeline) -> Rect {
    bounds::<NaturalLayout>(pipeline.element_tree().root().unwrap(), Point::ZERO).unwrap()
}

fn header_bounds(pipeline: &RenderingPipeline) -> Rect {
    bounds::<HeaderBarRenderObject>(pipeline.element_tree().root().unwrap(), Point::ZERO).unwrap()
}

fn assert_page_stays_below_header(pipeline: &mut RenderingPipeline, gap: f32) {
    for _ in 0..3 {
        pipeline.render_for_present().unwrap();
        assert_eq!(
            page_bounds(pipeline).top(),
            header_bounds(pipeline).bottom() + gap,
            "width reconciliation and animation updates must not scroll the page"
        );
    }
}

#[test]
fn responsive_page_reconciliation_does_not_scroll_past_its_heading() {
    let app = AnimationLab::new(false);
    app.navigation().selected_index_state().set(0);
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_paint_backend(Box::new(HeadlessPaint));
    pipeline.set_root(
        Window::new("Animation Lab", app.clone())
            .content_safe_area(scarlet_ui::SafeAreaEdges::NONE)
            .size(Size::new(715.0, 830.0))
            .create_element(),
    );
    pipeline.layout_initial();
    let gap = page_bounds(&pipeline).top() - header_bounds(&pipeline).bottom();
    assert!(gap >= 0.0);
    assert_page_stays_below_header(&mut pipeline, gap);

    // The delayed load rebuilds Play without remounting its scroll view.
    app.play.boot();
    assert_page_stays_below_header(&mut pipeline, gap);
    pipeline.advance_visual_animations(Duration::from_millis(500));
    assert_page_stays_below_header(&mut pipeline, gap);

    // Cross both the navigation and card/control layout breakpoints in place.
    for width in [1120.0, 715.0, 390.0, 715.0] {
        pipeline.resize(Size::new(width, 830.0));
        assert_page_stays_below_header(&mut pipeline, gap);
    }
    for selected in [1, 0, 2, 0] {
        app.navigation().selected_index_state().set(selected);
        assert_page_stays_below_header(&mut pipeline, gap);
    }

    // Anchoring the page preserves deliberate scrolling as well; it must not
    // remount/reset the ScrollView on every geometry or animation update.
    pipeline.resize(Size::new(390.0, 830.0));
    assert_page_stays_below_header(&mut pipeline, gap);
    pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
        x: 140,
        y: 400,
        delta_x: 0,
        delta_y: -120,
        phase: WheelPhase::Started,
        source: ScrollSource::Trackpad,
    }));
    pipeline.render_for_present().unwrap();
    let scrolled_top = page_bounds(&pipeline).top();
    assert!(scrolled_top < header_bounds(&pipeline).bottom());
    for width in [500.0, 390.0] {
        pipeline.resize(Size::new(width, 830.0));
        for _ in 0..3 {
            pipeline.render_for_present().unwrap();
            assert_eq!(page_bounds(&pipeline).top(), scrolled_top);
        }
    }
    app.play.smoke_step(2); // Change palette without changing the scroll position.
    pipeline.render_for_present().unwrap();
    assert_eq!(page_bounds(&pipeline).top(), scrolled_top);
}
