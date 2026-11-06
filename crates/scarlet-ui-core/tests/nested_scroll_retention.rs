//! Public-backend regression for hierarchical scroll pictures. Inner motion
//! preserves its content recording; outer-only motion also preserves the parent.
//! Renderer-specific tests verify that nested raster caches are actually used.

use scarlet_ui_core::{
    buffer::Buffer,
    compositor::DamageRect,
    event::{Event, MouseEvent, ScrollSource, WheelPhase},
    pipeline::RenderingPipeline,
    prelude::*,
    renderer::{BackendFrame, CpuPaintRenderer, PaintBackend, PaintCommand, PaintContext},
};
use std::{any::Any, cell::RefCell, rc::Rc};

const INNER_MARKERS: u16 = (1 << 5) - 1;

fn marker_color(index: u8) -> Color {
    Color::rgb(37u8 + index * 19, 71u8 + index * 11, 193u8 - index * 13)
}

fn marker(index: u8, width: f32, height: f32) -> impl View + Clone {
    Rectangle::new()
        .fill(marker_color(index))
        .frame(width, height)
}

fn viewport(depth: u8) -> Size {
    Size::new(
        180. + 20. * f32::from(depth - 1),
        100. + 60. * f32::from(depth - 1),
    )
}

#[derive(Clone)]
struct NestedScroll(u8);

impl View for NestedScroll {
    fn create_element(&self) -> Box<dyn Element> {
        let size = viewport(self.0);
        if self.0 == 1 {
            ScrollView::new(
                VStack::new((
                    marker(0, size.width, 60.),
                    marker(1, size.width, 60.),
                    marker(2, size.width, 60.),
                    marker(3, size.width, 60.),
                    marker(4, size.width, 60.),
                ))
                .spacing(0.),
            )
            .content_size(size.width, 300.)
            .scrollbar_visibility(ScrollbarVisibility::Never)
            .wheel_sensitivity(0.25)
            .frame(size.width, size.height)
            .create_element()
        } else {
            let sibling = 5 + 2 * (self.0 - 2);
            ScrollView::new(
                VStack::new((
                    marker(sibling, size.width, 16.),
                    NestedScroll(self.0 - 1),
                    marker(sibling + 1, size.width, 24.),
                    Rectangle::new().fill(Color::WHITE).frame(size.width, 240.),
                ))
                .spacing(0.)
                .alignment(Alignment::TopLeading),
            )
            .content_size(size.width, viewport(self.0 - 1).height + 280.)
            .scrollbar_visibility(ScrollbarVisibility::Never)
            .wheel_sensitivity(0.25)
            .frame(size.width, size.height)
            .create_element()
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone, Debug)]
struct Recording {
    id: u64,
    cache_identity: u64,
    origin: Point,
    raster_bounds: Option<Rect>,
    cached_ancestors: Vec<u64>,
    own_markers: u16,
    subtree_markers: u16,
}

fn command_marker(command: &PaintCommand) -> u16 {
    let color = match command {
        PaintCommand::FillPath { color, .. } | PaintCommand::FillRoundedRect { color, .. } => color,
        _ => return 0,
    };
    (0..9)
        .find(|&index| *color == marker_color(index))
        .map_or(0, |index| 1 << index)
}

fn capture_lists(
    context: &PaintContext<'_>,
    origin: Point,
    cached_ancestors: &[u64],
    recordings: &mut Vec<Recording>,
) -> u16 {
    let mut markers = 0;
    for command in context.commands() {
        markers |= command_marker(command);
        if let PaintCommand::DrawDisplayList {
            origin: local,
            list,
        } = command
        {
            let origin = Point::new(origin.x + local.x, origin.y + local.y);
            let mut ancestors = cached_ancestors.to_vec();
            if list.raster_cache_bounds().is_some() {
                ancestors.push(list.identity());
            }
            let subtree_markers = capture_lists(list.context(), origin, &ancestors, recordings);
            recordings.push(Recording {
                id: list.identity(),
                cache_identity: list.cache_identity(),
                origin,
                raster_bounds: list.raster_cache_bounds(),
                cached_ancestors: cached_ancestors.to_vec(),
                own_markers: list
                    .context()
                    .commands()
                    .iter()
                    .map(command_marker)
                    .fold(0, |a, b| a | b),
                subtree_markers,
            });
            markers |= subtree_markers;
        }
    }
    markers
}

#[derive(Default)]
struct Captured {
    recordings: Vec<Recording>,
    pixels: Option<Buffer>,
    frames: usize,
}

struct CaptureBackend {
    retained: bool,
    renderer: CpuPaintRenderer,
    captured: Rc<RefCell<Captured>>,
}

impl PaintBackend for CaptureBackend {
    fn supports_retained_display_lists(&self) -> bool {
        self.retained
    }

