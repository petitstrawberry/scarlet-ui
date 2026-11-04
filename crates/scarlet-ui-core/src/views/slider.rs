//! Slider View - Slider control for selecting a value from a range
//!
//! Slider is a control that allows selecting a value from a continuous range.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, RenderElement};
use crate::geometry::{EdgeInsets, Offset, Point, Rect, Size};
use crate::os::Mutex;
use crate::renderer::PaintContext;
use crate::state::State;
use crate::view::View;
use crate::views::style;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

static SLIDER_DRAGGING_REGISTRY: Mutex<BTreeMap<crate::state::StateId, State<bool>>> =
    Mutex::new(BTreeMap::new());

fn slider_dragging_state(value_state: &State<f32>) -> State<bool> {
    let id = value_state.id();
    let mut registry = SLIDER_DRAGGING_REGISTRY.lock();
    if let Some(state) = registry.get(&id) {
        return state.clone();
    }
    let state: State<bool> = State::initial(crate::state::generate_state_id());
    registry.insert(id, state.clone());
    state
}

/// Slider View
#[derive(Clone)]
pub struct Slider {
    value: State<f32>,
    min: f32,
    max: f32,
    dragging: State<bool>,
    on_change: Option<Arc<dyn Fn(f32) + 'static>>,
}

impl Slider {
    /// Create a new Slider
    pub fn new(value: State<f32>) -> Self {
        let dragging = slider_dragging_state(&value);
        Self {
            value,
            min: 0.0,
            max: 1.0,
            dragging,
            on_change: None,
        }
    }

    /// Set minimum value
    pub fn min(mut self, min: f32) -> Self {
        self.min = min;
        self
    }

    /// Set maximum value
    pub fn max(mut self, max: f32) -> Self {
        self.max = max;
        self
    }

    /// Set dragging state store
    pub fn dragging_state(mut self, dragging: State<bool>) -> Self {
        self.dragging = dragging;
        self
    }

    /// Set the change callback.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function called with the updated slider value.
    ///
    /// # Returns
    ///
    /// The updated Slider view.
    pub fn on_change(mut self, callback: impl Fn(f32) + 'static) -> Self {
        self.on_change = Some(Arc::new(callback));
        self
    }

    /// Set the change callback.
    ///
    /// This is an alias for [`Slider::on_change`].
    ///
    /// # Arguments
    ///
    /// * `callback` - Function called with the updated slider value.
    ///
    /// # Returns
    ///
    /// The updated Slider view.
    pub fn on_changed(self, callback: impl Fn(f32) + 'static) -> Self {
        self.on_change(callback)
    }

    /// Get value state
    pub fn get_value(&self) -> &State<f32> {
        &self.value
    }

    /// Get min
    pub fn get_min(&self) -> f32 {
        self.min
    }

    /// Get max
    pub fn get_max(&self) -> f32 {
        self.max
    }

    /// Get dragging state
    pub fn get_dragging(&self) -> &State<bool> {
        &self.dragging
    }

    /// Invoke the change callback if present.
    ///
    /// # Arguments
    ///
    /// * `value` - Updated slider value.
    pub fn invoke_on_change(&self, value: f32) {
        if let Some(callback) = self.on_change.as_ref() {
            callback(value);
        }
    }
}

impl View for Slider {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            SliderRenderObject::new(self.value.get(), self.min, self.max, self.dragging.get()),
        ))
    }

    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        alloc::vec![&self.value]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Slider RenderObject
///
/// Compact desktop slider using the shared control and track metrics:
/// - Height: 20px (visible track is 6px thick)
/// - Width: flexible (at least 100px)
/// - Track: semantic section surface
/// - Fill: semantic primary color
/// - Thumb: 20×16px frosted pill with a uniform semantic tint
pub struct SliderRenderObject {
    value: f32,
    min: f32,
    max: f32,
    size: Size,
    buffer: Option<Buffer>,
    dragging: bool,
    mouse_dragging: bool,
    active_touch_id: Option<u64>,
}

impl SliderRenderObject {
    fn thumb_diameter(&self) -> f32 {
        style::metrics().slider_thumb_diameter + 4.0
    }

