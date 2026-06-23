//! ScrollView - clipped viewport for larger child content.
//!
//! `ScrollView` keeps its scroll offset in the render object. The child remains
//! a normal View subtree, so controls inside the scrolled content keep their own
//! elements, hit testing, and event behavior.

use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, LayoutConstraints, RenderElement};
use crate::event::{Event, MouseEvent, Phase, WheelPhase};
use crate::geometry::{Point, Rect, Size};
use crate::renderer::PaintContext;
use crate::view::View;
use crate::views::modifiers::RepaintBoundary;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;
use core::marker::PhantomData;

const DEFAULT_AXIS_LOCK_RATIO: f32 = 1.35;
const DEFAULT_AXIS_LOCK_MIN_DELTA: f32 = 2.0;
const DEFAULT_EXCLUSIVE_AXIS_LOCK_RATIO: f32 = 4.0;
const DEFAULT_EXCLUSIVE_AXIS_LOCK_MIN_DELTA: f32 = 1.0;
const DEFAULT_WHEEL_SENSITIVITY: f32 = 0.25;
const DEFAULT_SCROLLBAR_THICKNESS: f32 = 6.0;
const DEFAULT_SCROLLBAR_INSET: f32 = 3.0;
const DEFAULT_SCROLLBAR_MIN_THUMB_LEN: f32 = 24.0;
const AUTO_REPAINT_BOUNDARY_MAX_PIXELS: u64 = 8_000_000;

/// Scrollable axes for [`ScrollView`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollAxis {
    /// Scroll horizontally.
    Horizontal,
    /// Scroll vertically.
    Vertical,
    /// Scroll horizontally and vertically.
    Both,
}

impl ScrollAxis {
    fn allows_x(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    fn allows_y(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }
}

/// Wheel direction for a scroll axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollWheelDirection {
    /// Use the normalized platform direction.
    Normal,
    /// Reverse the normalized platform direction.
    Inverted,
}

impl ScrollWheelDirection {
    fn multiplier(self) -> f32 {
        match self {
            Self::Normal => 1.0,
            Self::Inverted => -1.0,
        }
    }
}

/// Visibility policy for [`ScrollView`] scrollbars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarVisibility {
    /// Use the platform-style policy.
    ///
    /// Currently this behaves like [`ScrollbarVisibility::WhileScrolling`].
    Automatic,
    /// Show scrollbars whenever the corresponding axis can scroll.
    Always,
    /// Show scrollbars only while a wheel or trackpad gesture is active.
    WhileScrolling,
    /// Never show scrollbars.
    Never,
}

impl Default for ScrollbarVisibility {
    fn default() -> Self {
        Self::Automatic
    }
}

/// Scrollable viewport for a child view.
#[derive(Clone)]
pub struct ScrollView<V: View> {
    inner: V,
    axes: ScrollAxis,
    content_size: Option<Size>,
    wheel_scale: f32,
    horizontal_wheel_direction: ScrollWheelDirection,
    vertical_wheel_direction: ScrollWheelDirection,
    axis_lock_ratio: f32,
    axis_lock_min_delta: f32,
    exclusive_axis_lock_ratio: f32,
    exclusive_axis_lock_min_delta: f32,
    scrollbar_visibility: ScrollbarVisibility,
    scrollbar_color: Option<Color>,
}

impl<V: View> ScrollView<V> {
    /// Create a new scroll view for `inner`.
    ///
    /// # Arguments
    ///
    /// * `inner` - Child view rendered inside the scrollable content area.
    ///
    /// # Returns
    ///
    /// A vertically scrollable `ScrollView`.
    pub fn new(inner: V) -> Self {
        Self {
            inner,
            axes: ScrollAxis::Vertical,
            content_size: None,
            wheel_scale: DEFAULT_WHEEL_SENSITIVITY,
            horizontal_wheel_direction: ScrollWheelDirection::Normal,
            vertical_wheel_direction: ScrollWheelDirection::Normal,
            axis_lock_ratio: DEFAULT_AXIS_LOCK_RATIO,
            axis_lock_min_delta: DEFAULT_AXIS_LOCK_MIN_DELTA,
            exclusive_axis_lock_ratio: DEFAULT_EXCLUSIVE_AXIS_LOCK_RATIO,
            exclusive_axis_lock_min_delta: DEFAULT_EXCLUSIVE_AXIS_LOCK_MIN_DELTA,
            scrollbar_visibility: ScrollbarVisibility::default(),
            scrollbar_color: None,
        }
    }

