//! ProgressView View - Progress indicator
//!
//! ProgressView displays the progress of a long-running task.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, RenderElement};
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::renderer::PaintContext;
use crate::view::View;
use crate::views::style;
use alloc::boxed::Box;
use core::any::Any;

/// ProgressView View - shows progress (0.0 to 1.0)
#[derive(Clone)]
pub struct ProgressView {
    value: f32,
}

impl ProgressView {
    /// Create a new ProgressView
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
        }
    }

    /// Set progress value (0.0 to 1.0)
    pub fn value(mut self, value: f32) -> Self {
        self.value = value.clamp(0.0, 1.0);
        self
    }

    /// Get value
    pub fn get_value(&self) -> f32 {
        self.value
    }
}

impl View for ProgressView {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            ProgressViewRenderObject::new(self.value),
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        alloc::vec::Vec::new()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// ProgressView RenderObject
///
/// Design matching macOS/iOS progress bar:
/// - Height: 4px (determinate), 20px (indeterminate)
/// - Width: flexible (at least 100px)
/// - Background: Light gray (#E5E5EA)
/// - Fill: Blue (#007AFF)
/// - Fully rounded
pub struct ProgressViewRenderObject {
    value: f32,
    size: Size,
    buffer: Option<Buffer>,
}

impl ProgressViewRenderObject {
    /// Create a new ProgressViewRenderObject
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            size: Size::new(200.0, 4.0),
            buffer: None,
        }
    }

    /// Get value
    pub fn get_value(&self) -> f32 {
        self.value
    }

    /// Set value
    pub fn set_value(&mut self, value: f32) {
        self.value = value.clamp(0.0, 1.0);
    }

    // Layout allocation and visible track thickness are independent. A frame
    // can enlarge the control without moving the track away from its center.
    fn track_rect(&self, origin: Point) -> Rect {
        let height = self.size.height.min(4.0).max(0.0);
        Rect::from_xywh(
            origin.x,
            origin.y + (self.size.height - height) * 0.5,
            self.size.width,
            height,
        )
    }

    fn draw_progress(&mut self) {
        let width = libm::ceilf(self.size.width) as u32;
        let height = libm::ceilf(self.size.height) as u32;
        if self.buffer.as_ref().is_none_or(|buffer| {
            buffer.logical_width() != width || buffer.logical_height() != height
        }) {
            self.buffer = Some(Buffer::from_logical_dimensions(width, height));
        }
        let rect = self.track_rect(Point::ZERO);
        if let Some(ref mut buffer) = self.buffer {
            let mut canvas = graphics::Canvas::for_buffer(buffer);
            canvas.fill_rect(0, 0, canvas.width(), canvas.height(), Color::TRANSPARENT);
            let palette = ColorPalette::default();
            let y = libm::roundf(rect.origin.y) as i32;
            let width = libm::ceilf(rect.size.width) as u32;
            let height = libm::ceilf(rect.size.height) as u32;
            canvas.fill_rect(
                0,
                y,
                width,
                height,
                style::surface_color(&palette, style::SurfaceRole::Section),
            );
            canvas.fill_rect(
                0,
                y,
                (self.value * rect.size.width) as u32,
                height,
                palette.primary(),
            );
        }
    }
}

impl ElementRenderObject for ProgressViewRenderObject {
    fn layout(&mut self, constraints: crate::element::LayoutConstraints) -> Size {
        // ProgressView has fixed height (4px), flexible width
        let width = if constraints.max_width.is_finite() && constraints.max_width > 0.0 {
            constraints.max_width.max(constraints.min_width).max(100.0) // Min 100px width
        } else {
            constraints.min_width.max(200.0)
        };

        self.size = constraints.constrain(Size::new(width, 4.0));
        let Size { width, height } = self.size;

        // Create buffer
        let w = libm::ceilf(width) as u32;
        let h = libm::ceilf(height) as u32;
        let needs_resize = self
            .buffer
            .as_ref()
            .map_or(true, |b| b.logical_width() != w || b.logical_height() != h);
        if needs_resize {
            self.buffer = Some(Buffer::from_logical_dimensions(w, h));
        }

        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        self.draw_progress();
    }

    fn get_buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    fn clear_buffer(&mut self) {
        self.buffer = None;
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let palette = ColorPalette::default();
        let bg_color = style::surface_color(&palette, style::SurfaceRole::Section);
        let fill_color = palette.primary();
        let rect = self.track_rect(origin);
        style::track(ctx, rect, bg_color);
        let fill_width = self.size.width * self.value;
        if fill_width > 0.0 {
            style::track(
                ctx,
                Rect::from_xywh(rect.origin.x, rect.origin.y, fill_width, rect.size.height),
                fill_color,
            );
        }
        true
    }

    fn update(&mut self, new_view: &dyn crate::view::View) -> crate::element::UpdateResult {
        if let Some(progress) = new_view.as_any().downcast_ref::<ProgressView>() {
            let new_value = progress.value.clamp(0.0, 1.0);
            if (self.value - new_value).abs() > 0.001 {
                self.value = new_value;
                crate::element::UpdateResult::Updated
            } else {
                crate::element::UpdateResult::NoChange
            }
        } else {
            crate::element::UpdateResult::Replaced
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::LayoutConstraints;
    use crate::renderer::{CpuPaintRenderer, PaintCommand};

    #[test]
    fn framed_progress_centers_the_track_in_both_render_paths_and_can_shrink_again() {
        let mut progress = ProgressViewRenderObject::new(0.5);
        for height in [18., 28., 2., 0.] {
            assert_eq!(
                progress.layout(LayoutConstraints::tight(80., height)),
                Size::new(80., height)
            );
            progress.render();
            let mut paint = PaintContext::new();
            progress.paint(&mut paint, Point::new(3., 5.));
            let track_height = height.min(4.);
            let top = (height - track_height) * 0.5;
            let PaintCommand::FillRoundedRect { rect, .. } = &paint.commands()[0] else {
                panic!("missing progress track")
            };
            assert_eq!(*rect, Rect::from_xywh(3., 5. + top, 80., track_height));
            let mut gpu_commands =
                CpuPaintRenderer::new(Size::new(90., height + 10.), 1000, Color::TRANSPARENT);
            gpu_commands.execute(&paint);
            for y in 0..height as u32 {
                let inside = y >= top as u32 && y < (top + track_height) as u32;
                // Sample the track interior, away from rounded edge antialiasing.
                for pixel in [
                    progress.get_buffer().unwrap().get_pixel(10, y).unwrap(),
                    gpu_commands.buffer().get_pixel(13, y + 5).unwrap(),
                ] {
                    assert_eq!(pixel >> 24 != 0, inside, "height={height}, y={y}");
                }
            }
        }
        assert_eq!(
            progress.layout(LayoutConstraints::loose(80., 100.)).height,
            4.
        );
    }
}
