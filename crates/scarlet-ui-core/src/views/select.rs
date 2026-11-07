//! Select View - single-choice pull-down control.
//!
//! Select displays the current value and expands into a list of options when
//! clicked.

use crate::os::Mutex;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, RenderElement};
use crate::geometry::{EdgeInsets, Point, Rect, Size};
use crate::renderer::PaintContext;
use crate::state::State;
use crate::view::View;
use crate::views::style;

static SELECT_EXPANDED_REGISTRY: Mutex<BTreeMap<crate::state::StateId, State<bool>>> =
    Mutex::new(BTreeMap::new());

fn select_expanded_state(selected_index: &State<usize>) -> State<bool> {
    let id = selected_index.id();
    let mut registry = SELECT_EXPANDED_REGISTRY.lock();
    if let Some(state) = registry.get(&id) {
        return state.clone();
    }
    let state: State<bool> = State::initial(crate::state::generate_state_id());
    registry.insert(id, state.clone());
    state
}

/// Select change callback type.
pub type SelectChangeCallback = Box<dyn Fn(usize) + 'static>;

/// Visual presentation of a single-choice picker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectStyle {
    /// A full-width form field. This is the default.
    #[default]
    Form,
    /// A right-aligned value and compact circular chevron button.
    ///
    /// The overlay menu has 24-pixel rows, independent of the trigger height.
    Menu,
}

const MENU_ROW_HEIGHT: f32 = 24.0;
const POPUP_GAP: f32 = 6.0;
const POPUP_PADDING: f32 = 4.0;
const POPUP_RADIUS: f32 = 12.0;

/// Select View - displays a selected option and expands on click.
#[derive(Clone)]
pub struct Select {
    options: Vec<String>,
    selected_index: State<usize>,
    expanded: State<bool>,
    on_change: Option<Arc<dyn Fn(usize) + 'static>>,
    width: f32,
    row_height: Option<f32>,
    placeholder: String,
    max_visible_rows: usize,
    select_style: SelectStyle,
    enabled: bool,
}

impl Select {
    /// Create a new Select.
    ///
    /// # Arguments
    ///
    /// * `options` - Labels displayed by the select control.
    /// * `selected_index` - State containing the selected option index.
    ///
    /// # Returns
    ///
    /// A Select view bound to the supplied state.
    pub fn new(options: Vec<String>, selected_index: State<usize>) -> Self {
        let expanded = select_expanded_state(&selected_index);
        Self {
            options,
            selected_index,
            expanded,
            on_change: None,
            width: 260.0,
            row_height: None,
            placeholder: String::from("Select…"),
            max_visible_rows: 8,
            select_style: SelectStyle::Form,
            enabled: true,
        }
    }

    /// Choose a full form field or a compact, right-aligned menu picker.
    pub fn style(mut self, select_style: SelectStyle) -> Self {
        self.select_style = select_style;
        self
    }

    /// Enable selection, or display a non-interactive disabled control.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Set the change callback.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function called with the newly selected option index.
    ///
    /// # Returns
    ///
    /// The updated Select view.
    pub fn on_change(mut self, callback: impl Fn(usize) + 'static) -> Self {
        self.on_change = Some(Arc::new(callback));
        self
    }

    /// Set the preferred control width.
    ///
    /// Finite parent width constraints may allocate a narrower control, including
    /// zero width. Use an explicit remaining-width frame inside a labeled row.
    ///
    /// # Arguments
    ///
    /// * `width` - Logical width in pixels.
    ///
    /// # Returns
    ///
    /// The updated Select view.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Set the row height.
    ///
    /// # Arguments
    ///
    /// * `row_height` - Logical row height in pixels.
    ///
    /// # Returns
    ///
    /// The updated Select view.
    pub fn row_height(mut self, row_height: f32) -> Self {
        self.row_height = Some(row_height);
        self
    }

    /// Set placeholder text shown when no option is selected.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Set the maximum number of visible rows before scrolling.
    pub fn max_visible_rows(mut self, max: usize) -> Self {
        self.max_visible_rows = max.max(1);
        self
    }

    /// Get the selected index state.
    ///
    /// # Returns
    ///
    /// State storing the selected option index.
    pub fn selected_index(&self) -> &State<usize> {
        &self.selected_index
    }

    /// Get the expanded state.
    ///
    /// # Returns
    ///
    /// State storing whether the option list is open.
    pub fn expanded(&self) -> &State<bool> {
        &self.expanded
    }

    pub fn option_count(&self) -> usize {
        self.options.len()
    }

    /// Invoke the change callback if present.
    ///
    /// # Arguments
    ///
    /// * `index` - Newly selected option index.
    pub fn invoke_on_change(&self, index: usize) {
        if !self.enabled {
            return;
        }
        if let Some(callback) = self.on_change.as_ref() {
            callback(index);
        }
    }
}

impl View for Select {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            SelectRenderObject::new(
                self.options.clone(),
                self.selected_index.get(),
                self.expanded.get(),
                self.width,
                self.row_height.unwrap_or_else(style::control_height),
                self.placeholder.clone(),
                self.max_visible_rows,
            )
            .with_row_height(self.row_height)
            .with_enabled(self.enabled)
            .with_style(self.select_style),
        ))
    }

    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        alloc::vec![&self.selected_index, &self.expanded]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Select RenderObject - handles select rendering and hit testing.