    /// Set the scrollable axes.
    ///
    /// # Arguments
    ///
    /// * `axes` - Axes that should respond to wheel input.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn axes(mut self, axes: ScrollAxis) -> Self {
        self.axes = axes;
        self
    }

    /// Allow only horizontal scrolling.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn horizontal(mut self) -> Self {
        self.axes = ScrollAxis::Horizontal;
        self
    }

    /// Allow only vertical scrolling.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn vertical(mut self) -> Self {
        self.axes = ScrollAxis::Vertical;
        self
    }

    /// Allow horizontal and vertical scrolling.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn both_axes(mut self) -> Self {
        self.axes = ScrollAxis::Both;
        self
    }

    /// Set an explicit content size.
    ///
    /// The child is laid out with at least this size and at least the viewport
    /// size. Use this for timeline-style surfaces whose logical content extends
    /// beyond the current viewport.
    ///
    /// # Arguments
    ///
    /// * `width` - Logical content width.
    /// * `height` - Logical content height.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn content_size(mut self, width: f32, height: f32) -> Self {
        self.content_size = Some(Size::new(width.max(0.0), height.max(0.0)));
        self
    }

    /// Set the multiplier applied to wheel deltas.
    ///
    /// # Arguments
    ///
    /// * `scale` - Positive wheel delta multiplier.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn wheel_scale(mut self, scale: f32) -> Self {
        self.wheel_scale = sanitize_non_negative(scale, DEFAULT_WHEEL_SENSITIVITY);
        self
    }

    /// Set wheel sensitivity.
    ///
    /// This is the same value as [`ScrollView::wheel_scale`], named for user
    /// tuning. Values below `1.0` make scrolling less sensitive.
    ///
    /// # Arguments
    ///
    /// * `sensitivity` - Non-negative wheel sensitivity multiplier.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn wheel_sensitivity(mut self, sensitivity: f32) -> Self {
        self.wheel_scale = sanitize_non_negative(sensitivity, DEFAULT_WHEEL_SENSITIVITY);
        self
    }

    /// Set the horizontal wheel direction.
    ///
    /// # Arguments
    ///
    /// * `direction` - Direction mapping for horizontal wheel deltas.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn horizontal_wheel_direction(mut self, direction: ScrollWheelDirection) -> Self {
        self.horizontal_wheel_direction = direction;
        self
    }

    /// Set the vertical wheel direction.
    ///
    /// # Arguments
    ///
    /// * `direction` - Direction mapping for vertical wheel deltas.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn vertical_wheel_direction(mut self, direction: ScrollWheelDirection) -> Self {
        self.vertical_wheel_direction = direction;
        self
    }

    /// Set wheel axis lock parameters.
    ///
    /// When both axes are scrollable, dominant horizontal input suppresses small
    /// vertical jitter and dominant vertical input suppresses small horizontal
    /// jitter.
    ///
    /// # Arguments
    ///
    /// * `ratio` - Dominant axis ratio. Values at or below `1.0` disable locking.
    /// * `min_delta` - Minimum dominant delta before locking applies.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn wheel_axis_lock(mut self, ratio: f32, min_delta: f32) -> Self {
        self.axis_lock_ratio = sanitize_axis_lock_ratio(ratio);
        self.axis_lock_min_delta = sanitize_non_negative(min_delta, DEFAULT_AXIS_LOCK_MIN_DELTA);
        self
    }

    /// Set strict wheel axis lock parameters for single-axis scrolling.
    ///
    /// When only one axis is enabled, input on that axis is accepted only when
    /// it clearly dominates the disabled axis. Keep `min_delta` low for
    /// trackpads; strictness should usually come from `ratio`, not from a large
    /// absolute dead zone.
    ///
    /// # Arguments
    ///
    /// * `ratio` - Required dominant ratio for the enabled axis.
    /// * `min_delta` - Minimum enabled-axis delta before scrolling applies.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn exclusive_wheel_axis_lock(mut self, ratio: f32, min_delta: f32) -> Self {
        self.exclusive_axis_lock_ratio = sanitize_axis_lock_ratio(ratio);
        self.exclusive_axis_lock_min_delta =
            sanitize_non_negative(min_delta, DEFAULT_EXCLUSIVE_AXIS_LOCK_MIN_DELTA);
        self
    }

    /// Disable wheel axis locking.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn without_wheel_axis_lock(mut self) -> Self {
        self.axis_lock_ratio = 1.0;
        self.exclusive_axis_lock_ratio = 1.0;
        self.exclusive_axis_lock_min_delta = 0.0;
        self
    }

    /// Set scrollbar visibility.
    ///
    /// Scrollbars are still hidden for axes whose content fits in the viewport.
    ///
    /// # Arguments
    ///
    /// * `visibility` - Policy controlling when scrollbars are shown.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn scrollbar_visibility(mut self, visibility: ScrollbarVisibility) -> Self {
        self.scrollbar_visibility = visibility;
        self
    }

    /// Set scrollbar thumb color.
    ///
    /// # Arguments
    ///
    /// * `color` - Thumb color used for horizontal and vertical scrollbars.
    ///
    /// # Returns
    ///
    /// Updated scroll view.
    pub fn scrollbar_color(mut self, color: Color) -> Self {
        self.scrollbar_color = Some(color);
        self
    }

    /// Return the configured axes.
    ///
    /// # Returns
    ///
    /// Current scroll axis setting.
    pub fn scroll_axes(&self) -> ScrollAxis {
        self.axes
    }

    /// Return whether horizontal scrolling is enabled.
    ///
    /// # Returns
    ///
    /// `true` if the horizontal axis can scroll.
    pub fn can_scroll_horizontally(&self) -> bool {
        self.axes.allows_x()
    }

    /// Return whether vertical scrolling is enabled.
    ///
    /// # Returns
    ///
    /// `true` if the vertical axis can scroll.
    pub fn can_scroll_vertically(&self) -> bool {
        self.axes.allows_y()
    }

    /// Return the configured content size.
    ///
    /// # Returns
    ///
    /// Explicit content size, if configured.
    pub fn configured_content_size(&self) -> Option<Size> {
        self.content_size
    }

    /// Return the wheel delta scale.
    ///
    /// # Returns
    ///
    /// Wheel scale factor.
    pub fn wheel_scale_value(&self) -> f32 {
        self.wheel_scale
    }

    /// Return wheel directions.
    ///
    /// # Returns
    ///
    /// `(horizontal, vertical)` wheel directions.
    pub fn wheel_directions(&self) -> (ScrollWheelDirection, ScrollWheelDirection) {
        (
            self.horizontal_wheel_direction,
            self.vertical_wheel_direction,
        )
    }

    /// Return wheel axis lock parameters.
    ///
    /// # Returns
    ///
    /// `(ratio, min_delta)` axis lock parameters.
    pub fn wheel_axis_lock_values(&self) -> (f32, f32) {
        (self.axis_lock_ratio, self.axis_lock_min_delta)
    }

    /// Return exclusive wheel axis lock parameters.
    ///
    /// # Returns
    ///
    /// `(ratio, min_delta)` parameters used for single-axis scroll views.
    pub fn exclusive_wheel_axis_lock_values(&self) -> (f32, f32) {
        (
            self.exclusive_axis_lock_ratio,
            self.exclusive_axis_lock_min_delta,
        )
    }

    /// Return the scrollbar visibility policy.
    ///
    /// # Returns
    ///
    /// Current scrollbar visibility policy.
    pub fn scrollbar_visibility_value(&self) -> ScrollbarVisibility {
        self.scrollbar_visibility
    }

    /// Return the configured scrollbar color.
    ///
    /// # Returns
    ///
    /// Explicit scrollbar thumb color, if configured.
    pub fn scrollbar_color_value(&self) -> Option<Color> {
        self.scrollbar_color
    }
}

