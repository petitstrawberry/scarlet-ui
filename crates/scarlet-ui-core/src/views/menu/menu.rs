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

const MENU_EDGE_PADDING: f32 = 6.0;
const MENU_SEPARATOR_ROW_HEIGHT: f32 = 9.0;
const MENU_TEXT_INSET: f32 = 12.0;
const MENU_CHECK_GUTTER: f32 = 16.0;
const MENU_FONT_SIZE: f32 = 13.0;
const MENU_SHORTCUT_GAP: f32 = 24.0;

fn preferred_menu_width(items: &[MenuItemContent]) -> f32 {
    let submenu_gutter = if items
        .iter()
        .any(|item| matches!(item.action, MenuAction::Submenu))
    {
        MENU_CHECK_GUTTER
    } else {
        0.0
    };
    items
        .iter()
        .filter(|item| !matches!(item.action, MenuAction::Separator))
        .map(|item| {
            let label_width = graphics::measure_text_sized(&item.label, MENU_FONT_SIZE).0 as f32;
            let shortcut_width = item.shortcut.as_ref().map_or(0.0, |shortcut| {
                MENU_SHORTCUT_GAP + graphics::measure_text_sized(shortcut, MENU_FONT_SIZE).0 as f32
            });
            MENU_TEXT_INSET * 2.0
                + MENU_CHECK_GUTTER
                + label_width
                + shortcut_width
                + submenu_gutter
        })
        .fold(160.0, f32::max)
}

/// Menu item action
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    checked: bool,
    shortcut: Option<String>,
    callback: Option<Arc<dyn Fn() + 'static>>,
}

impl Clone for MenuItemContent {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            action: self.action,
            enabled: self.enabled,
            checked: self.checked,
            shortcut: self.shortcut.clone(),
            callback: self.callback.clone(),
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
            checked: false,
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