pub struct SelectRenderObject {
    options: Vec<String>,
    selected_index: usize,
    expanded: bool,
    width: f32,
    row_height: Option<f32>,
    placeholder: String,
    max_visible_rows: usize,
    select_style: SelectStyle,
    scroll_offset: usize,
    popup_viewport: Option<Rect>,
    buffer_origin: Point,
    hovered_index: Option<usize>,
    enabled: bool,
    focused: bool,
    hovered: bool,
    pressed: bool,
    size: Size,
    buffer: Option<Buffer>,
}

impl SelectRenderObject {
    /// Create a new SelectRenderObject.
    ///
    /// # Arguments
    ///
    /// * `options` - Labels shown by the select control.
    /// * `selected_index` - Currently selected option index.
    /// * `expanded` - Whether the option list starts open.
    /// * `width` - Preferred logical width in pixels.
    /// * `row_height` - Logical height of each row in pixels.
    ///
    /// # Returns
    ///
    /// A render object for a Select view.
    pub fn new(
        options: Vec<String>,
        selected_index: usize,
        expanded: bool,
        width: f32,
        row_height: f32,
        placeholder: String,
        max_visible_rows: usize,
    ) -> Self {
        let mut render_object = Self {
            options,
            selected_index,
            expanded,
            width,
            row_height: Some(row_height),
            placeholder,
            max_visible_rows,
            select_style: SelectStyle::Form,
            scroll_offset: 0,
            popup_viewport: None,
            buffer_origin: Point::ZERO,
            hovered_index: None,
            enabled: true,
            focused: false,
            hovered: false,
            pressed: false,
            size: Size::ZERO,
            buffer: None,
        };
        render_object.selected_index = render_object.clamped_selected_index();
        render_object
    }

    fn with_row_height(mut self, height: Option<f32>) -> Self {
        self.row_height = height;
        self
    }

    fn with_style(mut self, select_style: SelectStyle) -> Self {
        self.select_style = select_style;
        self.reset_scroll_on_open();
        self
    }

    fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        if !enabled {
            self.expanded = false;
        }
        self
    }

    pub(crate) fn is_focused(&self) -> bool {
        self.focused && self.enabled
    }
    pub(crate) fn set_focused(&mut self, focused: bool) {
        self.focused = focused && self.enabled;
        if !self.focused {
            self.pressed = false;
            self.expanded = false;
        }
    }
    pub(crate) fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered && self.enabled;
    }
    pub(crate) fn set_pressed(&mut self, pressed: bool) {
        self.pressed = pressed && self.enabled;
    }
    pub(crate) fn is_pressed(&self) -> bool {
        self.pressed
    }

    /// Set whether the list is expanded.
    ///
    /// # Arguments
    ///
    /// * `expanded` - Whether the option list should be shown.
    pub fn set_expanded(&mut self, expanded: bool) {
        let opening = expanded && self.enabled && !self.expanded;
        self.expanded = expanded && self.enabled;
        if opening {
            self.reset_scroll_on_open();
        }
    }

    /// Window viewport translated into this control's local coordinates.
    /// It is supplied by the pipeline, independently of ancestor scroll clips.
    pub(crate) fn set_popup_viewport(&mut self, viewport: Rect) -> bool {
        if self.popup_viewport == Some(viewport) {
            return false;
        }
        self.popup_viewport = Some(viewport);
        if self.expanded {
            self.reset_scroll_on_open();
            self.adjust_scroll();
        }
        self.expanded
    }

    fn reset_scroll_on_open(&mut self) {
        self.scroll_offset = if self.select_style == SelectStyle::Menu {
            self.menu_placement().0
        } else {
            self.selected_index
                .saturating_sub(self.max_visible_rows / 2)
                .min(self.options.len().saturating_sub(self.max_visible_rows))
        };
    }

    /// Return whether the list is expanded.
    ///
    /// # Returns
    ///
    /// `true` when the option list is open.
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub fn paint_height(&self) -> f32 {
        self.popup_height()
    }

    pub fn hovered_index(&self) -> Option<usize> {
        self.hovered_index
    }

    pub(crate) fn page_step(&self) -> usize {
        self.max_visible_rows.saturating_sub(1).max(1)
    }

    pub fn adjust_scroll(&mut self) {
        let visible = self.visible_row_count();
        if visible == 0 {
            return;
        }
        let target = self.hovered_index.unwrap_or(self.selected_index);
        if target < self.scroll_offset {
            self.scroll_offset = target;
        } else if target >= self.scroll_offset + visible {
            self.scroll_offset = target + 1 - visible;
        }
        self.scroll_offset = self
            .scroll_offset
            .min(self.options.len().saturating_sub(visible));
    }

    /// Scroll an open popup by option rows without changing its selected value.
    pub(crate) fn scroll_rows(&mut self, delta: i32) {
        let max = self.options.len().saturating_sub(self.visible_row_count());
        self.scroll_offset = if delta > 0 {
            self.scroll_offset.saturating_add(delta as usize).min(max)
        } else {
            self.scroll_offset
                .saturating_sub(delta.unsigned_abs() as usize)
        };
        self.hovered_index = None;
    }

    pub fn typeahead(&mut self, c: char) -> bool {
        if !c.is_ascii_graphic() && c != ' ' {
            return false;
        }
        let lower = c.to_ascii_lowercase();
        let count = self.options.len();
        if count == 0 {
            return false;
        }
        let start = self.hovered_index.unwrap_or(self.selected_index);
        for offset in 0..count {
            let i = (start + 1 + offset) % count;
            if self.options[i]
                .chars()
                .next()
                .is_some_and(|fc| fc.to_ascii_lowercase() == lower)
            {
                self.hovered_index = Some(i);
                self.adjust_scroll();
                return true;
            }
        }
        false
    }

    /// Set the hovered option index.
    ///
    /// # Arguments
    ///
    /// * `index` - Hovered option index, or `None` when no option is hovered.
    pub fn set_hovered_index(&mut self, index: Option<usize>) {
        self.hovered_index = index.filter(|index| *index < self.options.len());
    }

    /// Find an option index for a local Y coordinate in the expanded list.
    ///
    /// # Arguments
    ///
    /// * `y` - Local Y coordinate inside the select render object.
    ///
    /// # Returns
    ///
    /// Option index under the coordinate, or `None` outside option rows.
    pub fn option_index_at_y(&self, y: f32) -> Option<usize> {
        let rows_top = self.popup_rect().origin.y + POPUP_PADDING;
        let row_height = self.option_row_height();
        let visible = self.visible_row_count();
        if !self.expanded || y < rows_top || y >= rows_top + visible as f32 * row_height {
            return None;
        }
        Some(((y - rows_top) / row_height) as usize + self.scroll_start())
    }

    /// Hit-test an option using both coordinates, including the compact menu width.
    pub fn option_index_at_point(&self, point: Point) -> Option<usize> {
        let popup = self.popup_rect();
        if point.x < popup.origin.x + POPUP_PADDING
            || point.x >= popup.origin.x + popup.size.width - POPUP_PADDING
        {
            return None;
        }
        self.option_index_at_y(point.y)
    }

    fn clamped_selected_index(&self) -> usize {
        if self.options.is_empty() {
            0
        } else {
            self.selected_index.min(self.options.len() - 1)
        }
    }

    fn scroll_start(&self) -> usize {
        self.scroll_offset
            .min(self.options.len().saturating_sub(self.visible_row_count()))
    }

    fn option_row_height(&self) -> f32 {
        match self.select_style {
            SelectStyle::Form => self.preferred_row_height(),
            SelectStyle::Menu => MENU_ROW_HEIGHT,
        }
    }

    fn visible_row_count(&self) -> usize {
        match self.select_style {
            SelectStyle::Form => self.options.len().min(self.max_visible_rows),
            SelectStyle::Menu => self.menu_placement().1,
        }
    }

    // (opening scroll start, visible rows, popup top). The selected row is the
    // anchor, so edge constraints reduce the surrounding rows instead of moving
    // the selected value away from its trigger. Later keyboard/wheel scrolling
    // changes content, not the panel's position.
    fn menu_placement(&self) -> (usize, usize, f32) {
        let count = self.options.len();
        let wanted = count.min(self.max_visible_rows).max(1);
        let selected = self.clamped_selected_index();
        let center = self.effective_row_height() * 0.5;
        let (above_capacity, below_capacity) = if let Some(viewport) = self.popup_viewport {
            (
                libm::floorf(
                    ((center - viewport.top() - POPUP_PADDING - MENU_ROW_HEIGHT * 0.5)
                        / MENU_ROW_HEIGHT)
                        .max(0.0),
                ) as usize,
                libm::floorf(
                    ((viewport.bottom() - center - POPUP_PADDING - MENU_ROW_HEIGHT * 0.5)
                        / MENU_ROW_HEIGHT)
                        .max(0.0),
                ) as usize,
            )
        } else {
            (wanted, wanted)
        };
        let mut above = selected.min(wanted / 2).min(above_capacity);
        let below = count
            .saturating_sub(selected + 1)
            .min(wanted - 1 - above)
            .min(below_capacity);
        above = selected.min(wanted - 1 - below).min(above_capacity);
        let visible = (above + 1 + below).min(count);
        let mut top = center - POPUP_PADDING - (above as f32 + 0.5) * MENU_ROW_HEIGHT;
        // For a trigger partly outside a tiny viewport, visibility takes priority
        // over impossible alignment. Normal in-window triggers remain exact.
        if let Some(viewport) = self.popup_viewport {
            let height = visible as f32 * MENU_ROW_HEIGHT + POPUP_PADDING * 2.0;
            top = top
                .max(viewport.top())
                .min((viewport.bottom() - height).max(viewport.top()));
        }
        (selected.saturating_sub(above), visible, top)
    }

    fn popup_rect(&self) -> Rect {
        let width = match self.select_style {
            SelectStyle::Form => self.size.width,
            SelectStyle::Menu => {
                let widest_label = self.options.iter().fold(0.0_f32, |width, label| {
                    width.max(
                        crate::graphics::measure_text_sized(label, style::CONTROL_FONT_SIZE).0
                            as f32,
                    )
                });
                let preferred = (widest_label + 40.0).max(144.0);
                self.popup_viewport.map_or(preferred, |viewport| {
                    preferred.min(viewport.size.width.max(0.0))
                })
            }
        };
        let top = match self.select_style {
            SelectStyle::Form => self.effective_row_height() + POPUP_GAP,
            SelectStyle::Menu => self.menu_placement().2,
        };
        let left = match self.select_style {
            SelectStyle::Form => 0.0,
            SelectStyle::Menu => {
                let label = self
                    .options
                    .get(self.clamped_selected_index())
                    .map_or(self.placeholder.as_str(), String::as_str);
                let label_width =
                    crate::graphics::measure_text_sized(label, style::CONTROL_FONT_SIZE).0 as f32;
                let trigger_text_x = (self.size.width - 30.0 - label_width).max(4.0);
                let desired = trigger_text_x - 24.0;
                self.popup_viewport.map_or(desired, |viewport| {
                    desired
                        .max(viewport.left())
                        .min((viewport.right() - width).max(viewport.left()))
                })
            }
        };
        Rect::from_xywh(
            left,
            top,
            width,
            self.visible_row_count() as f32 * self.option_row_height() + POPUP_PADDING * 2.0,
        )
    }

    fn popup_height(&self) -> f32 {
        if self.expanded {
            self.popup_rect().bottom().max(self.effective_row_height())
        } else {
            self.effective_row_height()
        }
    }

    fn buffer_bounds(&self) -> Rect {
        let outsets = self.paint_outsets();
        let left = libm::floorf(-outsets.left);
        let top = libm::floorf(-outsets.top);
        let right = libm::ceilf(self.size.width + outsets.right);
        let bottom = libm::ceilf(self.size.height + outsets.bottom);
        Rect::from_xywh(left, top, right - left, bottom - top)
    }

    fn preferred_row_height(&self) -> f32 {
        self.row_height
            .unwrap_or_else(style::control_height)
            .max(0.0)
    }

    fn effective_row_height(&self) -> f32 {
        self.size.height
    }

    fn draw_select(&mut self) {
        if self.size.width <= 0.0 {
            return;
        }
        let bounds = self.buffer_bounds();
        self.buffer_origin = bounds.origin;
        let w = bounds.size.width as u32;
        let h = bounds.size.height as u32;
        if self
            .buffer
            .as_ref()
            .is_none_or(|buffer| buffer.logical_width() != w || buffer.logical_height() != h)
        {
            let scale = self
                .buffer
                .as_ref()
                .map_or(crate::graphics::current_scale_milli(), Buffer::scale_milli);
            self.buffer = Some(Buffer::from_logical_dimensions_with_scale(w, h, scale));
        }
        // One command list is authoritative for both retained and legacy CPU paths.
        let Some(mut buffer) = self.buffer.take() else {
            return;
        };
        {
            let mut ctx = PaintContext::new();
            self.paint(
                &mut ctx,
                Point::new(-self.buffer_origin.x, -self.buffer_origin.y),
            );
            #[allow(deprecated)]
            crate::renderer::CpuPaintRenderer::execute_into_buffer(
                &mut buffer,
                Color::CLEAR,
                &ctx,
                None,
            );
        }
        self.buffer = Some(buffer);
    }
}

