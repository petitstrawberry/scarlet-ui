//! SplitView - two-pane layout with a draggable divider.
//!
//! `SplitView` keeps its divider position in the render object. Each pane stays
//! a normal child View subtree.

use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, LayoutConstraints, RenderElement};
use crate::event::{Event, MouseButton, MouseEvent, Phase};
use crate::geometry::{Point, Rect, Size};
use crate::input_environment::InteractionMode;
use crate::renderer::PaintContext;
use crate::view::View;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;
use core::marker::PhantomData;

const DEFAULT_DIVIDER_HIT_SLOP: f32 = 6.0;
const DEFAULT_ADAPTIVE_STACK_NARROW_WIDTH: f32 = 640.0;

/// Axis used by [`SplitView`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    /// Panes are laid out left/right.
    Horizontal,
    /// Panes are laid out top/bottom.
    Vertical,
}

/// Policy controlling whether a [`SplitView`] may stack its panes adaptively.
///
/// The default is [`SplitAxisPolicy::Fixed`] for backward compatibility:
/// [`SplitView::axis`] is used exactly as configured. Applications opt into
/// [`SplitAxisPolicy::AdaptiveStack`] when a horizontal split should become a
/// vertical stack for a narrow available width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SplitAxisPolicy {
    /// Always use the configured [`SplitAxis`].
    #[default]
    Fixed,
    /// Stack a configured horizontal split vertically for narrow layouts.
    AdaptiveStack,
}

impl SplitAxisPolicy {
    /// Resolve the effective split axis for the available context.
    ///
    /// `AdaptiveStack` only changes configured horizontal splits. A configured
    /// vertical split stays vertical, and `Fixed` never changes either axis.
    ///
    /// # Arguments
    ///
    /// * `configured_axis` - Axis requested by the caller.
    /// * `interaction_mode` - Retained for source compatibility and ignored.
    /// * `available_width` - Width available to the split in logical pixels.
    /// * `narrow_width` - Width at or below which horizontal panes stack.
    ///
    /// # Returns
    ///
    /// The effective axis for layout, painting, and divider hit testing.
    pub const fn resolve(
        self,
        configured_axis: SplitAxis,
        _interaction_mode: InteractionMode,
        available_width: f32,
        narrow_width: f32,
    ) -> SplitAxis {
        self.resolve_for_width(configured_axis, available_width, narrow_width)
    }

    /// Resolve the effective split axis from actual container width.
    ///
    /// # Arguments
    ///
    /// * `configured_axis` - Axis requested by the caller.
    /// * `available_width` - Width available to the split in logical pixels.
    /// * `narrow_width` - Width at or below which horizontal panes stack.
    ///
    /// # Returns
    ///
    /// The effective split axis without consulting posture or input devices.
    pub const fn resolve_for_width(
        self,
        configured_axis: SplitAxis,
        available_width: f32,
        narrow_width: f32,
    ) -> SplitAxis {
        match (self, configured_axis) {
            (_, SplitAxis::Vertical) => SplitAxis::Vertical,
            (Self::Fixed, SplitAxis::Horizontal) => SplitAxis::Horizontal,
            (Self::AdaptiveStack, SplitAxis::Horizontal) if available_width <= narrow_width => {
                SplitAxis::Vertical
            }
            (Self::AdaptiveStack, SplitAxis::Horizontal) => SplitAxis::Horizontal,
        }
    }
}

/// Two-pane layout with a draggable divider.
#[derive(Clone)]
pub struct SplitView<A: View, B: View> {
    first: A,
    second: B,
    axis: SplitAxis,
    axis_policy: SplitAxisPolicy,
    adaptive_stack_narrow_width: f32,
    fraction: f32,
    min_first: f32,
    min_second: f32,
    divider_thickness: f32,
    divider_hit_slop: f32,
    divider_color: Color,
    active_divider_color: Color,
}

impl<A: View, B: View> SplitView<A, B> {
    /// Create a horizontal split view.
    ///
    /// # Arguments
    ///
    /// * `first` - Left/top pane.
    /// * `second` - Right/bottom pane.
    ///
    /// # Returns
    ///
    /// New split view with an even split.
    pub fn new(first: A, second: B) -> Self {
        let palette = ColorPalette::default();
        Self {
            first,
            second,
            axis: SplitAxis::Horizontal,
            axis_policy: SplitAxisPolicy::Fixed,
            adaptive_stack_narrow_width: DEFAULT_ADAPTIVE_STACK_NARROW_WIDTH,
            fraction: 0.5,
            min_first: 0.0,
            min_second: 0.0,
            divider_thickness: 1.0,
            divider_hit_slop: DEFAULT_DIVIDER_HIT_SLOP,
            divider_color: palette.divider(),
            active_divider_color: crate::views::style::focus_highlight(&palette),
        }
    }

