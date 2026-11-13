use super::*;
use crate::source::{ExampleId, SourceFile};
use scarlet_ui::{
    RenderingPipeline,
    event::{Event, MouseButton, MouseEvent, ScrollSource, WheelPhase},
    renderer::{BackendFrame, PaintBackend, PaintCommand},
    views::{
        ButtonRenderObject, HeaderBarRenderObject, TextRenderObject,
        navigation::NavigationViewRenderObject,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

// navigation! retains selection in a process-wide registry keyed by call site.
// These fixtures all use AnimationLab::navigation, so they cannot run concurrently.
static LAB_PIPELINE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    let _lab = LAB_PIPELINE_TESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
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

#[test]
fn vertical_pages_accept_browser_wheel_samples_with_diagonal_jitter() {
    let _lab = LAB_PIPELINE_TESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let app = AnimationLab::new(false);
    for selected in 0..3 {
        app.navigation().selected_index_state().set(selected);
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_paint_backend(Box::new(HeadlessPaint));
        pipeline.set_root(
            Window::new("Animation Lab", app.clone())
                .content_safe_area(scarlet_ui::SafeAreaEdges::NONE)
                .size(Size::new(715.0, 640.0))
                .create_element(),
        );
        pipeline.layout_initial();
        for _ in 0..3 {
            pipeline.render_for_present().unwrap();
        }
        // Browsers report pixel wheels without a reliable device identity or
        // gesture boundaries. Vertical motion must not stop when a finger also
        // moves a few pixels sideways partway through the scroll.
        for (dx, dy) in [(0, -20), (4, -8), (-3, -8), (1, -4)] {
            let before = page_bounds(&pipeline).top();
            pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
                x: 500,
                y: 350,
                delta_x: dx,
                delta_y: dy,
                phase: WheelPhase::Moved,
                source: ScrollSource::Unknown,
            }));
            pipeline.render_for_present().unwrap();
            assert!(
                page_bounds(&pipeline).top() < before,
                "page {selected} dropped vertical wheel ({dx}, {dy})"
            );
        }
        // A genuinely horizontal sample still belongs to a horizontal child
        // or ancestor; loosening the page must not capture a guessed gesture.
        let before = page_bounds(&pipeline).top();
        pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            x: 500,
            y: 350,
            delta_x: -20,
            delta_y: -4,
            phase: WheelPhase::Moved,
            source: ScrollSource::Unknown,
        }));
        pipeline.render_for_present().unwrap();
        assert_eq!(page_bounds(&pipeline).top(), before);
    }
}

/// Keep the real retained paint/event path, without rasterizing entire source files.
#[derive(Default)]
struct RecordedFrame {
    size: Size,
    full_viewport_fills: Vec<Color>,
    text: Vec<String>,
}

struct RecordingPaint(Rc<RefCell<RecordedFrame>>);
impl PaintBackend for RecordingPaint {
    fn supports_retained_display_lists(&self) -> bool {
        true
    }
    fn resize(&mut self, size: Size, _: u32) {
        self.0.borrow_mut().size = size;
    }
    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        _: Color,
        _: Option<&[Rect]>,
        _: Option<&[scarlet_ui::compositor::DamageRect]>,
    ) -> scarlet_ui::Result<BackendFrame<'a>> {
        let mut frame = self.0.borrow_mut();
        frame.text.clear();
        frame.full_viewport_fills.clear();
        let viewport = Rect::new(Point::ZERO, frame.size);
        let mut clips = Vec::<(Rect, f32)>::new();
        let mut opacity = 1.0;
        context.visit_commands(&mut |command, offset| {
            let translated = |rect: Rect| {
                Rect::from_xywh(
                    rect.left() + offset.x,
                    rect.top() + offset.y,
                    rect.width(),
                    rect.height(),
                )
            };
            let fill = match command {
                PaintCommand::PushClip {
                    rect,
                    corner_radius,
                } => {
                    clips.push((translated(*rect), *corner_radius));
                    None
                }
                PaintCommand::PopClip => {
                    clips.pop().expect("balanced paint clips");
                    None
                }
                PaintCommand::SetOpacity { opacity: next } => {
                    opacity = *next;
                    None
                }
                PaintCommand::FillPath { path, color } => {
                    // A bounding box alone would accept triangles or other partial fills.
                    let expected = scarlet_ui::renderer::path_rect(Rect::new(
                        Point::new(-offset.x, -offset.y),
                        viewport.size,
                    ));
                    (path == &expected).then_some(*color)
                }
                PaintCommand::FillRoundedRect {
                    rect,
                    corner_radius,
                    color,
                } if *corner_radius == 0.0 && translated(*rect) == viewport => Some(*color),
                PaintCommand::DrawText { text, .. } => {
                    frame.text.push(text.clone());
                    None
                }
                _ => None,
            };
            if let Some(color) = fill
                && opacity == 1.0
                && clips
                    .iter()
                    .all(|(clip, radius)| *radius == 0.0 && contains_rect(*clip, viewport))
            {
                frame.full_viewport_fills.push(color);
            }
        });
        assert!(
            clips.is_empty(),
            "paint clips must not leak out of the frame"
        );
        Ok(BackendFrame::External)
    }
}

