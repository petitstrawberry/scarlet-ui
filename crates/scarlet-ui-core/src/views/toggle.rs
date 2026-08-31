//! Toggle View - On/off switch control
//!
//! Toggle is a switch control that can be on or off.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, RenderElement};
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::renderer::PaintContext;
use crate::state::State;
use crate::view::View;
use crate::views::style::{self, SurfaceRole};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;

/// Toggle View - on/off switch
#[derive(Clone)]
pub struct Toggle {
    is_on: State<bool>,
}

impl Toggle {
    /// Create a new Toggle
    pub fn new(is_on: State<bool>) -> Self {
        Self { is_on }
    }

    /// Get the is_on state
    pub fn get_is_on(&self) -> &State<bool> {
        &self.is_on
    }
}

impl View for Toggle {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            ToggleRenderObject::new(self.is_on.get()),
        ))
    }

    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        alloc::vec![&self.is_on]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Toggle RenderObject
///
/// A platform-neutral switch using capsule geometry and the semantic palette.
pub struct ToggleRenderObject {
    is_on: bool,
    hovered: bool,
    pressed: bool,
    size: Size,
    buffer: Option<Buffer>,
}

impl ToggleRenderObject {
    fn colors(&self, palette: &ColorPalette) -> (Color, Color, Color, Color) {
        let base_track = if self.is_on {
            palette.primary()
        } else {
            style::surface_color(palette, SurfaceRole::Section)
        };
        let track = if self.pressed {
            base_track.darken(0.035)
        } else if self.hovered {
            base_track.lighten(0.018)
        } else {
            base_track
        };
        let track_border = if self.is_on {
            palette.primary().darken(0.06)
        } else {
            palette.divider()
        };
        let thumb = style::surface_color(palette, SurfaceRole::Floating);
        let thumb_border = palette.divider();
        (track, track_border, thumb, thumb_border)
    }

    fn fill_rounded_rect(
        canvas: &mut graphics::Canvas<'_>,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        radius: u32,
        color: Color,
    ) {
        let r = radius as i32;
        if r <= 0 {
            canvas.fill_rect(x, y, width, height, color);
            return;
        }

        let w = width as i32;
        let h = height as i32;
        let r_max = (width.min(height) / 2) as i32;
        let r = r.min(r_max);
        let r_sq = (r - 1).max(0);
        let r_sq = r_sq * r_sq;

        for py in 0..h {
            for px in 0..w {
                let mut inside = true;

                if px < r && py < r {
                    let dx = px - (r - 1);
                    let dy = py - (r - 1);
                    inside = dx * dx + dy * dy <= r_sq;
                } else if px >= w - r && py < r {
                    let dx = px - (w - r);
                    let dy = py - (r - 1);
                    inside = dx * dx + dy * dy <= r_sq;
                } else if px < r && py >= h - r {
                    let dx = px - (r - 1);
                    let dy = py - (h - r);
                    inside = dx * dx + dy * dy <= r_sq;
                } else if px >= w - r && py >= h - r {
                    let dx = px - (w - r);
                    let dy = py - (h - r);
                    inside = dx * dx + dy * dy <= r_sq;
                }

                if inside {
                    canvas.put_pixel(x + px, y + py, color);
                }
            }
        }
    }