    /// Set the split axis.
    ///
    /// # Arguments
    ///
    /// * `axis` - Horizontal or vertical split direction.
    ///
    /// # Returns
    ///
    /// Updated split view.
    pub fn axis(mut self, axis: SplitAxis) -> Self {
        self.axis = axis;
        self.axis_policy = SplitAxisPolicy::Fixed;
        self
    }

    /// Set the policy that controls adaptive stacking.
    ///
    /// [`SplitAxisPolicy::Fixed`] is the default and preserves the configured
    /// axis. Select [`SplitAxisPolicy::AdaptiveStack`] after setting a
    /// horizontal axis to stack panes vertically in touch or narrow contexts.
    /// Calling [`SplitView::axis`] afterwards restores the fixed policy.
    ///
    /// # Arguments
    ///
    /// * `policy` - Fixed or adaptive split-axis policy.
    ///
    /// # Returns
    ///
    /// Updated split view.
    pub fn axis_policy(mut self, policy: SplitAxisPolicy) -> Self {
        self.axis_policy = policy;
        self
    }

    /// Set the width threshold used by adaptive stacking.
    ///
    /// This threshold applies only to [`SplitAxisPolicy::AdaptiveStack`]. At
    /// or below it, a configured horizontal split stacks vertically even when
    /// the interaction mode is not touch. The default threshold is 640 logical
    /// pixels.
    ///
    /// # Arguments
    ///
    /// * `width` - Non-negative narrow-layout threshold in logical pixels.
    ///
    /// # Returns
    ///
    /// Updated split view.
    pub fn adaptive_stack_narrow_width(mut self, width: f32) -> Self {
        self.adaptive_stack_narrow_width =
            sanitize_non_negative(width, DEFAULT_ADAPTIVE_STACK_NARROW_WIDTH);
        self
    }

    /// Set the initial divider fraction.
    ///
    /// # Arguments
    ///
    /// * `fraction` - Ratio of the first pane along the split axis.
    ///
    /// # Returns
    ///
    /// Updated split view.
    pub fn fraction(mut self, fraction: f32) -> Self {
        self.fraction = clamp_fraction(fraction);
        self
    }

    /// Set the minimum first pane extent.
    ///
    /// # Arguments
    ///
    /// * `min_first` - Minimum width/height for the first pane.
    ///
    /// # Returns
    ///
    /// Updated split view.
    pub fn min_first(mut self, min_first: f32) -> Self {
        self.min_first = min_first.max(0.0);
        self
    }

    /// Set the minimum second pane extent.
    ///
    /// # Arguments
    ///
    /// * `min_second` - Minimum width/height for the second pane.
    ///
    /// # Returns
    ///
    /// Updated split view.
    pub fn min_second(mut self, min_second: f32) -> Self {
        self.min_second = min_second.max(0.0);
        self
    }

    /// Set divider thickness.
    ///
    /// # Arguments
    ///
    /// * `thickness` - Divider thickness in logical pixels.
    ///
    /// # Returns
    ///
    /// Updated split view.
    pub fn divider_thickness(mut self, thickness: f32) -> Self {
        self.divider_thickness = thickness.max(1.0);
        self
    }

    /// Set extra hit area around the divider.
    ///
    /// The visible divider keeps its configured thickness. This value expands
    /// pointer hit testing on both sides so thin dividers remain easy to drag.
    ///
    /// # Arguments
    ///
    /// * `slop` - Extra hit area in logical pixels on each side.
    ///
    /// # Returns
    ///
    /// Updated split view.
    pub fn divider_hit_slop(mut self, slop: f32) -> Self {
        self.divider_hit_slop = sanitize_non_negative(slop, DEFAULT_DIVIDER_HIT_SLOP);
        self
    }

    /// Set divider colors.
    ///
    /// # Arguments
    ///
    /// * `normal` - Normal divider color.
    /// * `active` - Hovered or dragged divider color.
    ///
    /// # Returns
    ///
    /// Updated split view.
    pub fn divider_colors(mut self, normal: Color, active: Color) -> Self {
        self.divider_color = normal;
        self.active_divider_color = active;
        self
    }