    /// Set whether this command displays a checkmark.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn checked_value(&self) -> bool {
        self.checked
    }
    pub fn enabled_value(&self) -> bool {
        self.enabled
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn shortcut_value(&self) -> Option<&str> {
        self.shortcut.as_deref()
    }
    pub fn action_value(&self) -> MenuAction {
        self.action
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
            checked: false,
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
        style::metrics().minimum_control_height + 4.0
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

    pub(crate) fn popup_renderer(&self, available_width: f32) -> MenuRenderObject {
        MenuRenderObject::with_requested_item_height(
            self.items.clone(),
            self.item_height,
            self.width
                .max(self.preferred_width())
                .min(available_width.max(0.0)),
        )
    }

    /// Get the menu items
    pub fn items(&self) -> &[MenuItemContent] {
        &self.items
    }

    /// Width needed by shared check, label, shortcut, and submenu columns.
    /// Hosts may clamp this to their available viewport; explicit width is unchanged.
    pub fn preferred_width(&self) -> f32 {
        preferred_menu_width(&self.items)
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
                    MENU_SEPARATOR_ROW_HEIGHT
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

    /// Return the measured shared-menu column width before host viewport clamping.
    pub fn preferred_width(&self) -> f32 {
        preferred_menu_width(&self.items)
    }

    /// Return a row's local bounds, using the same layout as hit testing.
    /// Popup controllers can position submenus without duplicating row metrics.
    pub fn item_rect(&self, index: usize) -> Option<Rect> {
        let item = self.items.get(index)?;
        let height = |item: &MenuItemContent| {
            if matches!(item.action, MenuAction::Separator) {
                MENU_SEPARATOR_ROW_HEIGHT
            } else {
                self.effective_item_height()
            }
        };
        let y = MENU_EDGE_PADDING + self.items[..index].iter().map(height).sum::<f32>();
        Some(Rect::from_xywh(0.0, y, self.width, height(item)))
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
                MENU_SEPARATOR_ROW_HEIGHT
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
                if item.enabled && !matches!(item.action, MenuAction::Separator) {
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
                    MENU_SEPARATOR_ROW_HEIGHT
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
        let bg_color = style::GlassMaterial::menu().tint.with_opacity(1.0);
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

            // Draw items
            let mut current_y = MENU_EDGE_PADDING;
            let font_size = MENU_FONT_SIZE;

            for (i, item) in self.items.iter().enumerate() {
                if matches!(item.action, MenuAction::Separator) {
                    // Draw separator line
                    #[cfg(feature = "std")]
                    let sep_y = (current_y + MENU_SEPARATOR_ROW_HEIGHT / 2.0).floor() as i32;
                    #[cfg(not(feature = "std"))]
                    let sep_y = libm::floorf(current_y + MENU_SEPARATOR_ROW_HEIGHT / 2.0) as i32;
                    canvas.fill_rect(
                        MENU_TEXT_INSET as i32,
                        sep_y,
                        width.saturating_sub((MENU_TEXT_INSET * 2.0) as u32),
                        1,
                        separator_color,
                    );
                    current_y += MENU_SEPARATOR_ROW_HEIGHT;
                } else {
                    // Draw hover background
                    if self.hovered_index == Some(i) && item.enabled {
                        canvas.fill_rect(
                            MENU_EDGE_PADDING as i32,
                            current_y as i32,
                            width.saturating_sub((MENU_EDGE_PADDING * 2.0) as u32),
                            libm::ceilf(item_height) as u32,
                            hover_color,
                        );
                    }

                    // Draw text
                    let text_x = (MENU_TEXT_INSET + MENU_CHECK_GUTTER) as i32;
                    let text_y = current_y as i32 + ((item_height as i32 - 16) / 2).max(0);

                    let text_color = if item.enabled {
                        text_color
                    } else {
                        palette.text_tertiary()
                    };

                    if item.checked {
                        let x = MENU_TEXT_INSET as i32;
                        let y = (current_y + item_height / 2.0) as i32;
                        canvas.draw_line(x, y, x + 3, y + 3, text_color);
                        canvas.draw_line(x + 3, y + 3, x + 9, y - 4, text_color);
                    }
                    canvas.draw_text_sized(text_x, text_y, &item.label, text_color, font_size);

                    // Draw shortcut if present
                    if let Some(ref shortcut) = item.shortcut {
                        let (shortcut_w, _) = graphics::measure_text_sized(shortcut, font_size);
                        let shortcut_x = (width as i32)
                            - shortcut_w as i32
                            - MENU_TEXT_INSET as i32
                            - if self
                                .items
                                .iter()
                                .any(|item| matches!(item.action, MenuAction::Submenu))
                            {
                                MENU_CHECK_GUTTER as i32
                            } else {
                                0
                            };
                        canvas.draw_text_sized(
                            shortcut_x,
                            text_y,
                            shortcut,
                            if item.enabled {
                                palette.text_secondary()
                            } else {
                                palette.text_tertiary()
                            },
                            font_size,
                        );
                    }

                    if matches!(item.action, MenuAction::Submenu) {
                        let x = width as i32 - MENU_TEXT_INSET as i32 - 4;
                        let y = (current_y + item_height / 2.0) as i32;
                        canvas.draw_line(x - 2, y - 4, x + 2, y, text_color);
                        canvas.draw_line(x + 2, y, x - 2, y + 4, text_color);
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

    fn reads_backdrop(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let palette = ColorPalette::default();
        let text_color = palette.text_primary();
        let hover_color = palette.menu_hover();
        let separator_color = palette.divider();
        let item_height = self.effective_item_height();

        let rect = Rect::new(origin, self.size);
        let radius = style::surface_radius(style::SurfaceRole::Floating);
        style::elevation_shadow(
            ctx,
            rect,
            radius,
            palette.shadow(),
            style::ElevationRole::Floating,
        );
        style::paint_menu_material(ctx, rect, radius);

        let mut current_y = MENU_EDGE_PADDING;
        let font_size = MENU_FONT_SIZE;
        let width = self.size.width.max(0.0);
        let has_submenus = self
            .items
            .iter()
            .any(|item| matches!(item.action, MenuAction::Submenu));

        for (i, item) in self.items.iter().enumerate() {
            if matches!(item.action, MenuAction::Separator) {
                #[cfg(feature = "std")]
                let separator_offset = (MENU_SEPARATOR_ROW_HEIGHT / 2.0).floor();
                #[cfg(not(feature = "std"))]
                let separator_offset = libm::floorf(MENU_SEPARATOR_ROW_HEIGHT / 2.0);
                ctx.fill_rect(
                    Rect::from_xywh(
                        origin.x + MENU_TEXT_INSET,
                        origin.y + current_y + separator_offset,
                        (width - MENU_TEXT_INSET * 2.0).max(0.0),
                        1.0,
                    ),
                    separator_color,
                );
                current_y += MENU_SEPARATOR_ROW_HEIGHT;
                continue;
            }

            if self.hovered_index == Some(i) && item.enabled {
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
            if item.checked {
                let x = origin.x + MENU_TEXT_INSET;
                let y = origin.y + current_y + item_height / 2.0;
                ctx.draw_line(
                    Point::new(x, y),
                    Point::new(x + 3.0, y + 3.0),
                    1.5,
                    item_text_color,
                );
                ctx.draw_line(
                    Point::new(x + 3.0, y + 3.0),
                    Point::new(x + 9.0, y - 4.0),
                    1.5,
                    item_text_color,
                );
            }
            ctx.draw_text(
                Point::new(origin.x + MENU_TEXT_INSET + MENU_CHECK_GUTTER, text_y),
                item.label.clone(),
                item_text_color,
                font_size,
            );

            if let Some(ref shortcut) = item.shortcut {
                let (shortcut_w, _) = graphics::measure_text_sized(shortcut, font_size);
                let shortcut_x = origin.x + width
                    - shortcut_w as f32
                    - MENU_TEXT_INSET
                    - if has_submenus { MENU_CHECK_GUTTER } else { 0.0 };
                ctx.draw_text(
                    Point::new(shortcut_x, text_y),
                    shortcut.clone(),
                    if item.enabled {
                        palette.text_secondary()
                    } else {
                        palette.text_tertiary()
                    },
                    font_size,
                );
            }

            if matches!(item.action, MenuAction::Submenu) {
                let x = origin.x + width - MENU_TEXT_INSET - 4.0;
                let y = origin.y + current_y + item_height / 2.0;
                let color = if item.enabled {
                    palette.text_secondary()
                } else {
                    palette.text_tertiary()
                };
                ctx.draw_line(
                    Point::new(x - 2.0, y - 4.0),
                    Point::new(x + 2.0, y),
                    1.5,
                    color,
                );
                ctx.draw_line(
                    Point::new(x + 2.0, y),
                    Point::new(x - 2.0, y + 4.0),
                    1.5,
                    color,
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
                        MENU_SEPARATOR_ROW_HEIGHT
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
    fn cloned_menu_keeps_callbacks_but_disabled_and_separator_rows_do_not_activate() {
        use alloc::rc::Rc;
        use core::cell::Cell;
        let calls = Rc::new(Cell::new(0));
        let calls_for_callback = calls.clone();
        let command = MenuItemContent::new("Run")
            .callback(move || calls_for_callback.set(calls_for_callback.get() + 1));
        let menu = Menu::new(alloc::vec![
            command.clone(),
            command.clone().enabled(false),
            command.clone().action(MenuAction::Separator),
        ])
        .clone();
        let renderer = MenuRenderObject::new(menu.items.clone(), 24.0, 120.0);
        renderer.invoke_item(0);
        renderer.invoke_item(1);
        renderer.invoke_item(2);
        renderer.invoke_item(99);
        assert_eq!(calls.get(), 1);
        assert!(Arc::ptr_eq(
            command.get_callback().unwrap(),
            menu.items[0].get_callback().unwrap()
        ));
    }

    #[test]
    fn row_rects_match_hit_testing_across_separators() {
        let menu = MenuRenderObject::new(
            alloc::vec![
                MenuItemContent::new("One"),
                MenuItemContent::separator(),
                MenuItemContent::new("Two"),
            ],
            24.0,
            160.0,
        );
        for (index, y, height) in [(0, 6.0, 24.0), (1, 30.0, 9.0), (2, 39.0, 24.0)] {
            let rect = menu.item_rect(index).unwrap();
            assert_eq!(rect, Rect::from_xywh(0.0, y, 160.0, height));
            assert_eq!(menu.hit_test(4.0, y + height / 2.0), Some(index));
        }
        assert!(menu.item_rect(3).is_none());
    }

    #[test]
    fn submenu_columns_reserve_shortcut_space_and_draw_direction_indicator() {
        let items = alloc::vec![
            MenuItemContent::new("Settings").action(MenuAction::Submenu),
            MenuItemContent::new("Command").shortcut("Ctrl+Shift+K"),
        ];
        let width = Menu::new(items.clone()).preferred_width();
        let menu = MenuRenderObject::new_adaptive(items, width);
        assert_eq!(menu.preferred_width(), width);
        assert!(width >= 160.0);
        let mut paint = PaintContext::new();
        menu.paint(&mut paint, Point::ZERO);
        assert_eq!(
            paint
                .commands()
                .iter()
                .filter(|command| matches!(command,
                    PaintCommand::StrokePath { stroke_width, .. } if *stroke_width == 1.5
                ))
                .count(),
            2,
            "submenu has a two-stroke chevron"
        );
        let shortcut_width = graphics::measure_text_sized("Ctrl+Shift+K", MENU_FONT_SIZE).0 as f32;
        assert!(paint.commands().iter().any(|command| matches!(command,
            PaintCommand::DrawText { text, position, color, .. }
                if text == "Ctrl+Shift+K" && position.x + shortcut_width == width - MENU_TEXT_INSET - MENU_CHECK_GUTTER
                    && *color == ColorPalette::default().text_secondary()
        )));
    }

    #[test]
    fn disabled_row_has_muted_text_without_active_hover_highlight() {
        let mut menu = MenuRenderObject::new_adaptive(
            alloc::vec![MenuItemContent::new("Unavailable").enabled(false)],
            180.0,
        );
        menu.set_hovered(Some(0));
        let mut paint = PaintContext::new();
        menu.paint(&mut paint, Point::ZERO);
        assert!(!paint.commands().iter().any(|command| matches!(command,
            PaintCommand::FillRoundedRect { color, .. } if *color == ColorPalette::default().menu_hover()
        )));
        assert!(paint.commands().iter().any(|command| matches!(command,
            PaintCommand::DrawText { text, color, .. } if text == "Unavailable" && *color == ColorPalette::default().text_tertiary()
        )));
    }

    #[test]
    fn checked_rows_paint_shared_checkmarks_and_align_labels() {
        let _environment = install_test_input_environment(pointer_environment());
        let mut menu = MenuRenderObject::new(
            alloc::vec![
                MenuItemContent::new("Checked").checked(true),
                MenuItemContent::separator().checked(true),
                MenuItemContent::new("Other"),
            ],
            24.0,
            160.0,
        );
        menu.layout(LayoutConstraints::unconstrained());
        let mut paint = PaintContext::new();
        menu.paint(&mut paint, Point::ZERO);
        let check_strokes = paint
            .commands()
            .iter()
            .filter(|command| {
                matches!(command,
                    PaintCommand::StrokePath { stroke_width, .. } if *stroke_width == 1.5
                )
            })
            .count();
        assert_eq!(check_strokes, 2, "separator must not paint a checkmark");
        for label in ["Checked", "Other"] {
            assert!(paint.commands().iter().any(|command| matches!(command,
                PaintCommand::DrawText { text, position, .. }
                    if text == label && position.x == MENU_TEXT_INSET + MENU_CHECK_GUTTER
            )));
        }
        assert_eq!(menu.size.height, 69.0, "separator reserves breathing room");
    }

    #[test]
    fn adaptive_rows_keep_visual_density_across_posture_changes() {
        let _environment = install_test_input_environment(pointer_environment());
        let mut menu = MenuRenderObject::new_adaptive(
            alloc::vec![MenuItemContent::new("Open"), MenuItemContent::new("Close")],
            120.0,
        );

        assert_eq!(menu.layout(LayoutConstraints::unconstrained()).height, 68.0);
        assert_eq!(menu.hit_test(8.0, 33.0), Some(0));
        assert_eq!(menu.hit_test(8.0, 34.0), Some(1));

        install_input_environment(touch_environment());
        assert_eq!(menu.layout(LayoutConstraints::unconstrained()).height, 68.0);
        assert_eq!(menu.hit_test(8.0, 33.0), Some(0));
        assert_eq!(menu.hit_test(8.0, 34.0), Some(1));
    }

    #[test]
    fn explicit_rows_do_not_grow_in_touch_environment() {
        let _environment = install_test_input_environment(touch_environment());
        let mut menu =
            MenuRenderObject::new(alloc::vec![MenuItemContent::new("Open")], 28.0, 120.0);

        assert_eq!(menu.layout(LayoutConstraints::unconstrained()).height, 40.0);
    }

    #[test]
    #[allow(deprecated)]
    fn legacy_popup_is_readable_opaque_fallback_and_retained_menu_reads_backdrop() {
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
        // These assertions inspect physical 1x pixels. Other pipeline tests
        // change the process-wide UI scale, so pin this fixture explicitly.
        menu.buffer = Some(Buffer::from_logical_dimensions_with_scale(
            menu.size.width as u32,
            menu.size.height as u32,
            1000,
        ));
        menu.render();

        let buffer = menu
            .get_buffer()
            .expect("layout creates a legacy popup buffer");
        assert_eq!(
            buffer.get_pixel(0, 0),
            Some(
                style::GlassMaterial::menu()
                    .tint
                    .with_opacity(1.0)
                    .to_bgra()
            )
        );
        assert_eq!(
            buffer.get_pixel(60, 0),
            Some(
                style::GlassMaterial::menu()
                    .tint
                    .with_opacity(1.0)
                    .to_bgra()
            )
        );
        assert_eq!(
            buffer.get_pixel(50, 10),
            Some(palette.menu_hover().to_bgra())
        );
        assert_eq!(buffer.get_pixel(16, 38), Some(palette.divider().to_bgra()));

        let mut paint = PaintContext::new();
        assert!(menu.paint(&mut paint, Point::ZERO));
        assert!(menu.reads_backdrop());
        assert!(paint.commands().iter().any(|command| matches!(
            command,
            PaintCommand::BackdropFilterRoundedRect { corner_radius, tint, .. }
                if *corner_radius == style::surface_radius(style::SurfaceRole::Floating)
                    && *tint == style::GlassMaterial::menu().tint
        )));
        assert!(!paint.commands().iter().any(|command| matches!(
            command,
            PaintCommand::StrokeRoundedRect { .. }
                | PaintCommand::FillVerticalGradientRoundedRect { .. }
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