impl<V: View + Clone + 'static> View for ScrollView<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_children(
            self.clone(),
            ScrollViewRenderObject::<V>::from_view(self),
            vec![
                RepaintBoundary::new(self.inner.clone())
                    .max_cache_pixels(AUTO_REPAINT_BOUNDARY_MAX_PIXELS)
                    .cache_nested_boundaries(false)
                    .create_element(),
            ],
        ))
    }

    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object for [`ScrollView`].
pub struct ScrollViewRenderObject<V: View> {
    axes: ScrollAxis,
    configured_content_size: Option<Size>,
    wheel_scale: f32,
    horizontal_wheel_direction: ScrollWheelDirection,
    vertical_wheel_direction: ScrollWheelDirection,
    axis_lock_ratio: f32,
    axis_lock_min_delta: f32,
    exclusive_axis_lock_ratio: f32,
    exclusive_axis_lock_min_delta: f32,
    scrollbar_visibility: ScrollbarVisibility,
    scrollbar_color: Option<Color>,
    viewport_size: Size,
    content_size: Size,
    offset_x: f32,
    offset_y: f32,
    scrollbar_active: bool,
    _marker: PhantomData<V>,
}

impl<V: View> ScrollViewRenderObject<V> {
    /// Create a new scroll render object.
    ///
    /// # Arguments
    ///
    /// * `axes` - Scrollable axes.
    /// * `content_size` - Optional logical content size.
    /// * `wheel_scale` - Wheel delta multiplier.
    ///
    /// # Returns
    ///
    /// New render object with zero scroll offset.
    pub fn new(axes: ScrollAxis, content_size: Option<Size>, wheel_scale: f32) -> Self {
        Self {
            axes,
            configured_content_size: content_size,
            wheel_scale,
            horizontal_wheel_direction: ScrollWheelDirection::Normal,
            vertical_wheel_direction: ScrollWheelDirection::Normal,
            axis_lock_ratio: DEFAULT_AXIS_LOCK_RATIO,
            axis_lock_min_delta: DEFAULT_AXIS_LOCK_MIN_DELTA,
            exclusive_axis_lock_ratio: DEFAULT_EXCLUSIVE_AXIS_LOCK_RATIO,
            exclusive_axis_lock_min_delta: DEFAULT_EXCLUSIVE_AXIS_LOCK_MIN_DELTA,
            scrollbar_visibility: ScrollbarVisibility::default(),
            scrollbar_color: None,
            viewport_size: Size::ZERO,
            content_size: Size::ZERO,
            offset_x: 0.0,
            offset_y: 0.0,
            scrollbar_active: false,
            _marker: PhantomData,
        }
    }

