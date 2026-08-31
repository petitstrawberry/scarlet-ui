//! Menu - Dropdown menu content
//!
//! Menu displays dropdown menu items vertically.

use crate::buffer::Buffer;
use crate::color::ColorPalette;
use crate::element::{Element, RenderElement};
use crate::geometry::{EdgeInsets, Point, Rect, Size};
use crate::graphics;
use crate::renderer::PaintContext;
use crate::view::View;
use crate::views::style;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

const MENU_EDGE_PADDING: f32 = 2.0;
const MENU_SEPARATOR_HEIGHT: f32 = 1.0;
const MENU_TEXT_INSET: f32 = 8.0;

/// Menu item action
#[derive(Clone, Copy)]
pub enum MenuAction {
    /// Regular menu command.
    Item,
    /// Separator
    Separator,
    /// Submenu
    Submenu,
}

/// Menu item content
pub struct MenuItemContent {
    label: String,
    action: MenuAction,
    enabled: bool,
    shortcut: Option<String>,
    callback: Option<Arc<dyn Fn() + 'static>>,
}

impl Clone for MenuItemContent {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            action: self.action,
            enabled: self.enabled,
            shortcut: self.shortcut.clone(),
            callback: None, // Callbacks cannot be cloned
        }
    }
}

impl MenuItemContent {
    /// Create a regular enabled menu command.
    ///
    /// # Arguments
    ///
    /// * `label` - Text displayed for the command.
    ///
    /// # Returns
    ///
    /// A command entry. Use [`MenuItemContent::separator`] for a divider.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            action: MenuAction::Item,
            enabled: true,
            shortcut: None,
            callback: Some(Arc::new(|| {})),
        }
    }

    /// Set the action
    pub fn action(mut self, action: MenuAction) -> Self {
        self.action = action;
        self
    }

    /// Set the callback
    pub fn callback(mut self, callback: impl Fn() + 'static) -> Self {
        self.callback = Some(Arc::new(callback));
        self
    }

    /// Set whether the item is enabled
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set the keyboard shortcut
    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Create a separator
    pub fn separator() -> Self {
        Self {
            label: String::new(),
            action: MenuAction::Separator,
            enabled: true,
            shortcut: None,
            callback: None,
        }
    }

    /// Get the callback
    pub fn get_callback(&self) -> Option<&Arc<dyn Fn() + 'static>> {
        self.callback.as_ref()
    }
}

/// Menu View - displays dropdown menu items vertically
#[derive(Clone)]
pub struct Menu {
    items: Vec<MenuItemContent>,
    item_height: Option<f32>,
    width: f32,
}

impl Menu {
    /// Create a new menu whose rows adapt to the current interaction density.
    ///
    /// The row height is resolved during layout, so an already-created menu
    /// grows from the compact pointer target to the touch target when the
    /// input environment changes.
    ///
    /// # Arguments
    ///
    /// * `items` - Entries displayed by the menu.
    ///
    /// # Returns
    ///
    /// A menu with adaptive row heights and the default menu width.
    pub fn new(items: Vec<MenuItemContent>) -> Self {
        Self {
            items,
            item_height: None,
            width: 200.0, // Default menu width
        }
    }

    /// Return the current adaptive menu row height.
    ///
    /// This is a snapshot of the shared visual metrics. Prefer [`Menu::new`]
    /// for view-based menus or [`MenuRenderObject::new_adaptive`] for direct
    /// legacy-popup rendering so the height follows future theme changes.
    ///
    /// # Returns
    ///
    /// The current compact menu row height.
    pub fn adaptive_item_height() -> f32 {
        style::metrics().minimum_control_height
    }

    /// Request a custom item height.
    ///
    /// An explicit height is preserved across input-environment changes. Leave
    /// the height unspecified to use the adaptive density policy.
    ///
    /// # Arguments
    ///
    /// * `height` - Preferred logical row height.
    ///
    /// # Returns
    ///
    /// The menu configured with the requested row height.
    pub fn item_height(mut self, height: f32) -> Self {
        self.item_height = Some(height);
        self
    }

    /// Set the menu width
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Get the menu items
    pub fn items(&self) -> &[MenuItemContent] {
        &self.items
    }
}

impl View for Menu {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            MenuRenderObject::with_requested_item_height(
                self.items.clone(),
                self.item_height,
                self.width,
            ),
        ))
    }

    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        Vec::new()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Menu RenderObject - handles vertical layout of menu items
pub struct MenuRenderObject {
    items: Vec<MenuItemContent>,
    requested_item_height: Option<f32>,
    width: f32,
    hovered_index: Option<usize>,
    size: Size,
    buffer: Option<Buffer>,
}