    fn track_metrics(&self) -> (f32, f32) {
        let thumb_diameter = self.thumb_diameter().max(1.0);
        let track_start = thumb_diameter / 2.0;
        let track_width = (self.size.width - thumb_diameter).max(1.0);
        (track_start, track_width)
    }

    /// Create a new SliderRenderObject
    pub fn new(value: f32, min: f32, max: f32, dragging: bool) -> Self {
        let metrics = style::metrics();
        Self {
            value: value.clamp(min, max),
            min,
            max,
            size: Size::new(200.0, metrics.slider_height),
            buffer: None,
            dragging,
            mouse_dragging: false,
            active_touch_id: None,
        }
    }

    /// Get value
    pub fn get_value(&self) -> f32 {
        self.value
    }

    /// Set value
    pub fn set_value(&mut self, value: f32) {
        self.value = value.clamp(self.min, self.max);
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn set_dragging(&mut self, dragging: bool) {
        self.mouse_dragging = dragging;
        self.dragging = dragging || self.active_touch_id.is_some();
    }

    pub fn is_mouse_dragging(&self) -> bool {
        self.mouse_dragging
    }

    pub fn active_touch_id(&self) -> Option<u64> {
        self.active_touch_id
    }

    pub fn begin_touch_drag(&mut self, id: u64) -> bool {
        if self.active_touch_id.is_some() {
            return false;
        }
        self.active_touch_id = Some(id);
        self.dragging = true;
        true
    }

    pub fn end_touch_drag(&mut self, id: u64) -> bool {
        if self.active_touch_id != Some(id) {
            return false;
        }
        self.active_touch_id = None;
        self.dragging = self.mouse_dragging;
        true
    }

    pub fn value_from_local_x(&self, local_x: f32) -> f32 {
        let (track_start, track_width) = self.track_metrics();
        let normalized = ((local_x - track_start) / track_width).clamp(0.0, 1.0);
        self.min + (self.max - self.min) * normalized
    }

    /// Get min
    pub fn get_min(&self) -> f32 {
        self.min
    }

    /// Get max
    pub fn get_max(&self) -> f32 {
        self.max
    }

    /// Replay the retained drawing for identical geometry and edges at every scale.
    fn draw_slider(&mut self) {
        let outsets = self.paint_outsets();
        let w = libm::ceilf(self.size.width + outsets.left + outsets.right) as u32;
        let h = libm::ceilf(self.size.height + outsets.top + outsets.bottom) as u32;
        let old_buffer = self.buffer.take();
        let scale = old_buffer.as_ref().map(|buffer| buffer.scale_milli());
        let mut buffer = old_buffer
            .filter(|buffer| buffer.logical_width() == w && buffer.logical_height() == h)
            .unwrap_or_else(|| match scale {
                Some(scale) => Buffer::from_logical_dimensions_with_scale(w, h, scale),
                None => Buffer::from_logical_dimensions(w, h),
            });
        let mut ctx = PaintContext::new();
        self.paint(&mut ctx, Point::new(outsets.left, outsets.top));
        #[allow(deprecated)]
        crate::renderer::CpuPaintRenderer::execute_into_buffer(
            &mut buffer,
            Color::CLEAR,
            &ctx,
            None,
        );
        self.buffer = Some(buffer);
    }
}

impl ElementRenderObject for SliderRenderObject {
    fn touch_drag_axis(&self, _local_point: Point) -> Option<crate::event::TouchDragAxis> {
        Some(crate::event::TouchDragAxis::Horizontal)
    }