    /// Return the configured split axis.
    ///
    /// # Returns
    ///
    /// Axis requested by the caller before adaptive resolution.
    pub fn split_axis(&self) -> SplitAxis {
        self.axis
    }

    /// Return the configured split-axis policy.
    ///
    /// # Returns
    ///
    /// The fixed or adaptive stacking policy.
    pub fn split_axis_policy(&self) -> SplitAxisPolicy {
        self.axis_policy
    }

    /// Return the adaptive stacking width threshold.
    ///
    /// # Returns
    ///
    /// The narrow-layout threshold in logical pixels.
    pub fn split_adaptive_stack_narrow_width(&self) -> f32 {
        self.adaptive_stack_narrow_width
    }

    /// Return the configured divider fraction.
    ///
    /// # Returns
    ///
    /// Initial divider fraction.
    pub fn split_fraction(&self) -> f32 {
        self.fraction
    }

    /// Return minimum pane extents.
    ///
    /// # Returns
    ///
    /// `(first, second)` minimum extents.
    pub fn minimum_extents(&self) -> (f32, f32) {
        (self.min_first, self.min_second)
    }

    /// Return divider thickness.
    ///
    /// # Returns
    ///
    /// Divider thickness in logical pixels.
    pub fn split_divider_thickness(&self) -> f32 {
        self.divider_thickness
    }

    /// Return divider hit slop.
    ///
    /// # Returns
    ///
    /// Extra hit area in logical pixels on each side of the divider.
    pub fn split_divider_hit_slop(&self) -> f32 {
        self.divider_hit_slop
    }

    /// Return divider colors.
    ///
    /// # Returns
    ///
    /// `(normal, active)` divider colors.
    pub fn split_divider_colors(&self) -> (Color, Color) {
        (self.divider_color, self.active_divider_color)
    }
}

impl<A: View + Clone + 'static, B: View + Clone + 'static> View for SplitView<A, B> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children(
            self.clone(),
            SplitViewRenderObject::<A, B>::from_view,
            |view| vec![view.first.clone_view(), view.second.clone_view()],
        ))
    }

    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        let mut listenables = self.first.listenables();
        listenables.extend(self.second.listenables());
        listenables
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object for [`SplitView`].
pub struct SplitViewRenderObject<A: View, B: View> {
    configured_axis: SplitAxis,
    axis: SplitAxis,
    axis_policy: SplitAxisPolicy,
    adaptive_stack_narrow_width: f32,
    fraction: f32,
    min_first: f32,
    min_second: f32,
    divider_thickness: f32,
    divider_hit_slop: f32,
    divider_color: Color,
    active_divider_color: Color,
    size: Size,
    first_extent: f32,
    divider_rect: Rect,
    hovered: bool,
    dragging: bool,
    drag_pointer_offset: f32,
    _marker: PhantomData<(A, B)>,
}

impl<A: View, B: View> SplitViewRenderObject<A, B> {
    /// Create a render object from a split view.
    ///
    /// # Arguments
    ///
    /// * `view` - Source split view.
    ///
    /// # Returns
    ///
    /// New render object.
    pub fn from_view(view: &SplitView<A, B>) -> Self {
        Self {
            configured_axis: view.split_axis(),
            axis: view.split_axis(),
            axis_policy: view.split_axis_policy(),
            adaptive_stack_narrow_width: view.split_adaptive_stack_narrow_width(),
            fraction: view.split_fraction(),
            min_first: view.minimum_extents().0,
            min_second: view.minimum_extents().1,
            divider_thickness: view.split_divider_thickness(),
            divider_hit_slop: view.split_divider_hit_slop(),
            divider_color: view.split_divider_colors().0,
            active_divider_color: view.split_divider_colors().1,
            size: Size::ZERO,
            first_extent: 0.0,
            divider_rect: Rect::zero(),
            hovered: false,
            dragging: false,
            drag_pointer_offset: 0.0,
            _marker: PhantomData,
        }
    }

    /// Return the current first pane extent.
    ///
    /// # Returns
    ///
    /// First pane width or height after layout.
    pub fn first_extent(&self) -> f32 {
        self.first_extent
    }

    /// Return the effective split axis from the latest layout.
    ///
    /// # Returns
    ///
    /// The axis used to lay out panes and place the divider.
    pub fn effective_axis(&self) -> SplitAxis {
        self.axis
    }