impl MenuRenderObject {
    /// Create a menu render object with an explicit row height.
    ///
    /// Explicit geometry is not enlarged solely because the input environment
    /// reports touch or tablet posture. Use [`MenuRenderObject::new_adaptive`]
    /// when environment-driven density is desired.
    ///
    /// # Arguments
    ///
    /// * `items` - Entries displayed by the menu.
    /// * `item_height` - Preferred logical row height.
    /// * `width` - Logical width of the menu surface.
    ///
    /// # Returns
    ///
    /// A render object that preserves the requested height.
    pub fn new(items: Vec<MenuItemContent>, item_height: f32, width: f32) -> Self {
        Self::with_requested_item_height(items, Some(item_height), width)
    }

    /// Create a menu render object with theme-adaptive row heights.
    ///
    /// Use this for legacy popup consumers instead of supplying a hard-coded
    /// desktop height such as `28.0`. The row height is resolved each time the
    /// menu is laid out from the shared visual metrics. Tablet posture alone
    /// does not enlarge persistent menus.
    ///
    /// # Arguments
    ///
    /// * `items` - Entries displayed by the menu.
    /// * `width` - Logical width of the menu surface.
    ///
    /// # Returns
    ///
    /// A render object with compact rows derived from the current theme.
    pub fn new_adaptive(items: Vec<MenuItemContent>, width: f32) -> Self {
        Self::with_requested_item_height(items, None, width)
    }

    fn with_requested_item_height(
        items: Vec<MenuItemContent>,
        requested_item_height: Option<f32>,
        width: f32,
    ) -> Self {
        let height = items
            .iter()
            .map(|item| {
                if matches!(item.action, MenuAction::Separator) {
                    MENU_SEPARATOR_HEIGHT
                } else {
                    requested_item_height
                        .unwrap_or_else(Menu::adaptive_item_height)
                        .max(0.0)
                }
            })
            .sum::<f32>()
            + MENU_EDGE_PADDING * 2.0;

        Self {
            items,
            requested_item_height,
            width,
            hovered_index: None,
            size: Size { width, height },
            buffer: None,
        }
    }

    /// Set the hovered item index
    pub fn set_hovered(&mut self, index: Option<usize>) {
        self.hovered_index = index;
    }

    /// Get the hovered item index
    pub fn hovered(&self) -> Option<usize> {
        self.hovered_index
    }

    /// Hit test - returns the item index at the given position
    pub fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x > self.width || y < 0.0 || y > self.size.height {
            return None;
        }

        let item_height = self.effective_item_height();
        let mut current_y = MENU_EDGE_PADDING;

        for (i, item) in self.items.iter().enumerate() {
            let item_h = if matches!(item.action, MenuAction::Separator) {
                MENU_SEPARATOR_HEIGHT
            } else {
                item_height
            };

            if y >= current_y && y < current_y + item_h {
                return Some(i);
            }

            current_y += item_h;
        }

        None
    }

    /// Invoke the action for the given item
    pub fn invoke_item(&self, index: usize) {
        if let Some(item) = self.items.get(index) {
            if let Some(callback) = item.get_callback() {
                if item.enabled {
                    callback();
                }
            }
        }
    }

    /// Calculate total height
    fn calculate_height(&self) -> f32 {
        let item_height = self.effective_item_height();
        self.items
            .iter()
            .map(|item| {
                if matches!(item.action, MenuAction::Separator) {
                    MENU_SEPARATOR_HEIGHT
                } else {
                    item_height
                }
            })
            .sum::<f32>()
            + MENU_EDGE_PADDING * 2.0
    }

    fn effective_item_height(&self) -> f32 {
        self.requested_item_height
            .unwrap_or_else(Menu::adaptive_item_height)
            .max(0.0)
    }
}

