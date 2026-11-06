//! Toggle View - On/off switch control
//!
//! Toggle is a switch control that can be on or off.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, RenderElement};
use crate::geometry::{EdgeInsets, Offset, Point, Rect, Size};
use crate::renderer::PaintContext;
use crate::state::State;
use crate::view::View;
use crate::views::style::{self, SurfaceRole};
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

/// Toggle View - on/off switch
#[derive(Clone)]
pub struct Toggle {
    is_on: State<bool>,
    enabled: bool,
    on_change: Option<Arc<dyn Fn(bool)>>,
}

impl Toggle {
    /// Create a new Toggle
    pub fn new(is_on: State<bool>) -> Self {
        Self {
            is_on,
            enabled: true,
            on_change: None,
        }
    }

    pub fn on_change(mut self, callback: impl Fn(bool) + 'static) -> Self {
        self.on_change = Some(Arc::new(callback));
        self
    }

    pub fn invoke_on_change(&self, value: bool) {
        if let Some(callback) = &self.on_change {
            callback(value);
        }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the is_on state
    pub fn get_is_on(&self) -> &State<bool> {
        &self.is_on
    }
}

impl View for Toggle {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(self.clone(), {
            let mut object = ToggleRenderObject::new(self.is_on.get());
            object.enabled = self.enabled;
            object
        }))
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
    enabled: bool,
    focused: bool,
    activation_key: Option<crate::event::KeyCode>,
    hovered: bool,
    pressed: crate::views::press::PressSources,
    size: Size,
    buffer: Option<Buffer>,
}