impl ElementRenderObject for SelectRenderObject {
    fn layout(&mut self, constraints: crate::element::LayoutConstraints) -> Size {
        self.size =
            constraints.constrain(Size::new(self.width.max(0.0), self.preferred_row_height()));

        let bounds = self.buffer_bounds();
        self.buffer_origin = bounds.origin;
        let w = bounds.size.width as u32;
        let h = bounds.size.height as u32;
        if self
            .buffer
            .as_ref()
            .is_none_or(|buffer| buffer.logical_width() != w || buffer.logical_height() != h)
        {
            self.buffer = Some(Buffer::from_logical_dimensions(w, h));
        }

        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn paint_outsets(&self) -> EdgeInsets {
        if !self.expanded {
            return EdgeInsets::ZERO;
        }
        let shadow = style::elevation_outsets(style::ElevationRole::Floating);
        let popup = self.popup_rect();
        EdgeInsets::new(
            (shadow.left - popup.left()).max(0.0),
            (shadow.top - popup.top()).max(0.0),
            (popup.right() + shadow.right - self.size.width).max(0.0),
            (popup.bottom() + shadow.bottom - self.size.height).max(0.0),
        )
    }

    fn buffer_origin(&self) -> Point {
        self.buffer_origin
    }

    fn captures_wheel_event(&self, event: &crate::event::MouseEvent) -> bool {
        matches!(event, crate::event::MouseEvent::Wheel { x, y, .. }
            if self.expanded && self.popup_rect().contains(Point::new(*x as f32, *y as f32)))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn hit_test(&self, point: crate::geometry::Point) -> bool {
        self.size.width > 0.0
            && if self.expanded && self.select_style == SelectStyle::Menu {
                self.popup_rect().contains(point)
            } else {
                Rect::new(Point::ZERO, self.size).contains(point)
                    || (self.expanded && self.popup_rect().contains(point))
            }
    }

    fn render(&mut self) {
        self.draw_select();
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
        let w = self.size.width;
        if w <= 0.0 {
            return true;
        }
        let trigger_h = self.effective_row_height();
        let selected_index = self.clamped_selected_index();
        let selected_label = self.options.get(selected_index);
        let palette = ColorPalette::default();
        let text = if self.enabled {
            palette.text_primary()
        } else {
            palette.text_tertiary()
        };
        let subtle = if self.enabled {
            palette.text_secondary()
        } else {
            palette.text_tertiary()
        };
        let label = selected_label.map_or(self.placeholder.as_str(), String::as_str);
        let label_color = if selected_label.is_some() {
            text
        } else {
            subtle
        };
        let trigger = Rect::from_xywh(origin.x, origin.y, w, trigger_h);
        let text_y = origin.y + style::control_text_y(trigger_h, style::CONTROL_FONT_SIZE);

        match self.select_style {
            SelectStyle::Form => {
                let mut material = style::GlassMaterial::editing();
                if !self.enabled {
                    material.tint = palette.background_secondary().with_opacity(0.72);
                } else if self.pressed {
                    material.tint = material.tint.darken(0.035);
                } else if self.hovered {
                    material.tint = material.tint.darken(0.012);
                }
                style::paint_glass_material(
                    ctx,
                    trigger,
                    style::metrics().control_radius,
                    material,
                );
                let inset = style::control_padding(None).left;
                let trailing = inset + 10.0 + 8.0;
                if w > inset + trailing {
                    ctx.push_clip(Rect::from_xywh(
                        origin.x + inset,
                        origin.y,
                        w - inset - trailing,
                        trigger_h,
                    ));
                    ctx.draw_text(
                        Point::new(origin.x + inset, text_y),
                        String::from(label),
                        label_color,
                        style::CONTROL_FONT_SIZE,
                    );
                    ctx.pop_clip();
                }
                if w >= 24.0 {
                    let x = origin.x + w - inset - 10.0;
                    let y = origin.y + (trigger_h - 6.0) * 0.5;
                    ctx.draw_line(Point::new(x, y), Point::new(x + 5.0, y + 5.0), 1.5, subtle);
                    ctx.draw_line(
                        Point::new(x + 5.0, y + 5.0),
                        Point::new(x + 10.0, y),
                        1.5,
                        subtle,
                    );
                }
            }
            SelectStyle::Menu if !self.expanded => {
                // Only the small chevron segment has a material surface. The
                // value stays on its parent's surface, with a real pixel clip.
                if w >= 24.0 {
                    let button = Rect::from_xywh(
                        origin.x + w - 22.0,
                        origin.y + (trigger_h - 20.0) * 0.5,
                        20.0,
                        20.0,
                    );
                    let fill = if self.enabled {
                        palette.text_secondary().with_opacity(if self.pressed {
                            0.18
                        } else if self.hovered {
                            0.12
                        } else {
                            0.08
                        })
                    } else {
                        palette.text_tertiary().with_opacity(0.06)
                    };
                    let mut material = style::GlassMaterial::thumb();
                    material.tint = fill.blend_over(material.tint);
                    style::paint_glass_material(ctx, button, 10.0, material);
                    let cx = button.origin.x + 10.0;
                    let cy = button.origin.y + 10.0;
                    for (dy, direction) in [(-2.0, -1.0), (2.0, 1.0)] {
                        ctx.draw_line(
                            Point::new(cx - 2.5, cy + dy),
                            Point::new(cx, cy + dy + direction * 2.5),
                            1.25,
                            subtle,
                        );
                        ctx.draw_line(
                            Point::new(cx, cy + dy + direction * 2.5),
                            Point::new(cx + 2.5, cy + dy),
                            1.25,
                            subtle,
                        );
                    }
                }
                if w > 44.0 {
                    let right = origin.x + w - 30.0;
                    let label_width =
                        crate::graphics::measure_text_sized(label, style::CONTROL_FONT_SIZE).0
                            as f32;
                    let label_x = (right - label_width).max(origin.x + 4.0);
                    ctx.push_clip(Rect::from_xywh(
                        origin.x + 4.0,
                        origin.y,
                        w - 34.0,
                        trigger_h,
                    ));
                    ctx.draw_text(
                        Point::new(label_x, text_y),
                        String::from(label),
                        label_color,
                        style::CONTROL_FONT_SIZE,
                    );
                    ctx.pop_clip();
                }
            }
            SelectStyle::Menu => {}
        }

        if self.expanded {
            let local_popup = self.popup_rect();
            let popup = Rect::new(
                Point::new(
                    origin.x + local_popup.origin.x,
                    origin.y + local_popup.origin.y,
                ),
                local_popup.size,
            );
            style::floating_shadow(ctx, popup, POPUP_RADIUS, palette.shadow());
            style::paint_menu_material(ctx, popup, POPUP_RADIUS);
            ctx.push_rounded_clip(popup, POPUP_RADIUS);
            // Opening moves keyboard focus feedback to the active row. Keep
            // the menu material neutral, without a tinted header or full wash.
            let row_h = self.option_row_height();
            let scroll_start = self.scroll_start();
            let scroll_end = (scroll_start + self.visible_row_count()).min(self.options.len());
            for index in scroll_start..scroll_end {
                let y = popup.origin.y + POPUP_PADDING + (index - scroll_start) as f32 * row_h;
                let active = self.hovered_index == Some(index);
                let row_text = if active { Color::WHITE } else { text };
                if active {
                    ctx.fill_rounded_rect(
                        Rect::from_xywh(
                            popup.origin.x + POPUP_PADDING,
                            y,
                            (popup.size.width - POPUP_PADDING * 2.0).max(0.0),
                            row_h,
                        ),
                        7.0,
                        palette.primary(),
                    );
                }
                if popup.size.width > 44.0 {
                    ctx.push_clip(Rect::from_xywh(
                        popup.origin.x + 24.0,
                        y,
                        popup.size.width - 36.0,
                        row_h,
                    ));
                    ctx.draw_text(
                        Point::new(
                            popup.origin.x + 24.0,
                            y + style::control_text_y(row_h, style::CONTROL_FONT_SIZE),
                        ),
                        self.options[index].clone(),
                        row_text,
                        style::CONTROL_FONT_SIZE,
                    );
                    ctx.pop_clip();
                }
                if index == selected_index && popup.size.width >= 24.0 {
                    let cx = popup.origin.x + 9.0;
                    let cy = y + row_h * 0.5;
                    ctx.draw_line(
                        Point::new(cx, cy),
                        Point::new(cx + 3.0, cy + 3.0),
                        1.5,
                        row_text,
                    );
                    ctx.draw_line(
                        Point::new(cx + 3.0, cy + 3.0),
                        Point::new(cx + 8.0, cy - 3.0),
                        1.5,
                        row_text,
                    );
                }
            }
            ctx.pop_clip();
        }
        true
    }

    fn update(&mut self, new_view: &dyn crate::view::View) -> crate::element::UpdateResult {
        if let Some(select) = new_view.as_any().downcast_ref::<Select>() {
            let new_options = select.options.clone();
            let new_selected_index = select.selected_index.get();
            let new_expanded = select.expanded.get();
            let new_width = select.width;
            let new_row_height = select.row_height;
            let new_placeholder = select.placeholder.clone();
            let new_max_visible_rows = select.max_visible_rows;

            if self.options != new_options
                || self.selected_index != new_selected_index
                || self.expanded != new_expanded
                || (self.width - new_width).abs() > 0.001
                || self.row_height != new_row_height
                || self.placeholder != new_placeholder
                || self.max_visible_rows != new_max_visible_rows
                || self.select_style != select.select_style
                || self.enabled != select.enabled
            {
                let just_expanded = !self.expanded && new_expanded;
                self.options = new_options;
                self.selected_index = new_selected_index;
                self.expanded = new_expanded && select.enabled;
                self.enabled = select.enabled;
                if !self.enabled {
                    self.focused = false;
                    self.hovered = false;
                    self.pressed = false;
                    self.hovered_index = None;
                    select.expanded.set(false);
                }
                self.select_style = select.select_style;
                self.width = new_width;
                self.row_height = new_row_height;
                self.placeholder = new_placeholder;
                self.max_visible_rows = new_max_visible_rows;
                self.selected_index = self.clamped_selected_index();
                if just_expanded {
                    self.hovered_index = None;
                    self.reset_scroll_on_open();
                    self.adjust_scroll();
                }
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

    fn sample_select(expanded: bool) -> SelectRenderObject {
        SelectRenderObject::new(
            alloc::vec![String::from("Wide WWWWW label 日本語")],
            0,
            expanded,
            260.0,
            32.0,
            String::new(),
            6,
        )
    }

    #[test]
    fn select_honors_narrow_and_zero_width_constraints_without_changing_preferred_width() {
        for expanded in [false, true] {
            let mut select = sample_select(expanded);
            for width in [260.0, 156.0, 24.0, 8.0, 0.0] {
                assert_eq!(
                    select.layout(LayoutConstraints::loose(width, 100.0)).width,
                    width
                );
                let mut ctx = PaintContext::new();
                select.paint(&mut ctx, Point::ZERO);
                if width == 0.0 {
                    assert!(ctx.commands().is_empty());
                    assert!(!select.hit_test(Point::ZERO));
                    select.render(); // Zero-width CPU fallback must not underflow.
                } else if width <= 44.0 {
                    if width < 24.0 && !expanded {
                        assert!(
                            !ctx.commands()
                                .iter()
                                .any(|c| matches!(c, PaintCommand::StrokePath { .. }))
                        );
                    }
                    assert!(
                        !ctx.commands()
                            .iter()
                            .any(|c| matches!(c, PaintCommand::DrawText { .. }))
                    );
                }
            }
            assert_eq!(
                select
                    .layout(LayoutConstraints::loose(f32::INFINITY, 100.0))
                    .width,
                260.0
            );
        }
    }

    #[test]
    fn select_labels_are_pixel_clipped_inside_control_and_popup_indicator_gutter() {
        for expanded in [false, true] {
            let mut select = sample_select(expanded);
            select.layout(LayoutConstraints::loose(156.0, 100.0));
            let origin = Point::new(7.0, 11.0);
            let mut ctx = PaintContext::new();
            select.paint(&mut ctx, origin);
            let mut clips = alloc::vec::Vec::new();
            let mut labels = 0;
            for command in ctx.commands() {
                match command {
                    PaintCommand::PushClip { rect, .. } => clips.push(*rect),
                    PaintCommand::PopClip => {
                        clips.pop().expect("balanced clips");
                    }
                    PaintCommand::DrawText { .. } => {
                        let clip = clips.last().expect("every label is clipped");
                        assert_eq!(
                            clip.origin.x,
                            origin.x + if labels == 0 { 8.0 } else { 24.0 }
                        );
                        assert_eq!(clip.size.width, if labels == 0 { 122.0 } else { 120.0 });
                        assert_eq!(clip.size.height, select.effective_row_height());
                        labels += 1;
                    }
                    _ => {}
                }
            }
            assert!(clips.is_empty());
            assert_eq!(labels, if expanded { 2 } else { 1 });
        }
    }

    #[test]
    fn parent_row_can_allocate_select_the_explicit_remaining_width() {
        use crate::view::ViewExt;
        use crate::views::{HStack, Text};
        for available in [324.0, 428.0, 168.0] {
            let select = Select::new(
                alloc::vec![String::from("Regular")],
                State::new(crate::state::generate_state_id(), 0),
            );
            let remaining = available - 150.0 - 18.0;
            let mut row = HStack::new((
                Text::new("Select").frame_width(150.0),
                select.frame_width(remaining),
            ))
            .spacing(18.0)
            .create_element();
            row.layout(LayoutConstraints::loose(available, 100.0));
            assert_eq!(row.children()[1].bounds().size.width, remaining);
            assert!(row.bounds().size.width <= available);
        }
    }

    #[test]
    fn expanding_select_preserves_control_corner_radius() {
        let mut select = SelectRenderObject::new(
            alloc::vec![String::from("Compact"), String::from("Regular")],
            1,
            false,
            200.0,
            32.0,
            String::new(),
            6,
        );
        select.layout(LayoutConstraints::tight(200.0, 32.0));

        let mut collapsed = PaintContext::new();
        select.paint(&mut collapsed, Point::ZERO);
        let collapsed_radius = collapsed
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::BackdropFilterRoundedRect { corner_radius, .. } => {
                    Some(*corner_radius)
                }
                _ => None,
            });

        select.set_expanded(true);
        let mut expanded = PaintContext::new();
        select.paint(&mut expanded, Point::ZERO);
        let expanded_radius = expanded
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::PushClip { corner_radius, .. } if *corner_radius > 0.0 => {
                    Some(*corner_radius)
                }
                _ => None,
            });

        assert_eq!(collapsed_radius, Some(style::metrics().control_radius));
        assert_eq!(expanded_radius, collapsed_radius);
    }
    fn menu_select(expanded: bool) -> SelectRenderObject {
        let mut select = SelectRenderObject::new(
            alloc::vec![
                String::from("Compact"),
                String::from("Regular"),
                String::from("Comfortable")
            ],
            1,
            expanded,
            260.0,
            36.0,
            String::new(),
            8,
        )
        .with_style(SelectStyle::Menu);
        select.layout(LayoutConstraints::loose(260.0, 100.0));
        select
    }

    #[test]
    fn menu_select_overlaps_trigger_with_exact_value_position_and_negative_hits() {
        let mut select = menu_select(false);
        let label_position = |ctx: &PaintContext<'_>| {
            ctx.commands()
                .iter()
                .find_map(|command| match command {
                    PaintCommand::DrawText { position, text, .. } if text == "Regular" => {
                        Some(*position)
                    }
                    _ => None,
                })
                .unwrap()
        };
        let closed_position = {
            let mut closed = PaintContext::new();
            select.paint(&mut closed, Point::ZERO);
            label_position(&closed)
        };
        select.set_expanded(true);
        let mut open = PaintContext::new();
        select.paint(&mut open, Point::ZERO);
        assert_eq!(label_position(&open), closed_position);
        let popup = select.popup_rect();
        assert_eq!(popup.origin.y, -22.0);
        assert_eq!(popup.size.height, 3.0 * 24.0 + 8.0);
        assert!(select.hit_test(Point::new(popup.origin.x + 10.0, -6.0)));
        assert_eq!(
            select.option_index_at_point(Point::new(popup.origin.x + 10.0, -6.0)),
            Some(0)
        );
        assert_eq!(
            select.option_index_at_point(Point::new(popup.origin.x + 10.0, 18.0)),
            Some(1)
        );
        assert_eq!(
            select.option_index_at_point(Point::new(popup.origin.x + 10.0, 42.0)),
            Some(2)
        );
        for point in [
            Point::new(popup.left() - 1.0, 18.0),
            Point::new(popup.right() + 1.0, 18.0),
            Point::new(popup.left() + 10.0, popup.top()),
            Point::new(popup.left() + 10.0, popup.bottom()),
        ] {
            assert_eq!(select.option_index_at_point(point), None);
        }
        assert!(
            popup.right() > select.size.width,
            "content may extend past the old trigger"
        );
    }