    /// Create a render object from a scroll view.
    ///
    /// # Arguments
    ///
    /// * `view` - Source scroll view.
    ///
    /// # Returns
    ///
    /// New render object with zero scroll offset.
    pub fn from_view(view: &ScrollView<V>) -> Self {
        let mut render_object = Self::new(
            view.scroll_axes(),
            view.configured_content_size(),
            view.wheel_scale_value(),
        );
        (
            render_object.horizontal_wheel_direction,
            render_object.vertical_wheel_direction,
        ) = view.wheel_directions();
        (
            render_object.axis_lock_ratio,
            render_object.axis_lock_min_delta,
        ) = view.wheel_axis_lock_values();
        (
            render_object.exclusive_axis_lock_ratio,
            render_object.exclusive_axis_lock_min_delta,
        ) = view.exclusive_wheel_axis_lock_values();
        render_object.scrollbar_visibility = view.scrollbar_visibility_value();
        render_object.scrollbar_color = view.scrollbar_color_value();
        render_object
    }

    /// Return the current viewport size.
    ///
    /// # Returns
    ///
    /// Viewport size after layout.
    pub fn viewport_size(&self) -> Size {
        self.viewport_size
    }

    /// Return the current content size.
    ///
    /// # Returns
    ///
    /// Content size after layout.
    pub fn content_size(&self) -> Size {
        self.content_size
    }

    /// Return the current scroll offset.
    ///
    /// # Returns
    ///
    /// `(x, y)` scroll offset in logical pixels.
    pub fn offset(&self) -> (f32, f32) {
        (self.offset_x, self.offset_y)
    }

    fn max_offset_x(&self) -> f32 {
        (self.content_size.width - self.viewport_size.width).max(0.0)
    }

    fn max_offset_y(&self) -> f32 {
        (self.content_size.height - self.viewport_size.height).max(0.0)
    }

    fn clamp_offsets(&mut self) {
        self.offset_x = clamp_f32(self.offset_x, 0.0, self.max_offset_x());
        self.offset_y = clamp_f32(self.offset_y, 0.0, self.max_offset_y());
    }

    fn set_offset(&mut self, x: f32, y: f32) -> bool {
        let old_x = self.offset_x;
        let old_y = self.offset_y;
        self.offset_x = x;
        self.offset_y = y;
        self.clamp_offsets();
        (self.offset_x - old_x).abs() > 0.01 || (self.offset_y - old_y).abs() > 0.01
    }

    fn normalized_wheel_delta(&self, delta_x: i32, delta_y: i32) -> (f32, f32) {
        let raw_x = delta_x as f32 * self.horizontal_wheel_direction.multiplier();
        let raw_y = delta_y as f32 * self.vertical_wheel_direction.multiplier();

        match self.axes {
            ScrollAxis::Both if self.axis_lock_ratio > 1.0 => {
                let mut x = raw_x;
                let mut y = raw_y;
                let abs_x = x.abs();
                let abs_y = y.abs();
                if abs_x >= self.axis_lock_min_delta && abs_x >= abs_y * self.axis_lock_ratio {
                    y = 0.0;
                } else if abs_y >= self.axis_lock_min_delta && abs_y >= abs_x * self.axis_lock_ratio
                {
                    x = 0.0;
                }
                (
                    self.scale_wheel_axis_delta(x),
                    self.scale_wheel_axis_delta(y),
                )
            }
            ScrollAxis::Horizontal => {
                let abs_x = raw_x.abs();
                let abs_y = raw_y.abs();
                if abs_x < self.exclusive_axis_lock_min_delta
                    || abs_x < abs_y * self.exclusive_axis_lock_ratio
                {
                    return (0.0, 0.0);
                }
                (self.scale_wheel_axis_delta(raw_x), 0.0)
            }
            ScrollAxis::Vertical => {
                let abs_x = raw_x.abs();
                let abs_y = raw_y.abs();
                if abs_y < self.exclusive_axis_lock_min_delta
                    || abs_y < abs_x * self.exclusive_axis_lock_ratio
                {
                    return (0.0, 0.0);
                }
                (0.0, self.scale_wheel_axis_delta(raw_y))
            }
            ScrollAxis::Both => (
                self.scale_wheel_axis_delta(raw_x),
                self.scale_wheel_axis_delta(raw_y),
            ),
        }
    }

    fn scale_wheel_axis_delta(&self, delta: f32) -> f32 {
        delta * self.wheel_scale
    }

    fn scrollbar_color(&self) -> Color {
        self.scrollbar_color.unwrap_or_else(|| {
            ColorPalette::default()
                .secondary()
                .with_opacity(default_scrollbar_opacity())
        })
    }

    fn should_show_scrollbar(&self, scrollable: bool) -> bool {
        if !scrollable {
            return false;
        }
        match self.scrollbar_visibility {
            ScrollbarVisibility::Automatic | ScrollbarVisibility::WhileScrolling => {
                self.scrollbar_active
            }
            ScrollbarVisibility::Always => true,
            ScrollbarVisibility::Never => false,
        }
    }