    /// Return whether the divider is being dragged.
    ///
    /// # Returns
    ///
    /// `true` while the divider drag is active.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Return whether the divider hit area is hovered.
    ///
    /// # Returns
    ///
    /// `true` while the pointer is over the divider hit area.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    fn split_extent(&self) -> f32 {
        match self.axis {
            SplitAxis::Horizontal => self.size.width,
            SplitAxis::Vertical => self.size.height,
        }
    }

    fn resolve_axis(&mut self) {
        self.axis = self.axis_policy.resolve_for_width(
            self.configured_axis,
            self.size.width,
            self.adaptive_stack_narrow_width,
        );
    }

    fn constrained_first_extent(&self, requested: f32) -> f32 {
        let max_first = (self.split_extent() - self.divider_thickness - self.min_second).max(0.0);
        requested.max(self.min_first.min(max_first)).min(max_first)
    }

    fn update_fraction_from_point(&mut self, point: Point) -> bool {
        let requested = match self.axis {
            SplitAxis::Horizontal => {
                point.x - self.drag_pointer_offset - self.divider_thickness / 2.0
            }
            SplitAxis::Vertical => {
                point.y - self.drag_pointer_offset - self.divider_thickness / 2.0
            }
        };
        let first_extent = self.constrained_first_extent(requested);
        let old = self.first_extent;
        self.first_extent = first_extent;
        if self.split_extent() > self.divider_thickness {
            self.fraction = first_extent / (self.split_extent() - self.divider_thickness);
        }
        (self.first_extent - old).abs() > 0.01
    }

    fn point_in_divider(&self, point: Point) -> bool {
        self.divider_hit_rect().contains(point)
    }

    fn divider_center_axis_position(&self) -> f32 {
        self.first_extent + self.divider_thickness / 2.0
    }

    fn point_axis_position(&self, point: Point) -> f32 {
        match self.axis {
            SplitAxis::Horizontal => point.x,
            SplitAxis::Vertical => point.y,
        }
    }

    fn divider_hit_rect(&self) -> Rect {
        match self.axis {
            SplitAxis::Horizontal => Rect::from_xywh(
                self.divider_rect.origin.x - self.divider_hit_slop,
                self.divider_rect.origin.y,
                self.divider_rect.size.width + self.divider_hit_slop * 2.0,
                self.divider_rect.size.height,
            ),
            SplitAxis::Vertical => Rect::from_xywh(
                self.divider_rect.origin.x,
                self.divider_rect.origin.y - self.divider_hit_slop,
                self.divider_rect.size.width,
                self.divider_rect.size.height + self.divider_hit_slop * 2.0,
            ),
        }
    }

    fn update_divider_rect(&mut self) {
        self.divider_rect = match self.axis {
            SplitAxis::Horizontal => Rect::from_xywh(
                self.first_extent,
                0.0,
                self.divider_thickness,
                self.size.height,
            ),
            SplitAxis::Vertical => Rect::from_xywh(
                0.0,
                self.first_extent,
                self.size.width,
                self.divider_thickness,
            ),
        };
    }
}