    fn fill_circle(
        canvas: &mut graphics::Canvas<'_>,
        center_x: i32,
        center_y: i32,
        radius: i32,
        color: Color,
    ) {
        if radius <= 0 {
            return;
        }
        let r_sq = radius * radius;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= r_sq {
                    canvas.put_pixel(center_x + dx, center_y + dy, color);
                }
            }
        }
    }

    fn downsample_2x(
        src: &[u8],
        src_width: u32,
        src_height: u32,
        dst: &mut [u8],
        dst_width: u32,
        dst_height: u32,
    ) {
        if src_width != dst_width * 2 || src_height != dst_height * 2 {
            return;
        }

        for y in 0..dst_height {
            for x in 0..dst_width {
                let sx = x * 2;
                let sy = y * 2;
                let mut sum = [0u32; 4];

                for oy in 0..2 {
                    for ox in 0..2 {
                        let idx = ((sy + oy) * src_width + (sx + ox)) as usize * 4;
                        sum[0] += src[idx] as u32;
                        sum[1] += src[idx + 1] as u32;
                        sum[2] += src[idx + 2] as u32;
                        sum[3] += src[idx + 3] as u32;
                    }
                }

                let dst_idx = (y * dst_width + x) as usize * 4;
                dst[dst_idx] = (sum[0] / 4) as u8;
                dst[dst_idx + 1] = (sum[1] / 4) as u8;
                dst[dst_idx + 2] = (sum[2] / 4) as u8;
                dst[dst_idx + 3] = (sum[3] / 4) as u8;
            }
        }
    }

    fn blend_bgra_over(dst: &mut [u8], src: &[u8], width: u32, height: u32) {
        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                let bgra = u32::from_le_bytes([src[idx], src[idx + 1], src[idx + 2], src[idx + 3]]);
                let color = Color::from_bgra(bgra);
                if color.a <= 0.0 {
                    continue;
                }
                let dst_bgra =
                    u32::from_le_bytes([dst[idx], dst[idx + 1], dst[idx + 2], dst[idx + 3]]);
                let dst_color = Color::from_bgra(dst_bgra);
                let out = color.blend_over(dst_color);
                let out_bytes = out.to_bgra().to_le_bytes();
                dst[idx..idx + 4].copy_from_slice(&out_bytes);
            }
        }
    }

    /// Create a new ToggleRenderObject
    pub fn new(is_on: bool) -> Self {
        let metrics = style::metrics();

        Self {
            is_on,
            hovered: false,
            pressed: false,
            size: Size::new(metrics.toggle_width, metrics.toggle_height),
            buffer: None,
        }
    }

    /// Get is_on state
    pub fn get_is_on(&self) -> bool {
        self.is_on
    }

    /// Set is_on state
    pub fn set_is_on(&mut self, is_on: bool) {
        self.is_on = is_on;
    }

    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered;
    }

    pub fn set_pressed(&mut self, pressed: bool) {
        self.pressed = pressed;
    }

    /// Draw toggle using Canvas API.
    fn draw_toggle(&mut self) {
        let width = libm::ceilf(self.size.width) as usize;
        let height = libm::ceilf(self.size.height) as usize;
        let w = width as u32;
        let h = height as u32;

        // Create or resize buffer
        let needs_resize = self
            .buffer
            .as_ref()
            .map_or(true, |b| b.logical_width() != w || b.logical_height() != h);
        if needs_resize {
            self.buffer = Some(Buffer::from_logical_dimensions(w, h));
        }

        let palette = ColorPalette::default();
        let (bg_color, border_color, thumb_color, thumb_border) = self.colors(&palette);
        if let Some(ref mut buffer) = self.buffer {
            let physical_w = buffer.width();
            let physical_h = buffer.height();
            let ui_scale = (buffer.scale_milli() as f32) / 1000.0;
            let metrics = style::metrics();

            let data = buffer.data_mut();
            data.fill(0);

            let aa_scale = 2u32;
            let w_hi = physical_w * aa_scale;
            let h_hi = physical_h * aa_scale;
            let mut track_hi = alloc::vec![0u8; (w_hi * h_hi * 4) as usize];
            let mut canvas_hi = graphics::Canvas::new(&mut track_hi, w_hi, h_hi);
            let radius_hi = (h_hi / 2).max(1);
            Self::fill_rounded_rect(&mut canvas_hi, 0, 0, w_hi, h_hi, radius_hi, border_color);

            let inset = aa_scale;
            let inner_w = w_hi.saturating_sub(inset * 2);
            let inner_h = h_hi.saturating_sub(inset * 2);
            if inner_w > 0 && inner_h > 0 {
                let radius_inner = radius_hi.saturating_sub(inset);
                Self::fill_rounded_rect(
                    &mut canvas_hi,
                    inset as i32,
                    inset as i32,
                    inner_w,
                    inner_h,
                    radius_inner,
                    bg_color,
                );
            }

            let mut track = alloc::vec![0u8; (physical_w * physical_h * 4) as usize];
            Self::downsample_2x(&track_hi, w_hi, h_hi, &mut track, physical_w, physical_h);
            data.copy_from_slice(&track);

            // Thumb position: on = right side, off = left side
            let thumb_diameter = metrics.toggle_thumb_diameter;
            let thumb_x = if self.is_on {
                self.size.width - metrics.toggle_thumb_inset - thumb_diameter
            } else {
                metrics.toggle_thumb_inset
            };
            let thumb_x = (thumb_x * ui_scale * aa_scale as f32) as i32;
            let thumb_y = (metrics.toggle_thumb_inset * ui_scale * aa_scale as f32) as i32;
            let thumb_size = libm::ceilf(thumb_diameter * ui_scale * aa_scale as f32) as i32;
            let radius = (thumb_size / 2).max(1);
            let center_x = thumb_x + radius;
            let center_y = thumb_y + radius;
            let mut thumb_hi = alloc::vec![0u8; (w_hi * h_hi * 4) as usize];
            let mut thumb_canvas = graphics::Canvas::new(&mut thumb_hi, w_hi, h_hi);
            Self::fill_circle(&mut thumb_canvas, center_x, center_y, radius, thumb_border);
            Self::fill_circle(
                &mut thumb_canvas,
                center_x,
                center_y,
                (radius - aa_scale as i32).max(0),
                thumb_color,
            );

            let mut thumb = alloc::vec![0u8; (physical_w * physical_h * 4) as usize];
            Self::downsample_2x(&thumb_hi, w_hi, h_hi, &mut thumb, physical_w, physical_h);
            Self::blend_bgra_over(data, &thumb, physical_w, physical_h);
        }
    }
}