    #[test]
    fn menu_select_edge_placement_keeps_selection_visible_and_scroll_panel_stable() {
        let mut select = SelectRenderObject::new(
            (0..100).map(|i| alloc::format!("Option {i}")).collect(),
            50,
            false,
            260.0,
            36.0,
            String::new(),
            8,
        )
        .with_style(SelectStyle::Menu);
        select.layout(LayoutConstraints::loose(260.0, 100.0));
        for viewport in [
            Rect::from_xywh(-50.0, 0.0, 400.0, 200.0),
            Rect::from_xywh(-50.0, -164.0, 400.0, 200.0),
        ] {
            select.set_expanded(false);
            select.set_popup_viewport(viewport);
            select.set_expanded(true);
            let popup = select.popup_rect();
            assert!(popup.top() >= viewport.top() && popup.bottom() <= viewport.bottom());
            assert_eq!(
                select.option_index_at_point(Point::new(popup.left() + 10.0, 18.0)),
                Some(50)
            );
            let original = popup;
            select.scroll_rows(2);
            assert_eq!(
                select.popup_rect(),
                original,
                "scrolling must not move the panel"
            );
            assert_eq!(select.selected_index, 50);
        }
        select.set_popup_viewport(Rect::from_xywh(0.0, -100.0, 260.0, 300.0));
        let popup = select.popup_rect();
        assert_eq!(
            popup.right(),
            260.0,
            "right window edge clamps otherwise overflowing menu"
        );
    }

