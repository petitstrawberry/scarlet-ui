//! Background View Modifier
//!
//! Adds a background color behind a child view.

use crate::color::Color;
use crate::element::LayoutConstraints;
use crate::element::{Element, ElementRenderObject, RenderElement, UpdateResult};
use crate::event::{Event, MouseEvent, Phase};
use crate::geometry::{Point, Rect, Size};
use crate::renderer::PaintContext;
use crate::view::View;
use alloc::boxed::Box;
use alloc::vec;
use core::any::Any;

/// Background view modifier - adds a background color
#[derive(Clone)]
pub struct Background<V: View> {
    inner: V,
    color: Color,
    hover_color: Option<Color>,
}

impl<V: View> Background<V> {
    /// Create a new Background modifier
    pub fn new(inner: V, color: Color) -> Self {
        Self {
            inner,
            color,
            hover_color: None,
        }
    }

    /// Paint this color while the pointer is over the view.
    ///
    /// Hover stays in the render object, so changing it repaints this view
    /// without rebuilding application state or laying out its children.
    pub fn hover_color(mut self, color: Color) -> Self {
        self.hover_color = Some(color);
        self
    }

    /// Get the inner view
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Get the background color
    pub fn background_color(&self) -> Color {
        self.color
    }
}

impl<V: View + Clone> View for Background<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| {
                let mut render = BackgroundRenderObject::new(view.color);
                render.hover_color = view.hover_color;
                render
            },
            |render, view| {
                if render.color == view.color && render.hover_color == view.hover_color {
                    return UpdateResult::NoChange;
                }
                render.color = view.color;
                render.hover_color = view.hover_color;
                UpdateResult::Updated
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

/// Background RenderObject
pub struct BackgroundRenderObject {
    color: Color,
    hover_color: Option<Color>,
    hovered: bool,
    size: Size,
}

impl BackgroundRenderObject {
    /// Create a new BackgroundRenderObject
    pub fn new(color: Color) -> Self {
        Self {
            color,
            hover_color: None,
            hovered: false,
            size: Size::ZERO,
        }
    }

    /// Get the background color
    pub fn get_color(&self) -> Color {
        self.color
    }

    fn current_color(&self) -> Color {
        if self.hovered {
            self.hover_color.unwrap_or(self.color)
        } else {
            self.color
        }
    }
}

impl ElementRenderObject for BackgroundRenderObject {
    fn update_needs_layout(&self) -> bool {
        false
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if self.hover_color.is_none() || !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }
        let hovered = match event {
            Event::Mouse(MouseEvent::Entered { .. }) => true,
            Event::Mouse(MouseEvent::Exited { .. }) => false,
            _ => return false,
        };
        let previous = self.current_color();
        self.hovered = hovered;
        self.current_color() != previous
    }

    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        // Background takes at least the minimum size
        let width = constraints.min_width.max(1.0);
        let height = constraints.min_height.max(1.0);

        self.size = Size { width, height };
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let mut child_size = Size::ZERO;
        for child in children {
            let size = child.layout(constraints);
            child_size.width = child_size.width.max(size.width);
            child_size.height = child_size.height.max(size.height);
            child.set_position(Point::ZERO);
        }

        // A child can report zero on one unconstrained axis while retaining a
        // meaningful size on the other axis. Keep that meaningful dimension:
        // replacing the whole size with the fallback would, for example,
        // collapse a fixed-height fill-width view to one pixel during a stack's
        // measurement pass.
        self.size = constraints.constrain(Size {
            width: child_size.width.max(1.0),
            height: child_size.height.max(1.0),
        });
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: Point) -> bool {
        let bounds = crate::geometry::Rect {
            origin: Point::ZERO,
            size: self.size,
        };
        bounds.contains(point)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        // Modifier doesn't directly render - child handles its own rendering
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        ctx.fill_rect(Rect::new(origin, self.size), self.current_color());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::RenderingPipeline;
    use crate::state::{State, StateId};
    use crate::view::ViewExt;
    use crate::views::{GridView, Spacer, Window};
    use alloc::rc::Rc;
    use core::cell::Cell;

    #[test]
    fn preserves_the_nonzero_child_axis_during_loose_measurement() {
        let mut element = Background::new(Spacer::new().frame(f32::INFINITY, 450.0), Color::WHITE)
            .create_element();

        let size = element.layout(LayoutConstraints::new(0.0, f32::INFINITY, 0.0, 564.0));

        assert_eq!(size, Size::new(1.0, 450.0));
    }

    #[test]
    fn hover_repaints_cells_without_rebuilding_the_grid() {
        let builds = Rc::new(Cell::new(0));
        let count = builds.clone();
        let grid = GridView::new(
            State::new(StateId::new(8901), (0..200).collect::<alloc::vec::Vec<_>>()),
            State::new(StateId::new(8902), None),
            4,
            60.0,
            move |_, _: i32, _| {
                count.set(count.get() + 1);
                Spacer::new()
                    .frame(f32::INFINITY, 60.0)
                    .background(Color::BLACK)
                    .hover_color(Color::WHITE)
            },
        )
        .spacing(0.0);
        let root = Window::new("hover", grid)
            .decorated(false)
            .size(Size::new(400.0, 300.0));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(root.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage().unwrap();
        let initial_builds = builds.get();
        assert!(
            initial_builds > 0 && initial_builds < 200,
            "grid must remain virtualized"
        );

        for (x, expected_bounds) in [(20, (0, 0, 100, 60)), (120, (0, 0, 200, 60))] {
            pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x, y: 20 }));
            let (buffer, damage) = pipeline.render_with_damage().expect("hover color changes");
            assert_eq!(buffer.get_pixel(x as u32, 20), Some(Color::WHITE.to_bgra()));
            if x > 100 {
                assert_eq!(buffer.get_pixel(20, 20), Some(Color::BLACK.to_bgra()));
            }
            let damage = damage.expect("hover damage stays local");
            assert!(!damage.is_empty());
            assert!(
                damage.iter().all(|&(dx, dy, w, h)| {
                    dx >= expected_bounds.0
                        && dy >= expected_bounds.1
                        && dx + w <= expected_bounds.2
                        && dy + h <= expected_bounds.3
                }),
                "only the old and new cells need paint: {damage:?}"
            );
            assert_eq!(
                builds.get(),
                initial_builds,
                "hover must not rebuild any grid cells"
            );
            pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: x + 1, y: 21 }));
            assert!(
                !pipeline.has_dirty(),
                "motion inside one cell changes no pixels"
            );
        }
        pipeline.handle_event(&Event::Mouse(MouseEvent::Exited { x: -1, y: -1 }));
        assert!(
            pipeline.render_with_damage().is_some(),
            "exit must clear the hover"
        );
        assert_eq!(builds.get(), initial_builds);
    }

    #[test]
    fn changing_background_style_preserves_pointer_hover() {
        let view = Spacer::new()
            .frame(30.0, 30.0)
            .background(Color::BLACK)
            .hover_color(Color::WHITE);
        let mut element = view.create_element();
        element.layout(LayoutConstraints::tight(30.0, 30.0));
        element.handle_event(
            &Event::Mouse(MouseEvent::Entered { x: 1, y: 1 }),
            Phase::Target,
        );
        let updated = Spacer::new()
            .frame(30.0, 30.0)
            .background(Color::BLACK)
            .hover_color(Color::RED);
        element.update(&updated);
        let render = element
            .render_object()
            .unwrap()
            .as_any()
            .downcast_ref::<BackgroundRenderObject>()
            .unwrap();
        assert_eq!(render.current_color(), Color::RED);
        element.handle_event(
            &Event::Mouse(MouseEvent::Exited { x: -1, y: -1 }),
            Phase::Target,
        );
        let render = element
            .render_object()
            .unwrap()
            .as_any()
            .downcast_ref::<BackgroundRenderObject>()
            .unwrap();
        assert_eq!(render.current_color(), Color::BLACK);
    }
}