    fn horizontal_scrollbar_rect(&self, origin: Point, reserve_vertical: bool) -> Option<Rect> {
        let max_offset = self.max_offset_x();
        if max_offset <= 0.0 || self.viewport_size.width <= 0.0 || self.content_size.width <= 0.0 {
            return None;
        }

        let reserved = if reserve_vertical {
            DEFAULT_SCROLLBAR_THICKNESS + DEFAULT_SCROLLBAR_INSET
        } else {
            0.0
        };
        let track_x = origin.x + DEFAULT_SCROLLBAR_INSET;
        let track_y = origin.y + self.viewport_size.height
            - DEFAULT_SCROLLBAR_INSET
            - DEFAULT_SCROLLBAR_THICKNESS;
        let track_w =
            (self.viewport_size.width - DEFAULT_SCROLLBAR_INSET * 2.0 - reserved).max(0.0);
        if track_w <= 0.0 || track_y < origin.y {
            return None;
        }

        let thumb_w = (self.viewport_size.width / self.content_size.width * track_w)
            .max(DEFAULT_SCROLLBAR_MIN_THUMB_LEN.min(track_w))
            .min(track_w);
        let travel = (track_w - thumb_w).max(0.0);
        let progress = if max_offset > 0.0 {
            self.offset_x / max_offset
        } else {
            0.0
        };
        Some(Rect::from_xywh(
            track_x + travel * progress.clamp(0.0, 1.0),
            track_y,
            thumb_w,
            DEFAULT_SCROLLBAR_THICKNESS,
        ))
    }

    fn vertical_scrollbar_rect(&self, origin: Point, reserve_horizontal: bool) -> Option<Rect> {
        let max_offset = self.max_offset_y();
        if max_offset <= 0.0 || self.viewport_size.height <= 0.0 || self.content_size.height <= 0.0
        {
            return None;
        }

        let reserved = if reserve_horizontal {
            DEFAULT_SCROLLBAR_THICKNESS + DEFAULT_SCROLLBAR_INSET
        } else {
            0.0
        };
        let track_x = origin.x + self.viewport_size.width
            - DEFAULT_SCROLLBAR_INSET
            - DEFAULT_SCROLLBAR_THICKNESS;
        let track_y = origin.y + DEFAULT_SCROLLBAR_INSET;
        let track_h =
            (self.viewport_size.height - DEFAULT_SCROLLBAR_INSET * 2.0 - reserved).max(0.0);
        if track_h <= 0.0 || track_x < origin.x {
            return None;
        }

        let thumb_h = (self.viewport_size.height / self.content_size.height * track_h)
            .max(DEFAULT_SCROLLBAR_MIN_THUMB_LEN.min(track_h))
            .min(track_h);
        let travel = (track_h - thumb_h).max(0.0);
        let progress = if max_offset > 0.0 {
            self.offset_y / max_offset
        } else {
            0.0
        };
        Some(Rect::from_xywh(
            track_x,
            track_y + travel * progress.clamp(0.0, 1.0),
            DEFAULT_SCROLLBAR_THICKNESS,
            thumb_h,
        ))
    }
}