    fn layout(&mut self, constraints: crate::element::LayoutConstraints) -> Size {
        // Slider has fixed height (20px), flexible width
        let width = if constraints.max_width.is_finite() {
            constraints.max_width.max(constraints.min_width).max(0.0)
        } else {
            constraints.min_width.max(200.0)
        };

        let height = style::metrics().slider_height;

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

    fn reads_backdrop(&self) -> bool {
        true
    }

    fn paint_outsets(&self) -> EdgeInsets {
        // Reserve the maximum held-state extent even after release, so partial
        // repainting clears the old capsule without changing layout or hit testing.
        EdgeInsets::all(10.0)
    }

    fn buffer_origin(&self) -> Point {
        let outsets = self.paint_outsets();
        Point::new(-outsets.left, -outsets.top)
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
        self.draw_slider();
    }

    fn get_buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    fn clear_buffer(&mut self) {
        self.buffer = None;
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let height = libm::ceilf(self.size.height) as u32;
        let (track_start, track_width) = self.track_metrics();
        let center_y = height as f32 / 2.0;
        let track_thickness = style::metrics().slider_track_thickness + 2.0;
        let track_y = center_y - track_thickness / 2.0;
        let range = self.max - self.min;
        let normalized_value = if range > 0.0 {
            (self.value - self.min) / range
        } else {
            0.0
        };
        let fill_width = normalized_value * track_width;
        let palette = ColorPalette::default();
        let track_color = palette
            .text_primary()
            .with_opacity(0.13)
            .blend_over(style::surface_color(&palette, style::SurfaceRole::Section));
        let fill_color = if self.dragging {
            palette.primary().darken(0.035)
        } else {
            palette.primary()
        };
        let thumb_color = style::GlassMaterial::thumb().tint;
        let thumb_color = if self.dragging {
            thumb_color.darken(0.035)
        } else {
            thumb_color
        };
        let thumb_radius = self.thumb_diameter().max(1.0) / 2.0;
        let thumb_center = Point::new(origin.x + track_start + fill_width, origin.y + center_y);

        style::track(
            ctx,
            Rect::from_xywh(
                origin.x + track_start,
                origin.y + track_y,
                track_width,
                track_thickness,
            ),
            track_color,
        );
        if fill_width > 0.0 {
            style::track(
                ctx,
                Rect::from_xywh(
                    origin.x + track_start,
                    origin.y + track_y,
                    fill_width,
                    track_thickness,
                ),
                fill_color,
            );
        }
        let thumb_rect = Rect::from_xywh(
            thumb_center.x - thumb_radius,
            thumb_center.y - 8.0,
            thumb_radius * 2.0,
            16.0,
        );
        if self.dragging {
            let held_rect =
                Rect::from_xywh(thumb_center.x - 12.5, thumb_center.y - 11.0, 25.0, 22.0);
            ctx.draw_rounded_rect_shadow(
                held_rect,
                11.0,
                Offset::new(0.0, 1.0),
                2.0,
                0.0,
                Color::BLACK.with_opacity(0.10),
            );
            style::paint_glass_material(
                ctx,
                held_rect,
                11.0,
                style::GlassMaterial {
                    tint: thumb_color,
                    ..style::GlassMaterial::thumb()
                },
            );
        } else {
            ctx.draw_rounded_rect_shadow(
                thumb_rect,
                8.0,
                Offset::new(0.0, 1.0),
                2.0,
                0.0,
                Color::BLACK.with_opacity(0.10),
            );
            style::paint_glass_material(
                ctx,
                thumb_rect,
                8.0,
                style::GlassMaterial {
                    tint: thumb_color,
                    ..style::GlassMaterial::thumb()
                },
            );
        }
        true
    }

    fn update(&mut self, new_view: &dyn crate::view::View) -> crate::element::UpdateResult {
        if let Some(slider) = new_view.as_any().downcast_ref::<Slider>() {
            let mut changed = false;
            let new_min = slider.min;
            let new_max = slider.max;
            if (self.min - new_min).abs() > 0.001 || (self.max - new_max).abs() > 0.001 {
                self.min = new_min;
                self.max = new_max;
                changed = true;
            }
            let new_value = slider.value.get().clamp(self.min, self.max);
            if (self.value - new_value).abs() > 0.001 {
                self.value = new_value;
                changed = true;
            }
            let dragging =
                slider.dragging.get() || self.mouse_dragging || self.active_touch_id.is_some();
            if self.dragging != dragging {
                self.dragging = dragging;
                changed = true;
            }
            if changed {
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
    use crate::renderer::PaintCommand;

    #[test]
    fn thumb_material_is_uniform_backdrop_only_and_preserves_center() {
        for on in [false, true] {
            let mut control = SliderRenderObject::new(if on { 1.0 } else { 0.0 }, 0.0, 1.0, false);
            let mut idle_center = None;
            for held in [false, true] {
                control.set_dragging(held);
                assert!(control.reads_backdrop());
                let mut ctx = PaintContext::new();
                control.paint(&mut ctx, Point::new(7.0, 13.0));
                let materials: alloc::vec::Vec<_> = ctx
                    .commands()
                    .iter()
                    .filter_map(|cmd| match cmd {
                        PaintCommand::BackdropFilterRoundedRect {
                            rect,
                            corner_radius,
                            blur_radius,
                            tint,
                        } => Some((*rect, *corner_radius, *blur_radius, *tint)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(materials.len(), 1, "only the thumb reads backdrop");
                let (rect, radius, blur, tint) = materials[0];
                assert_eq!(
                    rect.size,
                    Size::new(
                        if held { 25.0 } else { 20.0 },
                        if held { 22.0 } else { 16.0 }
                    )
                );
                assert_eq!(radius, rect.size.height / 2.0);
                assert_eq!(blur, style::GlassMaterial::thumb().blur_radius);
                assert_eq!(tint.a, style::GlassMaterial::thumb().tint.a);
                let center = Point::new(
                    rect.origin.x + rect.size.width / 2.0,
                    rect.origin.y + rect.size.height / 2.0,
                );
                if held {
                    assert_eq!(Some(center), idle_center);
                } else {
                    idle_center = Some(center);
                }
                assert!(!ctx.commands().iter().any(|cmd| matches!(
                    cmd,
                    PaintCommand::FillVerticalGradientRoundedRect { .. }
                        | PaintCommand::StrokeRoundedRect { .. }
                        | PaintCommand::StrokeRect { .. }
                        | PaintCommand::StrokePath { .. }
                )));
            }
        }
    }

    #[test]
    fn narrow_sliders_respect_allocated_width_and_keep_their_value() {
        let mut slider = SliderRenderObject::new(0.5, 0.0, 1.0, false);
        for width in [200.0, 72.0, 24.0, 0.0, 200.0] {
            let size = slider.layout(LayoutConstraints::tight(
                width,
                style::metrics().slider_height,
            ));
            assert_eq!(size.width, width);
            assert_eq!(slider.value, 0.5);
            if width >= 24.0 {
                let (start, track_width) = slider.track_metrics();
                assert!(start >= 0.0 && start + track_width <= width);
            }
        }
    }

    #[test]
    fn glass_thumb_uses_a_soft_shadow_without_highlight_lines() {
        let slider = SliderRenderObject::new(0.5, 0.0, 1.0, false);
        let mut ctx = PaintContext::new();
        slider.paint(&mut ctx, Point::ZERO);
        let thumb = ctx
            .commands()
            .iter()
            .rev()
            .find_map(|cmd| match cmd {
                PaintCommand::BackdropFilterRoundedRect { rect, .. } => Some(rect),
                _ => None,
            })
            .unwrap();
        assert_eq!(thumb.size, Size::new(20.0, 16.0));
        assert!(!ctx.commands().iter().any(|cmd| matches!(
            cmd,
            PaintCommand::FillVerticalGradientRoundedRect { .. } | PaintCommand::PushClip { .. }
        )));
        assert!(
            ctx.commands()
                .iter()
                .any(|cmd| matches!(cmd, PaintCommand::DrawRoundedRectShadow { .. }))
        );
    }

    #[test]
    fn dragging_changes_material_without_moving_the_value_or_thumb() {
        let mut slider = SliderRenderObject::new(0.5, 0.0, 1.0, false);
        let normal_metrics = slider.track_metrics();
        let fill = |ctx: &PaintContext<'_>| {
            ctx.commands()
                .iter()
                .rev()
                .find_map(|cmd| match cmd {
                    PaintCommand::BackdropFilterRoundedRect { tint, .. } => Some(*tint),
                    _ => None,
                })
                .unwrap()
        };
        let normal_color = {
            let mut normal = PaintContext::new();
            slider.paint(&mut normal, Point::ZERO);
            fill(&normal)
        };
        slider.set_dragging(true);
        let mut dragging = PaintContext::new();
        slider.paint(&mut dragging, Point::ZERO);
        assert_eq!(slider.track_metrics(), normal_metrics);
        assert_eq!(slider.get_value(), 0.5);
        assert_ne!(normal_color, fill(&dragging));
    }
    #[test]
    fn glass_thumb_endpoints_and_pointer_mapping_agree() {
        for value in [0.0, 1.0] {
            let slider = SliderRenderObject::new(value, 0.0, 1.0, false);
            let (start, width) = slider.track_metrics();
            assert_eq!(slider.value_from_local_x(start + value * width), value);
            let mut ctx = PaintContext::new();
            slider.paint(&mut ctx, Point::ZERO);
            let rect = ctx
                .commands()
                .iter()
                .rev()
                .find_map(|cmd| match cmd {
                    PaintCommand::BackdropFilterRoundedRect { rect, .. } => Some(*rect),
                    _ => None,
                })
                .unwrap();
            assert!(rect.origin.x >= 0.0);
            assert!(rect.origin.x + rect.size.width <= slider.size.width);
        }
    }

    #[test]
    fn legacy_buffer_matches_retained_material_at_1x_and_2x() {
        for scale in [1000, 2000] {
            for held in [false, true] {
                let mut control = SliderRenderObject::new(0.5, 0.0, 1.0, false);
                control.set_dragging(held);
                control.buffer = Some(Buffer::from_logical_dimensions_with_scale(
                    control.size.width as u32,
                    control.size.height as u32,
                    scale,
                ));
                control.draw_slider();
                let mut ctx = PaintContext::new();
                let outsets = control.paint_outsets();
                control.paint(&mut ctx, Point::new(outsets.left, outsets.top));
                let buffer_size = Size::new(
                    control.size.width + outsets.left + outsets.right,
                    control.size.height + outsets.top + outsets.bottom,
                );
                let mut renderer =
                    crate::renderer::CpuPaintRenderer::new(buffer_size, scale, Color::CLEAR);
                renderer.execute(&ctx);
                let buffer = control.get_buffer().unwrap();
                assert_eq!(buffer.scale_milli(), scale);
                assert_eq!(buffer.data(), renderer.buffer().data());
                assert!(buffer.data().iter().any(|&byte| byte != 0));
            }
        }
    }
    fn held_capsule(control: &SliderRenderObject) -> Option<(Rect, Color)> {
        let mut ctx = PaintContext::new();
        control.paint(&mut ctx, Point::ZERO);
        ctx.commands().iter().find_map(|cmd| match cmd {
            PaintCommand::BackdropFilterRoundedRect { rect, tint, .. }
                if rect.size.height == 22.0 =>
            {
                Some((*rect, *tint))
            }
            _ => None,
        })
    }

    #[test]
    fn held_capsule_expands_transparently_and_release_restores_idle_glass() {
        let mut control = SliderRenderObject::new(0.5, 0.0, 1.0, false);
        let size = control.size();
        let outsets = control.paint_outsets();
        assert!(held_capsule(&control).is_none());
        control.set_dragging(true);
        let (rect, tint) = held_capsule(&control).unwrap();
        assert_eq!(rect.size, Size::new(25.0, 22.0));
        assert!(tint.a > 0.0 && tint.a < 1.0);
        assert!(rect.origin.x >= -outsets.left);
        assert!(rect.origin.y >= -outsets.top);
        assert!(rect.origin.x + rect.size.width <= size.width + outsets.right);
        assert!(rect.origin.y + rect.size.height <= size.height + outsets.bottom);
        assert_eq!(control.size(), size);
        control.set_dragging(false);
        assert!(held_capsule(&control).is_none());
        assert_eq!(control.paint_outsets(), outsets);
        assert!(control.begin_touch_drag(7));
        assert!(held_capsule(&control).is_some());
        assert!(control.end_touch_drag(7));
        assert!(held_capsule(&control).is_none());
    }

    #[test]
    fn held_endpoints_keep_the_idle_value_mapping() {
        for value in [0.0, 1.0] {
            let mut slider = SliderRenderObject::new(value, 0.0, 1.0, false);
            let metrics = slider.track_metrics();
            slider.set_dragging(true);
            assert_eq!(slider.track_metrics(), metrics);
            assert_eq!(
                slider.value_from_local_x(metrics.0 + value * metrics.1),
                value
            );
            let (rect, _) = held_capsule(&slider).unwrap();
            assert_eq!(
                rect.origin.x + rect.size.width / 2.0,
                metrics.0 + value * metrics.1
            );
            let outsets = slider.paint_outsets();
            assert!(rect.origin.x >= -outsets.left);
            assert!(rect.origin.x + rect.size.width <= slider.size.width + outsets.right);
        }
    }
}
