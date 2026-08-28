//! Shared visual roles for ScarletUI's built-in widgets.
//!
//! Geometry is role-based: structural layout stays square, controls use a
//! compact radius, floating surfaces use a slightly larger radius, and tracks
//! use a capsule. Palette values and widget layout remain owned by the widget.

use crate::color::{Color, ColorPalette};
use crate::geometry::{EdgeInsets, Offset, Rect};
use crate::renderer::PaintContext;

const RAISED_TOP_LIGHTEN: f32 = 0.018;
const RAISED_BOTTOM_DARKEN: f32 = 0.010;
const PRESSED_TOP_DARKEN: f32 = 0.010;
const PRESSED_BOTTOM_LIGHTEN: f32 = 0.004;
const CHROME_TOP_LIGHTEN: f32 = 0.012;
const CHROME_BOTTOM_DARKEN: f32 = 0.008;
const TEXT_SELECTION_MIX: f32 = 0.24;
const TEXT_SELECTION_TINT_LIFT: f32 = 0.12;
const SELECTED_ITEM_MIX: f32 = 0.14;
const SELECTED_ITEM_TINT_LIFT: f32 = 0.08;

const FLOATING_AMBIENT_OFFSET: Offset = Offset::new(0.0, 1.0);
const FLOATING_AMBIENT_BLUR: f32 = 3.0;
const FLOATING_AMBIENT_OPACITY: f32 = 0.55;
const FLOATING_KEY_OFFSET: Offset = Offset::new(0.0, 4.0);
const FLOATING_KEY_BLUR: f32 = 10.0;
const FLOATING_KEY_OPACITY: f32 = 0.75;
const FLOATING_OUTSETS: EdgeInsets = EdgeInsets::new(10.0, 6.0, 10.0, 14.0);

const RAISED_SHADOW_OFFSET: Offset = Offset::new(0.0, 1.0);
const RAISED_SHADOW_BLUR: f32 = 4.0;
const RAISED_SHADOW_OPACITY: f32 = 0.40;
const RAISED_OUTSETS: EdgeInsets = EdgeInsets::new(4.0, 3.0, 4.0, 5.0);

const OVERLAY_AMBIENT_OFFSET: Offset = Offset::new(0.0, 2.0);
const OVERLAY_AMBIENT_BLUR: f32 = 8.0;
const OVERLAY_AMBIENT_OPACITY: f32 = 0.65;
const OVERLAY_KEY_OFFSET: Offset = Offset::new(0.0, 12.0);
const OVERLAY_KEY_BLUR: f32 = 24.0;
const OVERLAY_KEY_OPACITY: f32 = 0.85;
const OVERLAY_OUTSETS: EdgeInsets = EdgeInsets::new(24.0, 12.0, 24.0, 36.0);

/// Desktop visual metrics shared by built-in widgets.
///
/// These are deliberately private. Future input adaptation must be supplied
/// through the live view environment instead of a process-wide global mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VisualMetrics {
    pub(crate) window_radius: f32,
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
    pub(crate) toggle_width: f32,
    pub(crate) toggle_height: f32,
    pub(crate) toggle_thumb_diameter: f32,
    pub(crate) toggle_thumb_inset: f32,
    pub(crate) chrome_title_font_size: f32,
}

const VISUAL_METRICS: VisualMetrics = VisualMetrics {
    window_radius: 10.0,
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
    slider_thumb_diameter: 16.0,
    slider_track_thickness: 4.0,
    toggle_width: 36.0,
    toggle_height: 20.0,
    toggle_thumb_diameter: 16.0,
    toggle_thumb_inset: 2.0,
    chrome_title_font_size: 14.0,
};

/// Tonal surface hierarchy shared by built-in widgets and containers.
///
/// The hierarchy describes containment, not interaction state or elevation.
/// Raised and floating effects are applied separately so selected navigation
/// rows and tabs never accidentally acquire depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceRole {
    /// The application's base content plane.
    Canvas,
    /// Persistent structural chrome such as sidebars and tab bars.
    Structural,
    /// A grouped region within the current content plane.
    Section,
    /// Transient content such as menus and popovers.
    Floating,
    /// Modal content such as dialogs and sheets.
    Overlay,
}

/// Visual elevation independent from a surface's tonal role.
///
/// `Flat` is the default for structural and section surfaces. Applications
/// opt into depth only where the interaction model requires it: raised cards,
/// transient floating content, or modal overlays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ElevationRole {
    /// No shadow. Used by canvas, structural, and ordinary section surfaces.
    #[default]
    Flat,
    /// A card-like surface resting just above its parent plane.
    Raised,
    /// Menus, popovers, and expanded selects.
    Floating,
    /// Dialogs and modal sheets.
    Overlay,
}

pub(crate) const fn metrics() -> VisualMetrics {
    VISUAL_METRICS
}

pub(crate) fn surface_color(palette: &ColorPalette, role: SurfaceRole) -> Color {
    match role {
        SurfaceRole::Canvas => palette.background(),
        SurfaceRole::Structural => palette.background_secondary(),
        SurfaceRole::Section => palette.background_tertiary(),
        SurfaceRole::Floating | SurfaceRole::Overlay => palette.surface(),
    }
}

