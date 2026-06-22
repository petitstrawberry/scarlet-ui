//! SplitView - two-pane layout with a draggable divider.
//!
//! `SplitView` keeps its divider position in the render object. Each pane stays
//! a normal child View subtree.

use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, LayoutConstraints, RenderElement};
use crate::event::{Event, MouseButton, MouseEvent, Phase};
use crate::geometry::{Point, Rect, Size};
use crate::renderer::PaintContext;
use crate::view::View;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;
use core::marker::PhantomData;

const DEFAULT_DIVIDER_HIT_SLOP: f32 = 6.0;

/// Axis used by [`SplitView`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    /// Panes are laid out left/right.
    Horizontal,
    /// Panes are laid out top/bottom.
    Vertical,
}

/// Two-pane layout with a draggable divider.
#[derive(Clone)]
pub struct SplitView<A: View, B: View> {
    first: A,
    second: B,
    axis: SplitAxis,
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
            fraction: 0.5,
            min_first: 0.0,
            min_second: 0.0,
            divider_thickness: 1.0,
            divider_hit_slop: DEFAULT_DIVIDER_HIT_SLOP,
            divider_color: palette.border(),
            active_divider_color: palette.accent(),
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

    /// Return the split axis.
    ///
    /// # Returns
    ///
    /// Current split axis.
    pub fn split_axis(&self) -> SplitAxis {
        self.axis
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
        Box::new(RenderElement::with_children(
            self.clone(),
            SplitViewRenderObject::<A, B>::from_view(self),
            vec![self.first.create_element(), self.second.create_element()],
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
    axis: SplitAxis,
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
            axis: view.split_axis(),
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

        let old_axis = self.axis;
        let old_min_first = self.min_first;
        let old_min_second = self.min_second;
        let old_divider_thickness = self.divider_thickness;
        let old_divider_hit_slop = self.divider_hit_slop;
        let old_divider_color = self.divider_color;
        let old_active_divider_color = self.active_divider_color;

        self.axis = split_view.split_axis();
        if !self.dragging {
            self.fraction = split_view.split_fraction();
        }
        (self.min_first, self.min_second) = split_view.minimum_extents();
        self.divider_thickness = split_view.split_divider_thickness();
        self.divider_hit_slop = split_view.split_divider_hit_slop();
        (self.divider_color, self.active_divider_color) = split_view.split_divider_colors();

        if self.axis != old_axis
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