impl ToggleRenderObject {
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused && self.enabled;
        if !self.focused {
            self.activation_key = None;
        }
    }

    pub(crate) fn activation_key(&mut self, event: crate::event::KeyEvent) -> Option<bool> {
        use crate::event::{KeyCode, KeyEvent};
        match event {
            KeyEvent::Pressed {
                keycode: key @ (KeyCode::Enter | KeyCode::Space),
                modifiers,
            } if !modifiers.control && !modifiers.alt && !modifiers.super_key => {
                let first = self.activation_key.is_none();
                if first {
                    self.activation_key = Some(key);
                }
                Some(first)
            }
            KeyEvent::Released {
                keycode: key @ (KeyCode::Enter | KeyCode::Space),
                ..
            } => {
                if self.activation_key == Some(key) {
                    self.activation_key = None;
                }
                Some(false)
            }
            KeyEvent::Char {
                c: ' ' | '\r' | '\n',
            } => Some(false),
            _ => None,
        }
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    fn colors(&self, palette: &ColorPalette) -> (Color, Color) {
        let base_track = if self.is_on {
            palette.primary()
        } else {
            palette
                .text_primary()
                .with_opacity(0.10)
                .blend_over(style::surface_color(palette, SurfaceRole::Section))
        };
        let track = if self.pressed.is_pressed() {
            base_track.darken(0.035)
        } else if self.hovered {
            base_track.lighten(0.018)
        } else {
            base_track
        };
        let thumb = if self.pressed.is_pressed() {
            style::GlassMaterial::thumb().tint.darken(0.035)
        } else {
            style::GlassMaterial::thumb().tint
        };
        (track, thumb)
    }

    /// Create a new ToggleRenderObject
    pub fn new(is_on: bool) -> Self {
        let metrics = style::metrics();

        Self {
            is_on,
            enabled: true,
            focused: false,
            activation_key: None,
            hovered: false,
            pressed: crate::views::press::PressSources::default(),
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

    pub fn is_mouse_pressed(&self) -> bool {
        self.pressed.is_mouse_pressed()
    }

    pub fn set_pressed(&mut self, pressed: bool) {
        self.pressed.set_mouse(pressed);
    }

    pub fn set_touch_pressed(&mut self, id: u64, pressed: bool) {
        self.pressed.set_touch(id, pressed);
    }

    /// Replay the retained drawing for identical geometry and edges at every scale.
    fn draw_toggle(&mut self) {
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
        let (bg_color, thumb_color) = self.colors(&palette);
        // Keep the existing hit target; the visible reference switch is a slim capsule.
        let track_height = 16.0;
        let track_y = origin.y + (self.size.height - track_height) / 2.0;
        let rect = Rect::from_xywh(origin.x, track_y, self.size.width, track_height);
        ctx.fill_rounded_rect(rect, track_height / 2.0, bg_color);

        let thumb_width = 21.0;
        let thumb_height = 14.0;
        let inset = 1.0;
        let x = if self.is_on {
            self.size.width - inset - thumb_width
        } else {
            inset
        };
        let thumb_rect = Rect::from_xywh(origin.x + x, track_y + inset, thumb_width, thumb_height);
        if self.pressed.is_pressed() {
            let held_rect = Rect::from_xywh(
                thumb_rect.origin.x - 5.0,
                origin.y + self.size.height / 2.0 - 11.0,
                31.0,
                22.0,
            );
            ctx.draw_rounded_rect_shadow(
                held_rect,
                11.0,
                Offset::new(0.0, 0.5),
                2.0,
                0.0,
                Color::BLACK.with_opacity(0.035),
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
                Offset::new(0.0, 0.5),
                2.0,
                0.0,
                Color::BLACK.with_opacity(0.035),
            );
            style::paint_glass_material(
                ctx,
                thumb_rect,
                thumb_height / 2.0,
                style::GlassMaterial {
                    tint: thumb_color,
                    ..style::GlassMaterial::thumb()
                },
            );
        }
        true
    }

    fn update(&mut self, new_view: &dyn crate::view::View) -> crate::element::UpdateResult {
        if let Some(toggle) = new_view.as_any().downcast_ref::<Toggle>() {
            let new_is_on = toggle.is_on.get();
            if self.is_on != new_is_on || self.enabled != toggle.enabled {
                self.enabled = toggle.enabled;
                if !self.enabled {
                    self.activation_key = None;
                    self.focused = false;
                    self.hovered = false;
                    self.pressed = Default::default();
                }
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
    fn focus_does_not_add_chrome() {
        for on in [false, true] {
            for held in [false, true] {
                let mut toggle = ToggleRenderObject::new(on);
                toggle.set_pressed(held);
                let mut unfocused = PaintContext::new();
                toggle.paint(&mut unfocused, Point::ZERO);
                let unfocused = unfocused.into_display_list();
                for focused in [true, false] {
                    toggle.set_focused(focused);
                    assert_eq!(toggle.is_focused(), focused);
                    let mut paint = PaintContext::new();
                    toggle.paint(&mut paint, Point::ZERO);
                    assert_eq!(paint.commands(), unfocused.context().commands());
                    assert_eq!(toggle.get_is_on(), on);
                }
            }
        }
    }

    #[test]
    fn keyboard_activation_retains_focus_and_repeat_suppression() {
        use crate::event::{Event, FocusEvent, KeyCode, KeyEvent, KeyModifiers, Phase};
        let state = State::new(crate::state::generate_state_id(), false);
        let mut element = Toggle::new(state.clone()).create_element();
        let key = |pressed| {
            Event::Keyboard(if pressed {
                KeyEvent::Pressed {
                    keycode: KeyCode::Space,
                    modifiers: KeyModifiers::empty(),
                }
            } else {
                KeyEvent::Released {
                    keycode: KeyCode::Space,
                    modifiers: KeyModifiers::empty(),
                }
            })
        };
        assert!(!element.handle_event(&key(true), Phase::Target));
        assert!(element.handle_event(&Event::Focus(FocusEvent::Gained), Phase::Target));
        for expected in [true, false] {
            assert!(element.handle_event(&key(true), Phase::Target));
            assert!(element.handle_event(&key(true), Phase::Target));
            assert_eq!(state.get(), expected, "held keys must toggle only once");
            assert!(element.handle_event(&key(false), Phase::Target));
            assert!(element.wants_keyboard_focus());
        }
        element.handle_event(&Event::Focus(FocusEvent::Lost), Phase::Target);
        assert!(!element.handle_event(&key(true), Phase::Target));
        assert!(!state.get());
    }

    #[test]
    fn thumb_material_is_uniform_backdrop_only_and_preserves_center() {
        for on in [false, true] {
            let mut control = ToggleRenderObject::new(on);
            let mut idle_center = None;
            for held in [false, true] {
                control.set_pressed(held);
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
                            ..
                        } => Some((*rect, *corner_radius, *blur_radius, *tint)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(materials.len(), 1, "only the thumb reads backdrop");
                let (rect, radius, blur, tint) = materials[0];
                assert_eq!(
                    rect.size,
                    Size::new(
                        if held { 31.0 } else { 21.0 },
                        if held { 22.0 } else { 14.0 }
                    )
                );
                assert_eq!(radius, rect.size.height / 2.0);
                assert!(blur > 0.0 && blur < style::GlassMaterial::thumb().blur_radius);
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

        let PaintCommand::FillRoundedRect { color, .. } = &ctx.commands()[0] else {
            panic!("expected solid toggle track");
        };
        assert_eq!(*color, ColorPalette::default().primary());
    }

    #[test]
    fn inactive_toggle_uses_a_visible_palette_derived_neutral_track() {
        let toggle = ToggleRenderObject::new(false);
        let mut ctx = PaintContext::new();
        toggle.paint(&mut ctx, Point::ZERO);

        let PaintCommand::FillRoundedRect { color, .. } = &ctx.commands()[0] else {
            panic!("expected solid toggle track");
        };
        let palette = ColorPalette::default();
        let fill = palette
            .text_primary()
            .with_opacity(0.10)
            .blend_over(style::surface_color(&palette, SurfaceRole::Section));
        assert_eq!(*color, fill);
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
    fn entire_toggle_has_no_outline_in_any_state() {
        for on in [false, true] {
            for held in [false, true] {
                let mut toggle = ToggleRenderObject::new(on);
                toggle.set_pressed(held);
                let mut ctx = PaintContext::new();
                toggle.paint(&mut ctx, Point::ZERO);
                assert!(!ctx.commands().iter().any(|cmd| matches!(
                    cmd,
                    PaintCommand::StrokeRoundedRect { .. }
                        | PaintCommand::StrokeRect { .. }
                        | PaintCommand::StrokePath { .. }
                )));
            }
        }
    }
    #[test]
    fn glass_thumb_keeps_an_inside_gap_without_highlights() {
        for on in [false, true] {
            let toggle = ToggleRenderObject::new(on);
            let mut ctx = PaintContext::new();
            toggle.paint(&mut ctx, Point::new(10.0, 20.0));
            let lenses: alloc::vec::Vec<_> = ctx
                .commands()
                .iter()
                .filter_map(|cmd| match cmd {
                    PaintCommand::FillRoundedRect { rect, .. }
                    | PaintCommand::BackdropFilterRoundedRect { rect, .. } => Some(*rect),
                    _ => None,
                })
                .collect();
            assert_eq!(lenses.len(), 2);
            assert!(!ctx.commands().iter().any(|cmd| matches!(
                cmd,
                PaintCommand::FillVerticalGradientRoundedRect { .. }
                    | PaintCommand::PushClip { .. }
            )));

            assert_eq!(lenses[1].size, Size::new(21.0, 14.0));
            assert_eq!(lenses[1].origin.y, 23.0);
            assert_eq!(lenses[1].origin.x, if on { 24.0 } else { 11.0 });
        }
    }

    #[test]
    fn legacy_buffer_matches_retained_material_at_1x_and_2x() {
        for scale in [1000, 2000] {
            for held in [false, true] {
                let mut control = ToggleRenderObject::new(true);
                control.set_pressed(held);
                control.buffer = Some(Buffer::from_logical_dimensions_with_scale(
                    control.size.width as u32,
                    control.size.height as u32,
                    scale,
                ));
                control.draw_toggle();
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
    fn held_capsule(control: &ToggleRenderObject) -> Option<(Rect, Color)> {
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
        let mut control = ToggleRenderObject::new(true);
        let size = control.size();
        let outsets = control.paint_outsets();
        assert!(held_capsule(&control).is_none());
        control.set_pressed(true);
        let (rect, tint) = held_capsule(&control).unwrap();
        assert_eq!(rect.size, Size::new(31.0, 22.0));
        assert!(tint.a > 0.0 && tint.a < 1.0);
        assert!(rect.origin.x >= -outsets.left);
        assert!(rect.origin.y >= -outsets.top);
        assert!(rect.origin.x + rect.size.width <= size.width + outsets.right);
        assert!(rect.origin.y + rect.size.height <= size.height + outsets.bottom);
        assert_eq!(control.size(), size);
        control.set_pressed(false);
        assert!(held_capsule(&control).is_none());
        assert_eq!(control.paint_outsets(), outsets);
        control.set_touch_pressed(7, true);
        assert!(held_capsule(&control).is_some());
        control.set_touch_pressed(7, false);
        assert!(held_capsule(&control).is_none());
    }
}