impl<A: View + Clone + 'static, B: View + Clone + 'static> ElementRenderObject
    for SplitViewRenderObject<A, B>
{
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = Size::new(
            finite_split_axis(constraints.min_width, constraints.max_width),
            finite_split_axis(constraints.min_height, constraints.max_height),
        );
        self.resolve_axis();
        let extent = (self.split_extent() - self.divider_thickness).max(0.0);
        self.first_extent = self.constrained_first_extent(extent * self.fraction);
        self.update_divider_rect();
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        self.layout(constraints);

        let second_extent =
            (self.split_extent() - self.first_extent - self.divider_thickness).max(0.0);

        if let Some(first) = children.first_mut() {
            let constraints = match self.axis {
                SplitAxis::Horizontal => {
                    LayoutConstraints::tight(self.first_extent, self.size.height)
                }
                SplitAxis::Vertical => LayoutConstraints::tight(self.size.width, self.first_extent),
            };
            first.layout(constraints);
            first.set_position(Point::ZERO);
        }

        if let Some(second) = children.get_mut(1) {
            let constraints = match self.axis {
                SplitAxis::Horizontal => LayoutConstraints::tight(second_extent, self.size.height),
                SplitAxis::Vertical => LayoutConstraints::tight(self.size.width, second_extent),
            };
            let position = match self.axis {
                SplitAxis::Horizontal => {
                    Point::new(self.first_extent + self.divider_thickness, 0.0)
                }
                SplitAxis::Vertical => Point::new(0.0, self.first_extent + self.divider_thickness),
            };
            second.layout(constraints);
            second.set_position(position);
        }

        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: Point) -> bool {
        Rect::new(Point::ZERO, self.size).contains(point)
    }

    fn update(&mut self, new_view: &dyn View) -> crate::element::UpdateResult {
        let Some(split_view) = new_view.as_any().downcast_ref::<SplitView<A, B>>() else {
            return crate::element::UpdateResult::Replaced;
        };

        let old_configured_axis = self.configured_axis;
        let old_axis_policy = self.axis_policy;
        let old_adaptive_stack_narrow_width = self.adaptive_stack_narrow_width;
        let old_min_first = self.min_first;
        let old_min_second = self.min_second;
        let old_divider_thickness = self.divider_thickness;
        let old_divider_hit_slop = self.divider_hit_slop;
        let old_divider_color = self.divider_color;
        let old_active_divider_color = self.active_divider_color;

        self.configured_axis = split_view.split_axis();
        self.axis_policy = split_view.split_axis_policy();
        self.adaptive_stack_narrow_width = split_view.split_adaptive_stack_narrow_width();
        if !self.dragging {
            self.fraction = split_view.split_fraction();
        }
        (self.min_first, self.min_second) = split_view.minimum_extents();
        self.divider_thickness = split_view.split_divider_thickness();
        self.divider_hit_slop = split_view.split_divider_hit_slop();
        (self.divider_color, self.active_divider_color) = split_view.split_divider_colors();

        if self.configured_axis != old_configured_axis
            || self.axis_policy != old_axis_policy
            || (self.adaptive_stack_narrow_width - old_adaptive_stack_narrow_width).abs() > 0.001
            || (self.min_first - old_min_first).abs() > 0.001
            || (self.min_second - old_min_second).abs() > 0.001
            || (self.divider_thickness - old_divider_thickness).abs() > 0.001
            || (self.divider_hit_slop - old_divider_hit_slop).abs() > 0.001
            || self.divider_color != old_divider_color
            || self.active_divider_color != old_active_divider_color
        {
            crate::element::UpdateResult::Updated
        } else {
            crate::element::UpdateResult::NoChange
        }
    }

    fn update_needs_layout(&self) -> bool {
        true
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }

        let Event::Mouse(mouse_event) = event else {
            return false;
        };

        match *mouse_event {
            MouseEvent::Moved { x, y } => {
                let point = Point::new(x as f32, y as f32);
                if self.dragging {
                    return self.update_fraction_from_point(point);
                }
                let hovered = self.point_in_divider(point);
                let changed = hovered != self.hovered;
                self.hovered = hovered;
                changed
            }
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                ..
            } => {
                let point = Point::new(x as f32, y as f32);
                if self.point_in_divider(point) {
                    self.dragging = true;
                    self.hovered = true;
                    self.drag_pointer_offset =
                        self.point_axis_position(point) - self.divider_center_axis_position();
                    return true;
                }
                false
            }
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                ..
            } => {
                if self.dragging || self.hovered {
                    self.dragging = false;
                    self.hovered = false;
                    self.drag_pointer_offset = 0.0;
                    return true;
                }
                false
            }
            MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                ..
            } => {
                if self.dragging || self.hovered {
                    self.dragging = false;
                    self.hovered = false;
                    self.drag_pointer_offset = 0.0;
                    return true;
                }
                false
            }
            MouseEvent::Exited { .. } => {
                if self.hovered && !self.dragging {
                    self.hovered = false;
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let rect = Rect::from_xywh(
            origin.x + self.divider_rect.origin.x,
            origin.y + self.divider_rect.origin.y,
            self.divider_rect.size.width,
            self.divider_rect.size.height,
        );
        let color = if self.hovered || self.dragging {
            self.active_divider_color
        } else {
            self.divider_color
        };
        ctx.fill_rect(rect, color);
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        // PaintCommand path handles divider drawing.
    }
}