    #[test]
    fn menu_select_legacy_buffer_preserves_negative_paint_and_shadow_at_both_scales() {
        for scale in [1000, 2000] {
            let mut select = menu_select(true);
            select.buffer = Some(Buffer::from_logical_dimensions_with_scale(260, 36, scale));
            select.draw_select();
            let offset = select.buffer_origin();
            assert!(offset.y < 0.0);
            let mut ctx = PaintContext::new();
            select.paint(&mut ctx, Point::new(40.0, 80.0));
            let mut direct = crate::renderer::CpuPaintRenderer::new(
                Size::new(400.0, 240.0),
                scale,
                Color::CLEAR,
            );
            direct.execute(&ctx);
            let mut composited = Buffer::from_logical_dimensions_with_scale(400, 240, scale);
            composited.composite(
                select.get_buffer().unwrap(),
                ((40.0 + offset.x) * scale as f32 / 1000.0) as i32,
                ((80.0 + offset.y) * scale as f32 / 1000.0) as i32,
                1.0,
            );
            assert_eq!(select.get_buffer().unwrap().scale_milli(), scale);
            let mismatch = composited
                .as_slice()
                .iter()
                .zip(direct.buffer().as_slice())
                .enumerate()
                .find(|(_, (actual, expected))| {
                    // The isolated legacy buffer has a finite transparent
                    // sampling edge, unlike the full-scene backdrop. One alpha
                    // quantization step can differ after premultiplied blur.
                    // Preserve geometry and fully opaque foreground parity.
                    (**actual >> 24).abs_diff(**expected >> 24) > 1
                        || (**expected >> 24 == 255 && actual != expected)
                });
            assert!(
                mismatch.is_none(),
                "legacy negative-origin coverage/opaque pixels differ at scale {scale}: {mismatch:?}"
            );
        }
    }