pub(crate) fn surface_radius(role: SurfaceRole) -> f32 {
    match role {
        SurfaceRole::Canvas | SurfaceRole::Structural => 0.0,
        SurfaceRole::Section | SurfaceRole::Floating => metrics().popover_radius,
        SurfaceRole::Overlay => 10.0,
    }
}

pub(crate) fn focus_highlight(palette: &ColorPalette) -> Color {
    palette.primary_light().lighten(0.4)
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

pub(crate) fn raised_control_surface(
    ctx: &mut PaintContext<'_>,
    rect: Rect,
    fill: Color,
    border: Color,
    pressed: bool,
) {
    let radius = radius_for(rect, metrics().control_radius);
    fill_raised_surface(ctx, rect, radius, fill, pressed);
    if border.a > 0.0 {
        ctx.stroke_rounded_rect(rect, radius, metrics().border_width, border);
    }
}

pub(crate) fn fill_raised_surface(
    ctx: &mut PaintContext<'_>,
    rect: Rect,
    radius: f32,
    fill: Color,
    pressed: bool,
) {
    let (top, bottom) = if pressed {
        (
            fill.darken(PRESSED_TOP_DARKEN),
            fill.lighten(PRESSED_BOTTOM_LIGHTEN),
        )
    } else {
        (
            fill.lighten(RAISED_TOP_LIGHTEN),
            fill.darken(RAISED_BOTTOM_DARKEN),
        )
    };
    ctx.fill_vertical_gradient_rounded_rect(rect, radius_for(rect, radius), top, bottom);
}

pub(crate) fn chrome_surface(ctx: &mut PaintContext<'_>, rect: Rect, fill: Color) {
    ctx.fill_vertical_gradient_rounded_rect(
        rect,
        0.0,
        fill.lighten(CHROME_TOP_LIGHTEN),
        fill.darken(CHROME_BOTTOM_DARKEN),
    );
}

pub(crate) fn text_selection_highlight(accent: Color, surface: Color) -> Color {
    accent
        .lighten(TEXT_SELECTION_TINT_LIFT)
        .with_opacity(TEXT_SELECTION_MIX)
        .blend_over(surface)
}

pub(crate) fn selected_item_surface(accent: Color, surface: Color) -> Color {
    accent
        .lighten(SELECTED_ITEM_TINT_LIFT)
        .with_opacity(SELECTED_ITEM_MIX)
        .blend_over(surface)
}

pub(crate) const fn elevation_outsets(elevation: ElevationRole) -> EdgeInsets {
    match elevation {
        ElevationRole::Flat => EdgeInsets::ZERO,
        ElevationRole::Raised => RAISED_OUTSETS,
        ElevationRole::Floating => FLOATING_OUTSETS,
        ElevationRole::Overlay => OVERLAY_OUTSETS,
    }
}

pub(crate) fn elevation_shadow(
    ctx: &mut PaintContext<'_>,
    rect: Rect,
    radius: f32,
    shadow: Color,
    elevation: ElevationRole,
) {
    let radius = radius_for(rect, radius);
    match elevation {
        ElevationRole::Flat => {}
        ElevationRole::Raised => {
            ctx.draw_rounded_rect_shadow(
                rect,
                radius,
                RAISED_SHADOW_OFFSET,
                RAISED_SHADOW_BLUR,
                0.0,
                shadow.with_opacity(shadow.a * RAISED_SHADOW_OPACITY),
            );
        }
        ElevationRole::Floating => {
            ctx.draw_rounded_rect_shadow(
                rect,
                radius,
                FLOATING_AMBIENT_OFFSET,
                FLOATING_AMBIENT_BLUR,
                0.0,
                shadow.with_opacity(shadow.a * FLOATING_AMBIENT_OPACITY),
            );
            ctx.draw_rounded_rect_shadow(
                rect,
                radius,
                FLOATING_KEY_OFFSET,
                FLOATING_KEY_BLUR,
                0.0,
                shadow.with_opacity(shadow.a * FLOATING_KEY_OPACITY),
            );
        }
        ElevationRole::Overlay => {
            ctx.draw_rounded_rect_shadow(
                rect,
                radius,
                OVERLAY_AMBIENT_OFFSET,
                OVERLAY_AMBIENT_BLUR,
                0.0,
                shadow.with_opacity(shadow.a * OVERLAY_AMBIENT_OPACITY),
            );
            ctx.draw_rounded_rect_shadow(
                rect,
                radius,
                OVERLAY_KEY_OFFSET,
                OVERLAY_KEY_BLUR,
                0.0,
                shadow.with_opacity(shadow.a * OVERLAY_KEY_OPACITY),
            );
        }
    }
}

pub(crate) fn floating_shadow(ctx: &mut PaintContext<'_>, rect: Rect, radius: f32, shadow: Color) {
    elevation_shadow(ctx, rect, radius, shadow, ElevationRole::Floating);
}

pub(crate) fn popover_surface(
    ctx: &mut PaintContext<'_>,
    rect: Rect,
    fill: Color,
    border: Color,
    shadow: Color,
) {
    let metrics = metrics();
    let radius = radius_for(rect, metrics.popover_radius);
    elevation_shadow(ctx, rect, radius, shadow, ElevationRole::Floating);
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

pub(crate) fn control_thumb(
    ctx: &mut PaintContext<'_>,
    center: crate::geometry::Point,
    radius: f32,
    fill: Color,
    border: Color,
) {
    let radius = radius.max(0.0);
    if border.a > 0.0 {
        ctx.fill_circle(center, radius, border);
    }
    ctx.fill_circle(center, (radius - metrics().border_width).max(0.0), fill);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::renderer::PaintCommand;

    #[test]
    fn inline_control_primitives_use_shared_radius_and_hairline() {
        let rect = Rect::from_xywh(0.0, 0.0, 80.0, 28.0);
        let mut ctx = PaintContext::new();
        fill_control(&mut ctx, rect, Color::WHITE);
        stroke_control(&mut ctx, rect, metrics().border_width, Color::BLACK);
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

    #[test]
    fn raised_surface_uses_subtle_gradient_without_a_shadow() {
        let rect = Rect::from_xywh(0.0, 0.0, 100.0, 32.0);
        let fill = Color::gray(0.9);
        let mut ctx = PaintContext::new();
        raised_control_surface(&mut ctx, rect, fill, Color::BLACK, false);

        let [
            PaintCommand::FillVerticalGradientRoundedRect {
                top_color,
                bottom_color,
                ..
            },
            PaintCommand::StrokeRoundedRect { .. },
        ] = ctx.commands()
        else {
            panic!("expected gradient and hairline for raised control");
        };
        assert!(top_color.r > fill.r);
        assert!(bottom_color.r < fill.r);
    }

    #[test]
    fn floating_surface_uses_shared_shadow_stack_and_outsets() {
        let rect = Rect::from_xywh(20.0, 20.0, 180.0, 120.0);
        let mut ctx = PaintContext::new();
        popover_surface(
            &mut ctx,
            rect,
            Color::WHITE,
            Color::gray(0.8),
            Color::rgba_f32(0.0, 0.0, 0.0, 0.1),
        );

        assert!(matches!(
            ctx.commands()[0],
            PaintCommand::DrawRoundedRectShadow { .. }
        ));
        assert!(matches!(
            ctx.commands()[1],
            PaintCommand::DrawRoundedRectShadow { .. }
        ));
        assert!(matches!(
            ctx.commands()[2],
            PaintCommand::FillRoundedRect { .. }
        ));
        assert!(matches!(
            ctx.commands()[3],
            PaintCommand::StrokeRoundedRect { .. }
        ));
        assert_eq!(
            elevation_outsets(ElevationRole::Floating),
            EdgeInsets::new(10.0, 6.0, 10.0, 14.0)
        );
    }

    #[test]
    fn text_selection_uses_a_light_accent_wash() {
        let accent = Color::rgb_f32(0.82, 0.15, 0.15);
        let selection = text_selection_highlight(accent, Color::WHITE);
        let expected = accent
            .lighten(0.12)
            .with_opacity(0.24)
            .blend_over(Color::WHITE);

        assert_eq!(selection, expected);
        assert_eq!(selection.a, 1.0);
        assert!(selection.r - selection.g > 0.15);
    }

    #[test]
    fn surface_levels_follow_the_semantic_tonal_hierarchy() {
        let palette = ColorPalette::default();

        assert_eq!(
            surface_color(&palette, SurfaceRole::Canvas),
            palette.background()
        );
        assert_eq!(
            surface_color(&palette, SurfaceRole::Structural),
            palette.background_secondary()
        );
        assert_eq!(
            surface_color(&palette, SurfaceRole::Section),
            palette.background_tertiary()
        );
        assert_eq!(
            surface_color(&palette, SurfaceRole::Floating),
            palette.surface()
        );
    }

    #[test]
    fn selected_item_surface_is_lighter_than_text_selection() {
        let palette = ColorPalette::default();
        let surface = surface_color(&palette, SurfaceRole::Floating);
        let item = selected_item_surface(palette.primary(), surface);
        let text = text_selection_highlight(palette.primary(), surface);

        assert_eq!(item.a, 1.0);
        assert!(item.g > text.g);
        assert!(item.r - item.g > 0.08);
    }

    #[test]
    fn compact_controls_share_a_twenty_pixel_visual_height() {
        let metrics = metrics();

        assert_eq!(metrics.toggle_height, metrics.slider_height);
        assert!(metrics.toggle_width > metrics.toggle_height);
        assert!(metrics.toggle_thumb_diameter < metrics.toggle_height);
        assert!(metrics.slider_thumb_diameter < metrics.slider_height);
    }

    #[test]
    fn focus_highlight_is_a_light_opaque_primary_tint() {
        let palette = ColorPalette::default();
        let highlight = focus_highlight(&palette);

        assert_eq!(highlight, palette.primary_light().lighten(0.4));
        assert_eq!(highlight.a, 1.0);
        assert!(highlight.r > highlight.g);
    }
}