fn contains_rect(outer: Rect, inner: Rect) -> bool {
    inner.left() >= outer.left()
        && inner.top() >= outer.top()
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

fn settle(pipeline: &mut RenderingPipeline) {
    for _ in 0..3 {
        pipeline.render_for_present().unwrap();
    }
}

fn mounted_lab(
    app: &AnimationLab,
    width: f32,
    safe: scarlet_ui::EdgeInsets,
) -> (RenderingPipeline, Rc<RefCell<RecordedFrame>>) {
    app.selected.set(0);
    let recording = Rc::new(RefCell::new(RecordedFrame::default()));
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_paint_backend(Box::new(RecordingPaint(recording.clone())));
    pipeline.set_root(
        Window::new("Animation Lab", app.clone())
            .decorated(false)
            .shadow(false)
            .content_safe_area(scarlet_ui::SafeAreaEdges::NONE)
            .size(Size::new(width, 830.0))
            .create_element(),
    );
    pipeline.layout_initial();
    pipeline.set_safe_area_insets(safe);
    settle(&mut pipeline);
    (pipeline, recording)
}

fn visit_elements(
    element: &dyn Element,
    parent: Point,
    visit: &mut impl FnMut(&dyn Element, Rect),
) {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    visit(element, Rect::new(origin, element.bounds().size));
    for child in element.children() {
        visit_elements(child.as_ref(), origin, visit);
    }
}

fn buttons(pipeline: &RenderingPipeline, label: &str) -> Vec<(Rect, bool)> {
    let mut found = Vec::new();
    visit_elements(
        pipeline.element_tree().root().unwrap(),
        Point::ZERO,
        &mut |element, rect| {
            if let Some(button) = element
                .as_any()
                .downcast_ref::<RenderElement<Button, ButtonRenderObject>>()
                && button.view().label() == label
            {
                found.push((rect, button.render_object().is_mouse_pressed()));
            }
        },
    );
    found
}

fn has_text(pipeline: &RenderingPipeline, expected: &str) -> bool {
    let mut found = false;
    visit_elements(
        pipeline.element_tree().root().unwrap(),
        Point::ZERO,
        &mut |element, _| {
            if let Some(text) = element
                .as_any()
                .downcast_ref::<RenderElement<Text, TextRenderObject>>()
            {
                found |= text
                    .view()
                    .content()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    == expected;
            }
        },
    );
    found
}

fn body_viewport(pipeline: &RenderingPipeline) -> Rect {
    let root = pipeline.element_tree().root().unwrap();
    let safe = Rect::new(Point::ZERO, root.bounds().size).inset(pipeline.safe_area_insets());
    let header = header_bounds(pipeline);
    let mut navigation = None;
    visit_elements(root, Point::ZERO, &mut |element, rect| {
        if element
            .render_object()
            .is_some_and(|render| render.as_any().is::<NavigationViewRenderObject>())
        {
            // NavigationView's final child is its fixed destination bar/sidebar.
            let child = element.children().last().unwrap();
            navigation = Some(Rect::new(
                Point::new(
                    rect.left() + child.position().x,
                    rect.top() + child.position().y,
                ),
                child.bounds().size,
            ));
        }
    });
    let navigation = navigation.unwrap();
    let bottom = if navigation.top() >= header.bottom() {
        navigation.top()
    } else {
        safe.bottom()
    };
    Rect::from_xywh(
        header.left(),
        header.bottom(),
        header.width(),
        bottom - header.bottom(),
    )
}

/// Find controls from their live elements, then dispatch real pointer events.
/// Scrolling also goes through the dispatcher, including gesture termination.
fn click_button(pipeline: &mut RenderingPipeline, label: &str, index: usize) {
    for attempt in 0..12 {
        let matches = buttons(pipeline, label);
        let rect = matches
            .get(index)
            .unwrap_or_else(|| panic!("missing {label:?} button {index}"))
            .0;
        let visible = if label == "Back to demo" {
            header_bounds(pipeline)
        } else {
            body_viewport(pipeline)
        };
        if contains_rect(visible, rect) {
            let point = rect.center();
            let (x, y) = (point.x.round() as i32, point.y.round() as i32);
            pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x, y }));
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                click_count: 1,
            }));
            assert!(
                buttons(pipeline, label)[index].1,
                "pointer press must reach {label:?} #{index} at {rect:?}"
            );
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                y,
                click_count: 1,
            }));
            settle(pipeline);
            return;
        }
        assert!(
            attempt < 11,
            "could not reveal {label:?} #{index}: {rect:?}, viewport {visible:?}"
        );
        assert!(
            rect.left() >= visible.left() && rect.right() <= visible.right(),
            "{label:?} must fit the content width: {rect:?}, viewport {visible:?}"
        );
        let point = visible.center();
        let delta_y = (point.y - rect.center().y).round() as i32;
        for (phase, delta_y) in [(WheelPhase::Started, delta_y), (WheelPhase::Ended, 0)] {
            pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
                x: point.x.round() as i32,
                y: point.y.round() as i32,
                delta_x: 0,
                delta_y,
                phase,
                source: ScrollSource::Trackpad,
            }));
        }
        settle(pipeline);
    }
    unreachable!();
}