impl ElementRenderObject for ToggleRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        // The visible switch geometry is independent from any future expanded
        // touch hit target.
        let width = self.size.width;
        let height = self.size.height;

        self.size = Size { width, height };

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
        self.draw_toggle();
    }

    fn get_buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    fn clear_buffer(&mut self) {
        self.buffer = None;
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let palette = ColorPalette::default();
        let (bg_color, border_color, thumb_color, thumb_border) = self.colors(&palette);
        let metrics = style::metrics();
        let rect = Rect::new(origin, self.size);
        let border_width = metrics.border_width;
        style::track(ctx, rect, border_color);
        style::track(
            ctx,
            Rect::from_xywh(
                origin.x + border_width,
                origin.y + border_width,
                (self.size.width - border_width * 2.0).max(0.0),
                (self.size.height - border_width * 2.0).max(0.0),
            ),
            bg_color,
        );

        let thumb_diameter = metrics.toggle_thumb_diameter;
        let thumb_x = if self.is_on {
            self.size.width - metrics.toggle_thumb_inset - thumb_diameter
        } else {
            metrics.toggle_thumb_inset
        };
        let thumb_radius = thumb_diameter / 2.0;
        style::control_thumb(
            ctx,
            Point::new(
                origin.x + thumb_x + thumb_radius,
                origin.y + metrics.toggle_thumb_inset + thumb_radius,
            ),
            thumb_radius,
            thumb_color,
            thumb_border,
        );
        true
    }

    fn update(&mut self, new_view: &dyn crate::view::View) -> crate::element::UpdateResult {
        if let Some(toggle) = new_view.as_any().downcast_ref::<Toggle>() {
            let new_is_on = toggle.is_on.get();
            if self.is_on != new_is_on {
                self.is_on = new_is_on;
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
    use crate::renderer::PaintCommand;

    #[test]
    fn toggle_uses_compact_desktop_metrics() {
        let toggle = ToggleRenderObject::new(false);
        let metrics = style::metrics();

        assert_eq!(
            toggle.size(),
            Size::new(metrics.toggle_width, metrics.toggle_height)
        );
        assert_eq!(toggle.size(), Size::new(36.0, 20.0));
    }

    #[test]
    fn enabled_toggle_uses_the_primary_role_instead_of_an_independent_green() {
        let toggle = ToggleRenderObject::new(true);
        let mut ctx = PaintContext::new();
        toggle.paint(&mut ctx, Point::ZERO);

        let PaintCommand::FillRoundedRect { color, .. } = &ctx.commands()[1] else {
            panic!("expected inner toggle track");
        };
        assert_eq!(*color, ColorPalette::default().primary());
    }

    #[test]
    fn inactive_toggle_uses_the_section_surface_role() {
        let toggle = ToggleRenderObject::new(false);
        let mut ctx = PaintContext::new();
        toggle.paint(&mut ctx, Point::ZERO);

        let PaintCommand::FillRoundedRect { color, .. } = &ctx.commands()[1] else {
            panic!("expected inner toggle track");
        };
        let palette = ColorPalette::default();
        assert_eq!(*color, style::surface_color(&palette, SurfaceRole::Section));
    }

    #[test]
    fn pressed_toggle_changes_tone_without_changing_geometry() {
        let mut toggle = ToggleRenderObject::new(true);
        let normal_size = toggle.size();
        let normal = toggle.colors(&ColorPalette::default()).0;
        toggle.set_pressed(true);
        let pressed = toggle.colors(&ColorPalette::default()).0;

        assert_eq!(toggle.size(), normal_size);
        assert!(pressed.r < normal.r);
    }

    #[test]
    fn toggle_hairlines_are_opaque() {
        let toggle = ToggleRenderObject::new(false);
        let (_, track_border, _, thumb_border) = toggle.colors(&ColorPalette::default());

        assert_eq!(track_border.a, 1.0);
        assert_eq!(thumb_border.a, 1.0);
    }
}
