//! Shared visual roles for ScarletUI's built-in widgets.
//!
//! Geometry is role-based: structural layout stays square, controls use a
//! compact radius, floating surfaces use a slightly larger radius, and tracks
//! use a capsule. Palette values and widget layout remain owned by the widget.

use crate::color::Color;
use crate::geometry::Rect;
use crate::renderer::PaintContext;

/// Desktop visual metrics shared by built-in widgets.
///
/// These are deliberately private. Future input adaptation must be supplied
/// through the live view environment instead of a process-wide global mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VisualMetrics {
    pub(crate) control_radius: f32,
    pub(crate) item_radius: f32,
    pub(crate) popover_radius: f32,
    pub(crate) border_width: f32,
    pub(crate) focus_stroke_width: f32,
    pub(crate) minimum_control_height: f32,
    pub(crate) navigation_indicator_width: f32,
    pub(crate) navigation_item_height: f32,
    pub(crate) tab_indicator_height: f32,
    pub(crate) tab_bar_height: f32,
    pub(crate) scrollbar_thickness: f32,
    pub(crate) scrollbar_inset: f32,
    pub(crate) scrollbar_min_thumb_length: f32,
    pub(crate) slider_height: f32,
    pub(crate) slider_thumb_diameter: f32,
    pub(crate) slider_track_thickness: f32,
    pub(crate) chrome_title_font_size: f32,
}

const VISUAL_METRICS: VisualMetrics = VisualMetrics {
    control_radius: 6.0,
    item_radius: 4.0,
    popover_radius: 8.0,
    border_width: 1.0,
    focus_stroke_width: 1.5,
    minimum_control_height: 24.0,
    navigation_indicator_width: 3.0,
    navigation_item_height: 40.0,
    tab_indicator_height: 2.0,
    tab_bar_height: 30.0,
    scrollbar_thickness: 6.0,
    scrollbar_inset: 3.0,
    scrollbar_min_thumb_length: 24.0,
    slider_height: 20.0,
    slider_thumb_diameter: 20.0,
    slider_track_thickness: 4.0,
    chrome_title_font_size: 14.0,
};

pub(crate) const fn metrics() -> VisualMetrics {
    VISUAL_METRICS
}

pub(crate) fn radius_for(rect: Rect, radius: f32) -> f32 {
    radius
        .max(0.0)
        .min(rect.size.width.max(0.0) * 0.5)
        .min(rect.size.height.max(0.0) * 0.5)
}

pub(crate) fn fill_control(ctx: &mut PaintContext<'_>, rect: Rect, color: Color) {
    ctx.fill_rounded_rect(rect, radius_for(rect, metrics().control_radius), color);
}

pub(crate) fn stroke_control(ctx: &mut PaintContext<'_>, rect: Rect, width: f32, color: Color) {
    ctx.stroke_rounded_rect(
        rect,
        radius_for(rect, metrics().control_radius),
        width,
        color,
    );
}

pub(crate) fn control_surface(ctx: &mut PaintContext<'_>, rect: Rect, fill: Color, border: Color) {
    fill_control(ctx, rect, fill);
    stroke_control(ctx, rect, metrics().border_width, border);
}

pub(crate) fn popover_surface(ctx: &mut PaintContext<'_>, rect: Rect, fill: Color, border: Color) {
    let metrics = metrics();
    let radius = radius_for(rect, metrics.popover_radius);
    ctx.fill_rounded_rect(rect, radius, fill);
    ctx.stroke_rounded_rect(rect, radius, metrics.border_width, border);
}

pub(crate) fn item_highlight(ctx: &mut PaintContext<'_>, rect: Rect, color: Color) {
    ctx.fill_rounded_rect(rect, radius_for(rect, metrics().item_radius), color);
}

pub(crate) fn track(ctx: &mut PaintContext<'_>, rect: Rect, color: Color) {
    ctx.fill_rounded_rect(
        rect,
        radius_for(rect, rect.size.height.max(0.0) * 0.5),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::renderer::PaintCommand;

    #[test]
    fn control_surface_uses_shared_radius_and_hairline() {
        let rect = Rect::from_xywh(0.0, 0.0, 80.0, 28.0);
        let mut ctx = PaintContext::new();
        control_surface(&mut ctx, rect, Color::WHITE, Color::BLACK);
        let expected = metrics();

        let [
            PaintCommand::FillRoundedRect { corner_radius, .. },
            PaintCommand::StrokeRoundedRect {
                corner_radius: stroke_radius,
                stroke_width,
                ..
            },
        ] = ctx.commands()
        else {
            panic!("expected rounded fill and stroke");
        };
        assert_eq!(*corner_radius, expected.control_radius);
        assert_eq!(*stroke_radius, expected.control_radius);
        assert_eq!(*stroke_width, expected.border_width);
    }

    #[test]
    fn track_is_a_capsule_even_when_short() {
        let rect = Rect::from_xywh(0.0, 0.0, 100.0, 4.0);
        let mut ctx = PaintContext::new();
        track(&mut ctx, rect, Color::BLACK);

        let [PaintCommand::FillRoundedRect { corner_radius, .. }] = ctx.commands() else {
            panic!("expected rounded track fill");
        };
        assert_eq!(*corner_radius, 2.0);
    }
}