#[test]
fn demo_code_buttons_open_the_matching_source_and_header_returns_without_model_writes() {
    let _lab = LAB_PIPELINE_TESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cases = [
        (0, "View code", 0, ExampleId::Donut, SourceFile::Paint),
        (0, "View code", 1, ExampleId::Zigzag, SourceFile::Paint),
        (0, "View code", 2, ExampleId::Jelly, SourceFile::Paint),
        (0, "View code", 3, ExampleId::Confetti, SourceFile::Play),
        (1, "View code", 0, ExampleId::Explicit, SourceFile::Basics),
        (1, "View code", 1, ExampleId::Implicit, SourceFile::Basics),
        (2, "View code", 0, ExampleId::Charts, SourceFile::Layout),
        (2, "View code", 1, ExampleId::Charts, SourceFile::Layout),
        (2, "View code", 2, ExampleId::Charts, SourceFile::Layout),
    ];
    for width in [390.0, 1120.0] {
        let app = AnimationLab::new(false);
        let (mut pipeline, recording) = mounted_lab(&app, width, scarlet_ui::EdgeInsets::ZERO);
        // Subscribe only to the actual demo models, not navigation/source state.
        // This catches even accidental writes that leave the same visible labels.
        let play = app.play.page(width, CodeBrowser::default());
        let models: Vec<_> = play
            .listenables()
            .into_iter()
            .chain(app.basics.listenables())
            .chain(app.layout.listenables())
            .collect();
        let writes = Arc::new(AtomicUsize::new(0));
        let subscriptions: Vec<_> = models
            .iter()
            .map(|model| {
                let writes = writes.clone();
                model.subscribe_any(Arc::new(move || {
                    writes.fetch_add(1, Ordering::SeqCst);
                }))
            })
            .collect();
        for (page, label, index, example, file) in cases {
            app.selected.set(page);
            settle(&mut pipeline);
            click_button(&mut pipeline, label, index);
            assert_eq!(
                app.code[page].selected.get(),
                Some(example),
                "{width}px: {label} #{index}"
            );
            assert!(
                has_text(&pipeline, example.example().title),
                "the matching walkthrough must be mounted"
            );
            assert_eq!(
                buttons(&pipeline, "Copy excerpt").len(),
                example.example().sections.len()
            );
            assert!(buttons(&pipeline, "Copy file").is_empty());
            assert!(has_text(&pipeline, "Source code"));

            click_button(&mut pipeline, file.name(), 0);
            assert_eq!(
                buttons(&pipeline, "Copy file").len(),
                1,
                "{width}px {example:?}: clicking {} must mount full source, location={}",
                file.name(),
                app.code[page].location_key()
            );
            assert!(buttons(&pipeline, "Copy excerpt").is_empty());
            assert!(
                has_text(&pipeline, file.name()),
                "full file caption must be rendered"
            );
            // Short source lines survive wrapping even at the narrow size.
            let source_line = file
                .contents()
                .lines()
                .find(|line| {
                    line.len() >= 10 && line.len() <= 22 && line.chars().any(char::is_alphabetic)
                })
                .unwrap();
            assert!(
                recording
                    .borrow()
                    .text
                    .iter()
                    .any(|line| line == source_line),
                "the full file's actual source text must reach the paint backend: {source_line:?}"
            );

            click_button(&mut pipeline, "Back to demo", 0);
            assert!(!app.code[page].is_open());
            assert!(app.code.iter().all(|browser| !browser.is_open()));
            assert!(has_text(&pipeline, "Animation Lab"));
            assert!(buttons(&pipeline, "Back to demo").is_empty());
            assert!(
                !buttons(&pipeline, label).is_empty(),
                "demo controls must return"
            );
            assert_eq!(
                app.selected.get(),
                page,
                "return must preserve the navigation destination"
            );
            assert_eq!(
                writes.load(Ordering::SeqCst),
                0,
                "opening/reading/returning from {example:?} must not write demo state ({width}px)"
            );
        }
        for (model, subscription) in models.into_iter().zip(subscriptions) {
            model.unsubscribe(subscription);
        }
    }
}