impl crate::element::ElementRenderObject for MenuRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        let height = self.calculate_height();
        let width = self.width;

        self.size = Size { width, height };

        // Create buffer for the menu
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

    fn paint_outsets(&self) -> EdgeInsets {
        style::elevation_outsets(style::ElevationRole::Floating)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        // Render menu to buffer
        if crate::debug::is_enabled() {
            crate::logln!(
                "[MenuRenderObject] render: buffer={}",
                self.buffer.is_some()
            );
        }

        let palette = ColorPalette::default();
        let bg_color = style::surface_color(&palette, style::SurfaceRole::Floating);
        let border_color = palette.border();
        let text_color = palette.text_primary();
        let hover_color = palette.menu_hover();
        let separator_color = palette.divider();
        let item_height = self.effective_item_height();

        if let Some(ref mut buffer) = self.buffer {
            let mut canvas = graphics::Canvas::for_buffer(buffer);
            let width = canvas.width();
            let height = canvas.height();

            // Legacy popup windows are opaque. Keep their full rectangular
            // background so transparent rounded corners cannot appear black.
            canvas.fill_rect(0, 0, width, height, bg_color);
            canvas.draw_rect(0, 0, width, height, border_color);

            // Draw items
            let mut current_y = MENU_EDGE_PADDING;
            let font_size = 13.0;

            for (i, item) in self.items.iter().enumerate() {
                if matches!(item.action, MenuAction::Separator) {
                    // Draw separator line
                    let sep_y = current_y as i32;
                    canvas.fill_rect(
                        MENU_EDGE_PADDING as i32,
                        sep_y,
                        width.saturating_sub((MENU_EDGE_PADDING * 2.0) as u32),
                        MENU_SEPARATOR_HEIGHT as u32,
                        separator_color,
                    );
                    current_y += MENU_SEPARATOR_HEIGHT;
                } else {
                    // Draw hover background
                    if self.hovered_index == Some(i) {
                        canvas.fill_rect(
                            MENU_EDGE_PADDING as i32,
                            current_y as i32,
                            width.saturating_sub((MENU_EDGE_PADDING * 2.0) as u32),
                            libm::ceilf(item_height) as u32,
                            hover_color,
                        );
                    }

                    // Draw text
                    let text_x = MENU_TEXT_INSET as i32;
                    let text_y = current_y as i32 + ((item_height as i32 - 16) / 2).max(0);

                    let text_color = if item.enabled {
                        text_color
                    } else {
                        palette.text_tertiary()
                    };

                    canvas.draw_text_sized(text_x, text_y, &item.label, text_color, font_size);

                    // Draw shortcut if present
                    if let Some(ref shortcut) = item.shortcut {
                        let (shortcut_w, _) = graphics::measure_text_sized(shortcut, font_size);
                        let shortcut_x =
                            (width as i32) - shortcut_w as i32 - MENU_TEXT_INSET as i32;
                        canvas.draw_text_sized(shortcut_x, text_y, shortcut, text_color, font_size);
                    }

                    current_y += item_height;
                }
            }
        }
    }

    fn get_buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    fn clear_buffer(&mut self) {
        self.buffer = None;
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let palette = ColorPalette::default();
        let bg_color = style::surface_color(&palette, style::SurfaceRole::Floating);
        let border_color = palette.border();
        let text_color = palette.text_primary();
        let hover_color = palette.menu_hover();
        let separator_color = palette.divider();
        let item_height = self.effective_item_height();

        let rect = Rect::new(origin, self.size);
        style::popover_surface(ctx, rect, bg_color, border_color, palette.shadow());

        let mut current_y = MENU_EDGE_PADDING;
        let font_size = 13.0;
        let width = self.size.width.max(0.0);

        for (i, item) in self.items.iter().enumerate() {
            if matches!(item.action, MenuAction::Separator) {
                ctx.fill_rect(
                    Rect::from_xywh(
                        origin.x + 2.0,
                        origin.y + current_y,
                        (width - MENU_EDGE_PADDING * 2.0).max(0.0),
                        MENU_SEPARATOR_HEIGHT,
                    ),
                    separator_color,
                );
                current_y += MENU_SEPARATOR_HEIGHT;
                continue;
            }

            if self.hovered_index == Some(i) {
                style::item_highlight(
                    ctx,
                    Rect::from_xywh(
                        origin.x + MENU_EDGE_PADDING,
                        origin.y + current_y,
                        (width - MENU_EDGE_PADDING * 2.0).max(0.0),
                        item_height,
                    ),
                    hover_color,
                );
            }

            let item_text_color = if item.enabled {
                text_color
            } else {
                palette.text_tertiary()
            };
            let text_y = origin.y + current_y + ((item_height - 16.0) / 2.0).max(0.0);
            ctx.draw_text(
                Point::new(origin.x + MENU_TEXT_INSET, text_y),
                item.label.clone(),
                item_text_color,
                font_size,
            );

            if let Some(ref shortcut) = item.shortcut {
                let (shortcut_w, _) = graphics::measure_text_sized(shortcut, font_size);
                let shortcut_x = origin.x + width - shortcut_w as f32 - MENU_TEXT_INSET;
                ctx.draw_text(
                    Point::new(shortcut_x, text_y),
                    shortcut.clone(),
                    item_text_color,
                    font_size,
                );
            }

            current_y += item_height;
        }

        true
    }

    fn update(&mut self, new_view: &dyn View) -> crate::element::UpdateResult {
        let Some(menu) = new_view.as_any().downcast_ref::<Menu>() else {
            return crate::element::UpdateResult::Replaced;
        };
        self.items = menu.items.clone();
        self.requested_item_height = menu.item_height;
        self.width = menu.width;
        self.hovered_index = self.hovered_index.filter(|index| *index < self.items.len());
        self.size = Size::new(
            self.width,
            self.items
                .iter()
                .map(|item| {
                    if matches!(item.action, MenuAction::Separator) {
                        MENU_SEPARATOR_HEIGHT
                    } else {
                        self.effective_item_height()
                    }
                })
                .sum::<f32>()
                + MENU_EDGE_PADDING * 2.0,
        );
        crate::element::UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementRenderObject, LayoutConstraints};
    use crate::input_environment::{
        InputEnvironment, install_input_environment, install_test_input_environment,
    };
    use crate::renderer::PaintCommand;

    fn pointer_environment() -> InputEnvironment {
        InputEnvironment::new(1, None, None, false, true, true, false)
    }

    fn touch_environment() -> InputEnvironment {
        InputEnvironment::new(2, Some(true), None, true, false, true, false)
    }

    #[test]
    fn adaptive_rows_keep_visual_density_across_posture_changes() {
        let _environment = install_test_input_environment(pointer_environment());
        let mut menu = MenuRenderObject::new_adaptive(
            alloc::vec![MenuItemContent::new("Open"), MenuItemContent::new("Close")],
            120.0,
        );

        assert_eq!(menu.layout(LayoutConstraints::unconstrained()).height, 52.0);
        assert_eq!(menu.hit_test(8.0, 25.0), Some(0));
        assert_eq!(menu.hit_test(8.0, 26.0), Some(1));

        install_input_environment(touch_environment());
        assert_eq!(menu.layout(LayoutConstraints::unconstrained()).height, 52.0);
        assert_eq!(menu.hit_test(8.0, 25.0), Some(0));
        assert_eq!(menu.hit_test(8.0, 26.0), Some(1));
    }

    #[test]
    fn explicit_rows_do_not_grow_in_touch_environment() {
        let _environment = install_test_input_environment(touch_environment());
        let mut menu =
            MenuRenderObject::new(alloc::vec![MenuItemContent::new("Open")], 28.0, 120.0);

        assert_eq!(menu.layout(LayoutConstraints::unconstrained()).height, 32.0);
    }

    #[test]
    #[allow(deprecated)]
    fn legacy_buffer_matches_retained_menu_semantics() {
        let _environment = install_test_input_environment(pointer_environment());
        let palette = ColorPalette::default();
        let mut menu = MenuRenderObject::new_adaptive(
            alloc::vec![
                MenuItemContent::new("Open").shortcut("Ctrl+O"),
                MenuItemContent::separator(),
                MenuItemContent::new("Unavailable").enabled(false),
            ],
            120.0,
        );
        menu.set_hovered(Some(0));
        menu.layout(LayoutConstraints::unconstrained());
        menu.render();

        let buffer = menu
            .get_buffer()
            .expect("layout creates a legacy popup buffer");
        assert_eq!(buffer.get_pixel(0, 0), Some(palette.border().to_bgra()));
        assert_eq!(buffer.get_pixel(60, 0), Some(palette.border().to_bgra()));
        assert_eq!(
            buffer.get_pixel(50, 10),
            Some(palette.menu_hover().to_bgra())
        );
        assert_eq!(buffer.get_pixel(10, 26), Some(palette.divider().to_bgra()));

        let mut paint = PaintContext::new();
        assert!(menu.paint(&mut paint, Point::ZERO));
        assert!(paint.commands().iter().any(|command| matches!(
            command,
            PaintCommand::FillRoundedRect { corner_radius, color, .. }
                if *corner_radius == style::surface_radius(style::SurfaceRole::Floating)
                    && *color == style::surface_color(&palette, style::SurfaceRole::Floating)
        )));
        assert!(paint.commands().iter().any(|command| matches!(
            command,
            PaintCommand::StrokeRoundedRect { corner_radius, color, .. }
                if *corner_radius == style::surface_radius(style::SurfaceRole::Floating)
                    && *color == palette.border()
        )));
        assert!(paint.commands().iter().any(|command| matches!(
            command,
            PaintCommand::FillRoundedRect { corner_radius, color, .. }
                if *corner_radius == style::metrics().item_radius && *color == palette.menu_hover()
        )));
        assert!(paint.commands().iter().any(|command| matches!(
            command,
            PaintCommand::FillPath { color, .. } if *color == palette.divider()
        )));
        assert!(paint.commands().iter().any(|command| matches!(
            command,
            PaintCommand::DrawText { text, color, .. }
                if text == "Unavailable" && *color == palette.text_tertiary()
        )));
    }
}