fn finite_split_axis(min: f32, max: f32) -> f32 {
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

fn clamp_fraction(fraction: f32) -> f32 {
    if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

fn sanitize_non_negative(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::Text;

    #[test]
    fn horizontal_layout_respects_fraction() {
        let mut render_object = SplitViewRenderObject::<Text, Text>::from_view(
            &SplitView::new(Text::new("A"), Text::new("B"))
                .fraction(0.25)
                .divider_thickness(2.0),
        );

        render_object.layout(LayoutConstraints::tight(402.0, 100.0));

        assert_eq!(render_object.first_extent(), 100.0);
    }

    #[test]
    fn adaptive_stack_policy_resolves_only_from_available_width() {
        assert_eq!(
            SplitAxisPolicy::AdaptiveStack.resolve(
                SplitAxis::Horizontal,
                InteractionMode::Touch,
                1200.0,
                640.0,
            ),
            SplitAxis::Horizontal
        );
        assert_eq!(
            SplitAxisPolicy::AdaptiveStack.resolve(
                SplitAxis::Horizontal,
                InteractionMode::Pointer,
                640.0,
                640.0,
            ),
            SplitAxis::Vertical
        );
        assert_eq!(
            SplitAxisPolicy::AdaptiveStack.resolve(
                SplitAxis::Horizontal,
                InteractionMode::Pointer,
                900.0,
                640.0,
            ),
            SplitAxis::Horizontal
        );
    }

    #[test]
    fn fixed_policy_never_overrides_the_configured_axis() {
        assert_eq!(
            SplitAxisPolicy::Fixed.resolve(
                SplitAxis::Horizontal,
                InteractionMode::Touch,
                320.0,
                640.0,
            ),
            SplitAxis::Horizontal
        );
        assert_eq!(
            SplitAxisPolicy::Fixed.resolve(
                SplitAxis::Vertical,
                InteractionMode::Pointer,
                1200.0,
                640.0,
            ),
            SplitAxis::Vertical
        );
    }

    #[test]
    fn adaptive_narrow_layout_stacks_panes_and_uses_a_horizontal_divider_hit_area() {
        let mut render_object = SplitViewRenderObject::<Text, Text>::from_view(
            &SplitView::new(Text::new("A"), Text::new("B"))
                .axis_policy(SplitAxisPolicy::AdaptiveStack)
                .divider_thickness(2.0),
        );

        render_object.layout(LayoutConstraints::tight(402.0, 100.0));

        assert_eq!(render_object.effective_axis(), SplitAxis::Vertical);
        assert_eq!(render_object.first_extent(), 49.0);
        assert_eq!(
            render_object.divider_rect,
            Rect::from_xywh(0.0, 49.0, 402.0, 2.0)
        );
        assert!(render_object.point_in_divider(Point::new(20.0, 45.0)));
        assert!(!render_object.point_in_divider(Point::new(20.0, 40.0)));
    }

    #[test]
    fn drag_updates_first_extent() {
        let mut render_object = SplitViewRenderObject::<Text, Text>::from_view(
            &SplitView::new(Text::new("A"), Text::new("B")).divider_thickness(2.0),
        );
        render_object.layout(LayoutConstraints::tight(402.0, 100.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 201,
                y: 20,
                click_count: 1,
            }),
            Phase::Target,
        ));
        assert!(render_object.is_dragging());

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Moved { x: 251, y: 20 }),
            Phase::Target,
        ));
        assert_eq!(render_object.first_extent(), 250.0);
    }

    #[test]
    fn divider_hit_area_is_wider_than_visible_divider_without_jumping() {
        let mut render_object = SplitViewRenderObject::<Text, Text>::from_view(
            &SplitView::new(Text::new("A"), Text::new("B")).divider_thickness(2.0),
        );
        render_object.layout(LayoutConstraints::tight(402.0, 100.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 195,
                y: 20,
                click_count: 1,
            }),
            Phase::Target,
        ));
        assert!(render_object.is_dragging());
        assert_eq!(render_object.first_extent(), 200.0);

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::Moved { x: 245, y: 20 }),
            Phase::Target,
        ));
        assert_eq!(render_object.first_extent(), 250.0);
    }

    #[test]
    fn releasing_drag_clears_active_state() {
        let mut render_object = SplitViewRenderObject::<Text, Text>::from_view(
            &SplitView::new(Text::new("A"), Text::new("B")).divider_thickness(2.0),
        );
        render_object.layout(LayoutConstraints::tight(402.0, 100.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 201,
                y: 20,
                click_count: 1,
            }),
            Phase::Target,
        ));
        assert!(render_object.is_dragging());
        assert!(render_object.is_hovered());

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 251,
                y: 20,
                click_count: 1,
            }),
            Phase::Target,
        ));
        assert!(!render_object.is_dragging());
        assert!(!render_object.is_hovered());
    }
}