impl<V: View + Clone + 'static> ElementRenderObject for ScrollViewRenderObject<V> {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let width = finite_viewport_axis(constraints.min_width, constraints.max_width);
        let height = finite_viewport_axis(constraints.min_height, constraints.max_height);
        self.viewport_size = Size::new(width, height);
        self.content_size = self.configured_content_size.unwrap_or(self.viewport_size);
        self.clamp_offsets();
        self.viewport_size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let viewport_width = finite_viewport_axis(constraints.min_width, constraints.max_width);
        let viewport_height = finite_viewport_axis(constraints.min_height, constraints.max_height);
        self.viewport_size = Size::new(viewport_width, viewport_height);

        let base_content = self.configured_content_size.unwrap_or(self.viewport_size);
        let content_width = base_content.width.max(viewport_width);
        let content_height = base_content.height.max(viewport_height);

        if let Some(child) = children.first_mut() {
            let child_constraints =
                LayoutConstraints::tight(content_width.max(0.0), content_height.max(0.0));
            child.set_viewport_hint(Rect::from_xywh(
                self.offset_x,
                self.offset_y,
                self.viewport_size.width,
                self.viewport_size.height,
            ));
            self.content_size = child.layout(child_constraints);
            self.content_size.width = self.content_size.width.max(content_width);
            self.content_size.height = self.content_size.height.max(content_height);
            self.clamp_offsets();
            child.set_position(Point::new(-self.offset_x, -self.offset_y));
        } else {
            self.content_size = Size::ZERO;
            self.clamp_offsets();
        }

        self.viewport_size
    }

    fn size(&self) -> Size {
        self.viewport_size
    }

    fn hit_test(&self, point: Point) -> bool {
        Rect::new(Point::ZERO, self.viewport_size).contains(point)
    }

    fn update(&mut self, new_view: &dyn View) -> crate::element::UpdateResult {
        let Some(scroll_view) = new_view.as_any().downcast_ref::<ScrollView<V>>() else {
            return crate::element::UpdateResult::Replaced;
        };

        let old_axes = self.axes;
        let old_content_size = self.configured_content_size;
        let old_wheel_scale = self.wheel_scale;
        let old_horizontal_wheel_direction = self.horizontal_wheel_direction;
        let old_vertical_wheel_direction = self.vertical_wheel_direction;
        let old_axis_lock_ratio = self.axis_lock_ratio;
        let old_axis_lock_min_delta = self.axis_lock_min_delta;
        let old_exclusive_axis_lock_ratio = self.exclusive_axis_lock_ratio;
        let old_exclusive_axis_lock_min_delta = self.exclusive_axis_lock_min_delta;
        let old_scrollbar_visibility = self.scrollbar_visibility;
        let old_scrollbar_color = self.scrollbar_color;

        self.axes = scroll_view.scroll_axes();
        self.configured_content_size = scroll_view.configured_content_size();
        self.wheel_scale = scroll_view.wheel_scale_value();
        (
            self.horizontal_wheel_direction,
            self.vertical_wheel_direction,
        ) = scroll_view.wheel_directions();
        (self.axis_lock_ratio, self.axis_lock_min_delta) = scroll_view.wheel_axis_lock_values();
        (
            self.exclusive_axis_lock_ratio,
            self.exclusive_axis_lock_min_delta,
        ) = scroll_view.exclusive_wheel_axis_lock_values();
        self.scrollbar_visibility = scroll_view.scrollbar_visibility_value();
        self.scrollbar_color = scroll_view.scrollbar_color_value();
        self.clamp_offsets();

        if self.axes != old_axes
            || self.configured_content_size != old_content_size
            || (self.wheel_scale - old_wheel_scale).abs() > 0.001
            || self.horizontal_wheel_direction != old_horizontal_wheel_direction
            || self.vertical_wheel_direction != old_vertical_wheel_direction
            || (self.axis_lock_ratio - old_axis_lock_ratio).abs() > 0.001
            || (self.axis_lock_min_delta - old_axis_lock_min_delta).abs() > 0.001
            || (self.exclusive_axis_lock_ratio - old_exclusive_axis_lock_ratio).abs() > 0.001
            || (self.exclusive_axis_lock_min_delta - old_exclusive_axis_lock_min_delta).abs()
                > 0.001
            || self.scrollbar_visibility != old_scrollbar_visibility
            || self.scrollbar_color != old_scrollbar_color
        {
            crate::element::UpdateResult::Updated
        } else {
            crate::element::UpdateResult::NoChange
        }
    }

    fn update_needs_layout(&self) -> bool {
        true
    }

    fn clip_bounds(&self, origin: Point) -> Option<(Rect, f32)> {
        Some((Rect::new(origin, self.viewport_size), 0.0))
    }

    fn captures_wheel_event(&self, event: &MouseEvent) -> bool {
        let MouseEvent::Wheel {
            delta_x, delta_y, ..
        } = *event
        else {
            return false;
        };
        let (scaled_x, scaled_y) = self.normalized_wheel_delta(delta_x, delta_y);
        (scaled_x.abs() > 0.01 && self.max_offset_x() > 0.0)
            || (scaled_y.abs() > 0.01 && self.max_offset_y() > 0.0)
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }

        let Event::Mouse(MouseEvent::Wheel {
            delta_x,
            delta_y,
            phase: wheel_phase,
            ..
        }) = event
        else {
            return false;
        };

        let old_scrollbar_active = self.scrollbar_active;
        let gesture_active = matches!(wheel_phase, WheelPhase::Started | WheelPhase::Moved);

        let (scaled_x, scaled_y) = self.normalized_wheel_delta(*delta_x, *delta_y);
        let scrollable = (self.axes.allows_x() && self.max_offset_x() > 0.0)
            || (self.axes.allows_y() && self.max_offset_y() > 0.0);
        self.scrollbar_active = gesture_active && scrollable;

        let mut next_x = self.offset_x;
        let mut next_y = self.offset_y;

        if self.axes.allows_x() {
            next_x += scaled_x;
        }
        if self.axes.allows_y() {
            next_y += scaled_y;
        }

        let offset_changed = self.set_offset(next_x, next_y);
        let scrollbar_deactivated = old_scrollbar_active && !self.scrollbar_active;
        offset_changed || scrollbar_deactivated
    }

    fn paint_overlay(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        let horizontal_scrollable = self.axes.allows_x() && self.max_offset_x() > 0.0;
        let vertical_scrollable = self.axes.allows_y() && self.max_offset_y() > 0.0;
        let show_horizontal = self.should_show_scrollbar(horizontal_scrollable);
        let show_vertical = self.should_show_scrollbar(vertical_scrollable);
        if !show_horizontal && !show_vertical {
            return false;
        }

        let color = self.scrollbar_color();
        let radius = DEFAULT_SCROLLBAR_THICKNESS * 0.5;
        if show_horizontal && let Some(rect) = self.horizontal_scrollbar_rect(origin, show_vertical)
        {
            ctx.fill_rounded_rect(rect, radius, color);
        }
        if show_vertical && let Some(rect) = self.vertical_scrollbar_rect(origin, show_horizontal) {
            ctx.fill_rounded_rect(rect, radius, color);
        }
        true
    }

    fn apply_scroll_offset(&mut self, children: &mut [Box<dyn Element>]) -> bool {
        if let Some(child) = children.first_mut() {
            child.set_position(Point::new(-self.offset_x, -self.offset_y));
            child.set_viewport_hint(Rect::from_xywh(
                self.offset_x,
                self.offset_y,
                self.viewport_size.width,
                self.viewport_size.height,
            ));
        }
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        // Container does not draw; children render inside the clipped viewport.
    }
}