fn assert_safe_page(
    pipeline: &RenderingPipeline,
    recording: &RefCell<RecordedFrame>,
    expected: Color,
) {
    let frame = recording.borrow();
    assert!(
        frame.full_viewport_fills.contains(&expected),
        "page background must paint the complete viewport without a safe-area clip; expected {expected:?}, got {:?}",
        frame.full_viewport_fills
    );
    let safe = Rect::new(Point::ZERO, frame.size).inset(pipeline.safe_area_insets());
    let header = header_bounds(pipeline);
    assert!(
        contains_rect(safe, header),
        "header must remain safe: {header:?} within {safe:?}"
    );
    let page = page_bounds(pipeline);
    assert!(
        page.left() >= safe.left() && page.right() <= safe.right(),
        "page content must respect horizontal safe edges: {page:?} within {safe:?}"
    );
    assert!(
        page.top() >= header.bottom(),
        "initial page content must start below the safe header"
    );
    for (back, _) in buttons(pipeline, "Back to demo") {
        assert!(
            contains_rect(header, back),
            "the return control must stay inside the safe header"
        );
    }
}

#[test]
fn page_background_reaches_all_viewport_edges_while_demo_and_source_controls_stay_safe() {
    let _lab = LAB_PIPELINE_TESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let insets = scarlet_ui::EdgeInsets::new(17.0, 47.0, 23.0, 31.0);
    for width in [390.0, 1120.0] {
        let app = AnimationLab::new(false);
        let (mut pipeline, recording) = mounted_lab(&app, width, insets);
        for (page, label) in [(0, "View code"), (1, "View code"), (2, "View code")] {
            app.selected.set(page);
            settle(&mut pipeline);
            let expected = if page == 2 {
                app.layout.background_color()
            } else {
                SURFACE
            };
            assert_eq!(app.surface(), expected);
            assert_safe_page(&pipeline, &recording, expected);

            click_button(&mut pipeline, label, 0);
            assert_eq!(app.surface(), crate::code::SURFACE);
            assert_safe_page(&pipeline, &recording, crate::code::SURFACE);
            click_button(&mut pipeline, "Back to demo", 0);
            assert_safe_page(&pipeline, &recording, expected);
        }
        let light = app.layout.background_color();
        click_button(&mut pipeline, "Dark theme", 0);
        pipeline.advance_visual_animations(Duration::from_millis(150));
        settle(&mut pipeline);
        let midway = *recording.borrow().full_viewport_fills.last().unwrap();
        assert_ne!(
            midway, light,
            "the full-window surface must begin animating"
        );
        assert_ne!(
            midway,
            app.layout.background_color(),
            "safe-area background must share the transition, not jump to the target"
        );
        pipeline.advance_visual_animations(Duration::from_millis(1000));
        settle(&mut pipeline);
        let dark = app.layout.background_color();
        assert_ne!(
            dark, light,
            "the fixture must exercise a real surface color change"
        );
        assert_safe_page(&pipeline, &recording, dark);
        click_button(&mut pipeline, "View code", 0);
        assert_safe_page(&pipeline, &recording, crate::code::SURFACE);
        click_button(&mut pipeline, "Back to demo", 0);
        assert_safe_page(&pipeline, &recording, dark);
    }
}