    #[test]
    fn menu_select_keeps_internal_focus_without_a_ring() {
        for expanded in [false, true] {
            let mut select = menu_select(expanded);
            select.set_focused(true);
            let mut ctx = PaintContext::new();
            select.paint(&mut ctx, Point::ZERO);
            let focus: Vec<_> = ctx
                .commands()
                .iter()
                .filter_map(|command| match command {
                    PaintCommand::BackdropFilterRoundedRect { rect, .. }
                        if rect.size.height == 20.0 =>
                    {
                        Some(*rect)
                    }
                    _ => None,
                })
                .collect();
            if expanded {
                assert!(focus.is_empty());
            } else {
                assert_eq!(focus, alloc::vec![Rect::from_xywh(238.0, 8.0, 20.0, 20.0)]);
            }
            let rings: Vec<_> = ctx
                .commands()
                .iter()
                .filter_map(|command| match command {
                    PaintCommand::StrokeRoundedRect { rect, .. } => Some(*rect),
                    _ => None,
                })
                .collect();
            assert!(rings.is_empty());
        }
    }

    #[test]
    fn form_select_open_focus_keeps_menu_neutral_without_a_header_island() {
        let mut select = sample_select(true);
        select.layout(LayoutConstraints::loose(260.0, 100.0));
        select.set_focused(true);
        let mut ctx = PaintContext::new();
        select.paint(&mut ctx, Point::ZERO);
        let focus: Vec<_> = ctx
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCommand::BackdropFilterRoundedRect { rect, tint, .. }
                    if rect.size.height == select.effective_row_height()
                        && *tint != style::GlassMaterial::editing().tint =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .collect();
        assert!(
            focus.is_empty(),
            "open form focus belongs to its active row"
        );
    }