fn finite_viewport_axis(min: f32, max: f32) -> f32 {
    if min.is_finite() && max.is_finite() && min == max {
        max.max(0.0)
    } else if max.is_finite() {
        max.max(min).max(0.0)
    } else if min.is_finite() {
        min.max(0.0)
    } else {
        0.0
    }
}

fn clamp_f32(value: f32, min: f32, max: f32) -> f32 {
    if !value.is_finite() {
        return min;
    }
    value.max(min).min(max.max(min))
}

fn sanitize_axis_lock_ratio(value: f32) -> f32 {
    if value.is_finite() {
        value.max(1.0)
    } else {
        DEFAULT_AXIS_LOCK_RATIO
    }
}

fn sanitize_non_negative(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        fallback
    }
}

fn default_scrollbar_opacity() -> f32 {
    0.55
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ScrollSource, WheelPhase};
    use crate::views::Text;

    #[test]
    fn wheel_updates_vertical_offset_and_clamps() {
        let mut render_object = ScrollViewRenderObject::<Text>::new(
            ScrollAxis::Vertical,
            Some(Size::new(100.0, 300.0)),
            DEFAULT_WHEEL_SENSITIVITY,
        );
        render_object.layout(LayoutConstraints::tight(100.0, 100.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: 80,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 20.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: 10_000,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 200.0));
    }

    #[test]
    fn dominant_horizontal_wheel_locks_out_vertical_jitter() {
        let view = ScrollView::new(Text::new("content"))
            .axes(ScrollAxis::Both)
            .content_size(300.0, 300.0)
            .wheel_sensitivity(1.0);
        let mut render_object = ScrollViewRenderObject::<Text>::from_view(&view);
        render_object.layout(LayoutConstraints::tight(100.0, 100.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 80,
                delta_y: 10,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (80.0, 0.0));
    }

    #[test]
    fn horizontal_wheel_direction_can_be_inverted_per_view() {
        let view = ScrollView::new(Text::new("content"))
            .axes(ScrollAxis::Both)
            .content_size(300.0, 100.0)
            .wheel_sensitivity(1.0)
            .horizontal_wheel_direction(ScrollWheelDirection::Inverted);
        let mut render_object = ScrollViewRenderObject::<Text>::from_view(&view);
        render_object.layout(LayoutConstraints::tight(100.0, 100.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: -100,
                delta_y: 0,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (100.0, 0.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 50,
                delta_y: 0,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (50.0, 0.0));
    }

    #[test]
    fn horizontal_only_ignores_vertical_wheel_delta() {
        let view = ScrollView::new(Text::new("content"))
            .horizontal()
            .wheel_sensitivity(1.0)
            .content_size(300.0, 300.0);
        let mut render_object = ScrollViewRenderObject::<Text>::from_view(&view);
        render_object.layout(LayoutConstraints::tight(100.0, 100.0));

        assert!(!render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: 80,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 0.0));

        assert!(!render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 8,
                delta_y: 40,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 0.0));

        assert!(!render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 28,
                delta_y: 8,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 0.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 4,
                delta_y: 0,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (4.0, 0.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 80,
                delta_y: 8,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (84.0, 0.0));
    }

    #[test]
    fn vertical_only_ignores_horizontal_wheel_delta() {
        let view = ScrollView::new(Text::new("content"))
            .vertical()
            .wheel_sensitivity(1.0)
            .content_size(300.0, 300.0);
        let mut render_object = ScrollViewRenderObject::<Text>::from_view(&view);
        render_object.layout(LayoutConstraints::tight(100.0, 100.0));

        assert!(!render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 80,
                delta_y: 0,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 0.0));

        assert!(!render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 40,
                delta_y: 8,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 0.0));

        assert!(!render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 8,
                delta_y: 28,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 0.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: 4,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 4.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 8,
                delta_y: 80,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 84.0));
    }

    #[test]
    fn wheel_at_scroll_edge_is_not_consumed_by_scroll_view_itself() {
        let view = ScrollView::new(Text::new("content"))
            .vertical()
            .wheel_sensitivity(1.0)
            .content_size(100.0, 200.0);
        let mut render_object = ScrollViewRenderObject::<Text>::from_view(&view);
        render_object.layout(LayoutConstraints::tight(100.0, 100.0));

        assert!(!render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: -40,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 0.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: 100,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 100.0));

        assert!(!render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: 40,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 100.0));
    }

    #[test]
    fn non_scrollable_content_does_not_consume_wheel() {
        let view = ScrollView::new(Text::new("content"))
            .vertical()
            .wheel_sensitivity(1.0)
            .content_size(100.0, 100.0);
        let mut render_object = ScrollViewRenderObject::<Text>::from_view(&view);
        render_object.layout(LayoutConstraints::tight(100.0, 100.0));

        assert!(!render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: 40,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.offset(), (0.0, 0.0));
    }

    #[test]
    fn non_matching_inner_axis_lets_outer_scroll_view_handle_wheel() {
        let inner_view = ScrollView::new(Text::new("inner"))
            .horizontal()
            .wheel_sensitivity(1.0)
            .content_size(300.0, 300.0);
        let outer_view = ScrollView::new(Text::new("outer"))
            .vertical()
            .wheel_sensitivity(1.0)
            .content_size(100.0, 300.0);
        let mut inner = ScrollViewRenderObject::<Text>::from_view(&inner_view);
        let mut outer = ScrollViewRenderObject::<Text>::from_view(&outer_view);
        inner.layout(LayoutConstraints::tight(100.0, 100.0));
        outer.layout(LayoutConstraints::tight(100.0, 100.0));

        let wheel = Event::Mouse(MouseEvent::Wheel {
            delta_x: 8,
            delta_y: 40,
            x: 10,
            y: 10,
            phase: WheelPhase::Moved,
            source: ScrollSource::Trackpad,
        });

        assert!(!inner.handle_event(&wheel, Phase::Target));
        assert_eq!(inner.offset(), (0.0, 0.0));

        assert!(outer.handle_event(&wheel, Phase::Bubble));
        assert_eq!(outer.offset(), (0.0, 40.0));
    }

    #[test]
    fn wheel_can_bubble_past_multiple_non_matching_scroll_axes() {
        let inner_view = ScrollView::new(Text::new("inner"))
            .horizontal()
            .wheel_sensitivity(1.0)
            .content_size(300.0, 100.0);
        let middle_view = ScrollView::new(Text::new("middle"))
            .horizontal()
            .wheel_sensitivity(1.0)
            .content_size(300.0, 100.0);
        let outer_view = ScrollView::new(Text::new("outer"))
            .vertical()
            .wheel_sensitivity(1.0)
            .content_size(100.0, 300.0);
        let mut inner = ScrollViewRenderObject::<Text>::from_view(&inner_view);
        let mut middle = ScrollViewRenderObject::<Text>::from_view(&middle_view);
        let mut outer = ScrollViewRenderObject::<Text>::from_view(&outer_view);
        inner.layout(LayoutConstraints::tight(100.0, 100.0));
        middle.layout(LayoutConstraints::tight(100.0, 100.0));
        outer.layout(LayoutConstraints::tight(100.0, 100.0));

        let wheel = Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: 32,
            x: 10,
            y: 10,
            phase: WheelPhase::Moved,
            source: ScrollSource::Trackpad,
        });

        assert!(!inner.handle_event(&wheel, Phase::Target));
        assert!(!middle.handle_event(&wheel, Phase::Bubble));
        assert!(outer.handle_event(&wheel, Phase::Bubble));
        assert_eq!(outer.offset(), (0.0, 32.0));
    }

    #[test]
    fn always_visible_scrollbar_paints_for_scrollable_axis() {
        let view = ScrollView::new(Text::new("content"))
            .vertical()
            .content_size(100.0, 300.0)
            .scrollbar_visibility(ScrollbarVisibility::Always);
        let mut render_object = ScrollViewRenderObject::<Text>::from_view(&view);
        render_object.layout(LayoutConstraints::tight(100.0, 100.0));

        let mut ctx = PaintContext::new();
        assert!(render_object.paint_overlay(&mut ctx, Point::ZERO));
        assert_eq!(ctx.commands().len(), 1);
    }

    #[test]
    fn scrollbar_does_not_paint_when_content_fits() {
        let view = ScrollView::new(Text::new("content"))
            .vertical()
            .content_size(100.0, 100.0)
            .scrollbar_visibility(ScrollbarVisibility::Always);
        let mut render_object = ScrollViewRenderObject::<Text>::from_view(&view);
        render_object.layout(LayoutConstraints::tight(100.0, 100.0));

        let mut ctx = PaintContext::new();
        assert!(!render_object.paint_overlay(&mut ctx, Point::ZERO));
        assert!(ctx.commands().is_empty());
    }

    #[test]
    fn while_scrolling_scrollbar_tracks_wheel_phase() {
        let view = ScrollView::new(Text::new("content"))
            .vertical()
            .content_size(100.0, 300.0)
            .scrollbar_visibility(ScrollbarVisibility::WhileScrolling);
        let mut render_object = ScrollViewRenderObject::<Text>::from_view(&view);
        render_object.layout(LayoutConstraints::tight(100.0, 100.0));

        let mut ctx = PaintContext::new();
        assert!(!render_object.paint_overlay(&mut ctx, Point::ZERO));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: 40,
                x: 10,
                y: 10,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        assert!(render_object.paint_overlay(&mut ctx, Point::ZERO));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: 0,
                x: 10,
                y: 10,
                phase: WheelPhase::Ended,
                source: ScrollSource::Trackpad,
            }),
            Phase::Target,
        ));
        let mut ended_ctx = PaintContext::new();
        assert!(!render_object.paint_overlay(&mut ended_ctx, Point::ZERO));
    }
}