    fn resize(&mut self, size: Size, scale_milli: u32) {
        self.renderer.resize(size, scale_milli);
    }

    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        background: Color,
        damage: Option<&[Rect]>,
        _: Option<&[DamageRect]>,
    ) -> scarlet_ui_core::Result<BackendFrame<'a>> {
        let mut captured = self.captured.borrow_mut();
        captured.recordings.clear();
        capture_lists(context, Point::ZERO, &[], &mut captured.recordings);
        self.renderer.set_background_color(background);
        // The oracle always rasterizes the complete, non-retained scene.
        self.renderer
            .execute_with_damage(context, if self.retained { damage } else { None });
        captured.pixels = Some(self.renderer.buffer().clone());
        captured.frames += 1;
        Ok(BackendFrame::External)
    }
}

struct Harness {
    pipeline: RenderingPipeline,
    captured: Rc<RefCell<Captured>>,
}

impl Harness {
    fn new(depth: u8, retained: bool) -> Self {
        let size = viewport(depth);
        let captured = Rc::new(RefCell::new(Captured::default()));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_paint_backend(Box::new(CaptureBackend {
            retained,
            renderer: CpuPaintRenderer::new(size, 1000, Color::WHITE),
            captured: captured.clone(),
        }));
        pipeline.set_root(
            Window::new("Nested scroll retention", NestedScroll(depth))
                .decorated(false)
                .shadow(false)
                .corner_radius(0.)
                .size(size)
                .create_element(),
        );
        pipeline.resize(size);
        pipeline.layout_initial();
        pipeline.render_for_present().unwrap();
        Self { pipeline, captured }
    }

    fn scroll(&mut self, point: Point, delta_y: i32) {
        assert!(
            self.pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
                x: point.x as i32,
                y: point.y as i32,
                delta_x: 0,
                delta_y,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            })),
            "a scroll viewport must consume delta {delta_y} at {point:?}"
        );
        let previous_frames = self.captured.borrow().frames;
        self.pipeline.render_for_present().unwrap();
        assert_eq!(self.captured.borrow().frames, previous_frames + 1);
    }

    fn end_gesture(&mut self, point: Point) {
        self.pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            x: point.x as i32,
            y: point.y as i32,
            delta_x: 0,
            delta_y: 0,
            phase: WheelPhase::Ended,
            source: ScrollSource::Trackpad,
        }));
        self.pipeline.render_for_present().unwrap();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.pipeline.teardown();
    }
}

fn paint_record(recordings: &[Recording], marker: u8) -> &Recording {
    recordings
        .iter()
        .find(|record| record.own_markers & (1 << marker) != 0)
        .unwrap_or_else(|| panic!("missing paint record for marker {marker}"))
}

fn inner_cache_units(recordings: &[Recording]) -> Vec<Recording> {
    let units: Vec<_> = recordings
        .iter()
        .filter(|record| {
            record.raster_bounds.is_some()
                && record.subtree_markers & INNER_MARKERS != 0
                && record.subtree_markers & !INNER_MARKERS == 0
        })
        .cloned()
        .collect();
    assert_eq!(
        units
            .iter()
            .fold(0, |markers, record| markers | record.subtree_markers),
        INNER_MARKERS,
        "inner content must remain reachable as cacheable DrawDisplayLists, including inside a parent picture. Cached records: {:?}",
        recordings
            .iter()
            .filter(|record| record.raster_bounds.is_some())
            .collect::<Vec<_>>()
    );
    units
}

fn parent_cache(recordings: &[Recording], depth: u8) -> &Recording {
    let sibling_markers = 3 << (5 + 2 * (depth - 2));
    recordings.iter().find(|record| {
        record.raster_bounds.is_some()
            && record.subtree_markers & INNER_MARKERS == INNER_MARKERS
            && record.subtree_markers & sibling_markers == sibling_markers
    }).expect("the outer scroll must retain a parent picture containing the inner viewport and static siblings")
}

// Pick each viewport by its distinct fixture size, not by element-tree shape.
// Its right-hand gutter is outside the nested viewport, so the event targets
// that scroll owner rather than accidentally bubbling from an exhausted child.
fn scroll_hit(element: &dyn Element, parent: Point, size: Size) -> Option<Point> {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if element
        .render_object()
        .is_some_and(|render| render.owns_child_viewport())
        && element.bounds().size == size
    {
        return Some(Point::new(
            origin.x + size.width - 6.,
            origin.y + size.height / 2.,
        ));
    }
    element
        .children()
        .iter()
        .find_map(|child| scroll_hit(child.as_ref(), origin, size))
}