    #[test]
    fn both_select_styles_use_real_material_without_old_gradient_or_border() {
        for mut select in [
            sample_select(false),
            menu_select(false),
            sample_select(true),
            menu_select(true),
        ] {
            select.layout(LayoutConstraints::loose(260.0, 100.0));
            assert!(select.reads_backdrop());
            let mut ctx = PaintContext::new();
            select.paint(&mut ctx, Point::ZERO);
            assert!(
                ctx.commands().iter().any(|command| matches!(
                    command,
                    PaintCommand::BackdropFilterRoundedRect { .. }
                ))
            );
            assert!(!ctx.commands().iter().any(|command| matches!(
                command,
                PaintCommand::FillVerticalGradientRoundedRect { .. }
                    | PaintCommand::StrokeRoundedRect { .. }
            )));
        }
    }

    #[test]
    fn menu_select_hover_can_clear_without_losing_selected_check() {
        let mut select = menu_select(true);
        let accent = ColorPalette::default().primary();
        for hovered in [Some(0), None] {
            select.set_hovered_index(hovered);
            let mut ctx = PaintContext::new();
            select.paint(&mut ctx, Point::ZERO);
            let highlights: Vec<_> = ctx
                .commands()
                .iter()
                .filter_map(|command| match command {
                    PaintCommand::FillRoundedRect { rect, color, .. } if *color == accent => {
                        Some(*rect)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(highlights.len(), usize::from(hovered.is_some()));
            assert_eq!(select.selected_index, 1);
            // Only the selected check remains; the popup replaces the trigger.
            assert_eq!(
                ctx.commands()
                    .iter()
                    .filter(|command| matches!(command, PaintCommand::StrokePath { .. }))
                    .count(),
                2
            );
            for command in ctx.commands() {
                if let PaintCommand::DrawText { text, color, .. } = command {
                    if text == "Compact" {
                        assert_eq!(
                            *color,
                            if hovered.is_some() {
                                Color::WHITE
                            } else {
                                ColorPalette::default().text_primary()
                            }
                        );
                    }
                    if text == "Regular" {
                        assert_eq!(*color, ColorPalette::default().text_primary());
                    }
                }
            }
        }
    }

    #[test]
    fn menu_select_keeps_untruncated_unicode_in_a_pixel_clip() {
        let mut select = sample_select(true).with_style(SelectStyle::Menu);
        select.layout(LayoutConstraints::loose(156.0, 100.0));
        let mut ctx = PaintContext::new();
        select.paint(&mut ctx, Point::ZERO);
        let labels: Vec<_> = ctx
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCommand::DrawText { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, alloc::vec!["Wide WWWWW label 日本語"]);
    }
}