fn exercise(depth: u8, check_retention: bool) {
    let mut retained = Harness::new(depth, true);
    let mut full = Harness::new(depth, false);
    let initial = retained.captured.borrow().recordings.clone();
    let first = paint_record(&initial, 0);
    let initial_units = check_retention.then(|| inner_cache_units(&initial));
    let parent_lineage = check_retention.then(|| parent_cache(&initial, depth).cache_identity);
    let mut previous_origin = first.origin;
    let mut previous_pixels = retained.captured.borrow().pixels.clone().unwrap();
    let mut offsets = vec![0.; usize::from(depth + 1)];

    assert!(
        full.captured.borrow().recordings.is_empty(),
        "oracle must not consume retained lists"
    );
    assert!(previous_pixels.data() == full.captured.borrow().pixels.as_ref().unwrap().data());
    // Fractional offsets, reversals, and switching between moving the inner
    // viewport and moving an ancestor while the inner viewport is stopped.
    let mut gestures: Vec<(u8, &[i32])> = vec![
        (1, &[-3, -17, -44, -96, 20, 140]),
        (depth, &[-3, -13, -28, 12]),
        (1, &[-17, -23, 12, 28]),
    ];
    if depth == 3 {
        gestures.push((2, &[-8, -24, 16, 16]));
    }
    gestures.push((depth, &[32]));
    for (level, deltas) in gestures {
        let hit = scroll_hit(
            retained.pipeline.element_tree().root().unwrap(),
            Point::ZERO,
            viewport(level),
        )
        .expect("fixture must contain the requested scroll viewport");
        for &delta in deltas {
            let previous_parent = check_retention
                .then(|| parent_cache(&retained.captured.borrow().recordings, depth).clone());
            retained.scroll(hit, delta);
            full.pipeline.request_redraw();
            full.scroll(hit, delta);
            offsets[usize::from(level)] -= delta as f32 * 0.25;
            let current = retained.captured.borrow();
            let pixels = current.pixels.as_ref().unwrap();
            assert!(
                pixels.data() == full.captured.borrow().pixels.as_ref().unwrap().data(),
                "{depth}-level scene differs from full redraw after level {level}, delta {delta}"
            );
            assert!(
                pixels.data() != previous_pixels.data(),
                "input must visibly move the content"
            );
            let moved = paint_record(&current.recordings, 0);
            assert_ne!(moved.origin, previous_origin);
            let total_offset: f32 = offsets.iter().sum();
            assert_eq!(
                moved.origin,
                Point::new(first.origin.x, first.origin.y - total_offset)
            );

            if let Some(initial_units) = &initial_units {
                let units = inner_cache_units(&current.recordings);
                let parent = parent_cache(&current.recordings, depth);
                assert_eq!(
                    Some(parent.cache_identity),
                    parent_lineage,
                    "inner motion must preserve the parent cache lineage"
                );
                if level == depth {
                    let old = previous_parent.as_ref().unwrap();
                    assert_eq!(
                        parent.id, old.id,
                        "outer-only motion must reuse the whole parent picture while inner content is stopped"
                    );
                    assert_eq!(parent.raster_bounds, old.raster_bounds);
                    assert_eq!(
                        parent.origin,
                        Point::new(old.origin.x, old.origin.y + delta as f32 * 0.25)
                    );
                }
                for old in initial_units {
                    let new = units.iter().find(|record| record.id == old.id).expect(
                        "inner content must reuse its cached recording when either viewport moves",
                    );
                    assert_eq!(new.raster_bounds, old.raster_bounds);
                    assert!(
                        new.cached_ancestors.contains(&parent.id),
                        "the inner picture must remain composable within the parent cache"
                    );
                    assert_eq!(
                        new.origin,
                        Point::new(old.origin.x, old.origin.y - total_offset)
                    );
                }
                for index in 0..(5 + 2 * (depth - 1)) {
                    let old = paint_record(&initial, index);
                    let new = paint_record(&current.recordings, index);
                    assert_eq!(new.id, old.id, "unchanged marker {index} was rerecorded");
                    let owner_level = if index < 5 { 1 } else { 2 + (index - 5) / 2 };
                    let expected_offset: f32 = offsets[usize::from(owner_level)..].iter().sum();
                    assert_eq!(
                        new.origin,
                        Point::new(old.origin.x, old.origin.y - expected_offset),
                        "marker {index} must move only with its scroll owner and ancestors"
                    );
                }
            }
            previous_origin = moved.origin;
            previous_pixels = pixels.clone();
        }
        retained.end_gesture(hit);
        full.end_gesture(hit);
    }
    assert!(
        offsets.iter().all(|&offset| offset == 0.),
        "gestures must return every viewport to its origin"
    );
}

#[test]
fn two_level_scroll_reuses_nested_parent_and_child_caches() {
    exercise(2, true);
}

#[test]
fn three_level_scroll_reuses_nested_parent_and_child_caches() {
    exercise(3, true);
}

#[test]
fn two_level_nested_scroll_matches_non_retained_full_redraw() {
    exercise(2, false);
}

#[test]
fn three_level_nested_scroll_matches_non_retained_full_redraw() {
    exercise(3, false);
}
