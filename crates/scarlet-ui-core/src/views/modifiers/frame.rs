//! Frame View Modifier
//!
//! Constrains a child view to a specific size.

use crate::animation::{Animated, Animation, current_transaction};
use crate::element::LayoutConstraints;
use crate::element::{Element, ElementRenderObject, RenderElement, UpdateResult};
use crate::geometry::{Point, Size};
use crate::view::View;
use alloc::boxed::Box;
use alloc::vec;
use core::any::Any;
use core::time::Duration;

/// Frame view modifier - constrains a child to a specific size
#[derive(Clone)]
pub struct Frame<V: View> {
    inner: V,
    width: Option<f32>,
    height: Option<f32>,
    min_width: f32,
    min_height: f32,
    max_width: f32,
    max_height: f32,
}

impl<V: View> Frame<V> {
    /// Create a new Frame modifier with fixed size
    pub fn new(inner: V, width: f32, height: f32) -> Self {
        let (min_width, max_width) = if width.is_finite() {
            (width, width)
        } else {
            (0.0, f32::INFINITY)
        };
        let (min_height, max_height) = if height.is_finite() {
            (height, height)
        } else {
            (0.0, f32::INFINITY)
        };
        Self {
            inner,
            width: Some(width),
            height: Some(height),
            min_width,
            min_height,
            max_width,
            max_height,
        }
    }

    /// Create a new Frame modifier with width only
    pub fn width(inner: V, width: f32) -> Self {
        Self {
            inner,
            width: Some(width),
            height: None,
            min_width: if width.is_finite() { width } else { 0.0 },
            min_height: 0.0,
            max_width: explicit_dimension(width),
            max_height: f32::INFINITY,
        }
    }

    /// Create a new Frame modifier with height only
    pub fn height(inner: V, height: f32) -> Self {
        Self {
            inner,
            width: None,
            height: Some(height),
            min_width: 0.0,
            min_height: if height.is_finite() { height } else { 0.0 },
            max_width: f32::INFINITY,
            max_height: explicit_dimension(height),
        }
    }

    /// Set minimum width
    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width;
        self
    }

    /// Set minimum height
    pub fn min_height(mut self, min_height: f32) -> Self {
        self.min_height = min_height;
        self
    }

    /// Set maximum width
    pub fn max_width(mut self, max_width: f32) -> Self {
        self.max_width = max_width;
        self
    }

    /// Set maximum height
    pub fn max_height(mut self, max_height: f32) -> Self {
        self.max_height = max_height;
        self
    }

    /// Get the inner view
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Get the width constraint
    pub fn width_value(&self) -> Option<f32> {
        self.width
    }

    /// Get the height constraint
    pub fn height_value(&self) -> Option<f32> {
        self.height
    }
}

impl<V: View + Clone> View for Frame<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| {
                FrameRenderObject::new(
                    view.width,
                    view.height,
                    view.min_width,
                    view.min_height,
                    view.max_width,
                    view.max_height,
                )
            },
            |render, view| render.update_frame(view, current_transaction().effective_animation()),
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Frame RenderObject
pub struct FrameRenderObject {
    values: FrameValues,
    // Keep transition state out of static frames. Independent tracks preserve
    // an unchanged axis's timeline when another property changes midflight.
    animation: Option<Box<FrameAnimationTracks>>,
    size: Size,
}

#[derive(Clone, Copy, PartialEq)]
struct FrameValues {
    width: Option<f32>,
    height: Option<f32>,
    min_width: f32,
    min_height: f32,
    max_width: f32,
    max_height: f32,
}

impl FrameValues {
    fn new(
        width: Option<f32>,
        height: Option<f32>,
        min_width: f32,
        min_height: f32,
        max_width: f32,
        max_height: f32,
    ) -> Self {
        Self {
            width: width.map(explicit_dimension),
            height: height.map(explicit_dimension),
            min_width: bound_dimension(min_width, 0.0),
            min_height: bound_dimension(min_height, 0.0),
            max_width: bound_dimension(max_width, f32::INFINITY),
            max_height: bound_dimension(max_height, f32::INFINITY),
        }
        .nonnegative()
    }

    fn nonnegative(self) -> Self {
        // Springs may overshoot below zero. Keep that motion internal to the
        // tracks: layout only sees valid lengths, preserving None and +infinity.
        Self {
            width: self.width.map(|value| value.max(0.0)),
            height: self.height.map(|value| value.max(0.0)),
            min_width: self.min_width.max(0.0),
            min_height: self.min_height.max(0.0),
            max_width: self.max_width.max(0.0),
            max_height: self.max_height.max(0.0),
        }
    }
}

struct FrameAnimationTracks {
    width: Option<Animated<f32>>,
    height: Option<Animated<f32>>,
    min_width: Animated<f32>,
    min_height: Animated<f32>,
    max_width: Animated<f32>,
    max_height: Animated<f32>,
}

impl FrameAnimationTracks {
    fn new(values: FrameValues) -> Self {
        Self {
            width: values.width.map(Animated::new),
            height: values.height.map(Animated::new),
            min_width: Animated::new(values.min_width),
            min_height: Animated::new(values.min_height),
            max_width: Animated::new(values.max_width),
            max_height: Animated::new(values.max_height),
        }
    }

    fn values(&self) -> FrameValues {
        FrameValues {
            width: self.width.as_ref().map(Animated::value),
            height: self.height.as_ref().map(Animated::value),
            min_width: self.min_width.value(),
            min_height: self.min_height.value(),
            max_width: self.max_width.value(),
            max_height: self.max_height.value(),
        }
        .nonnegative()
    }

    fn retarget(&mut self, target: FrameValues, animation: Option<Animation>) -> bool {
        retarget_optional_dimension(&mut self.width, target.width, animation)
            | retarget_optional_dimension(&mut self.height, target.height, animation)
            | retarget_dimension(&mut self.min_width, target.min_width, animation)
            | retarget_dimension(&mut self.min_height, target.min_height, animation)
            | retarget_dimension(&mut self.max_width, target.max_width, animation)
            | retarget_dimension(&mut self.max_height, target.max_height, animation)
    }

    fn is_active(&self) -> bool {
        self.width.as_ref().is_some_and(Animated::is_active)
            || self.height.as_ref().is_some_and(Animated::is_active)
            || self.min_width.is_active()
            || self.min_height.is_active()
            || self.max_width.is_active()
            || self.max_height.is_active()
    }

    fn advance(&mut self, delta: Duration) -> bool {
        self.width
            .as_mut()
            .is_some_and(|value| value.advance(delta))
            | self
                .height
                .as_mut()
                .is_some_and(|value| value.advance(delta))
            | self.min_width.advance(delta)
            | self.min_height.advance(delta)
            | self.max_width.advance(delta)
            | self.max_height.advance(delta)
    }

    fn finish(&mut self) -> bool {
        self.width.as_mut().is_some_and(Animated::finish)
            | self.height.as_mut().is_some_and(Animated::finish)
            | self.min_width.finish()
            | self.min_height.finish()
            | self.max_width.finish()
            | self.max_height.finish()
    }
}

// Nonfinite explicit dimensions mean "fill the offered space". Canonicalize
// NaN so repeated updates compare equal and never feed it into interpolation.
fn explicit_dimension(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        f32::INFINITY
    }
}

fn bound_dimension(value: f32, fallback: f32) -> f32 {
    if value.is_nan() { fallback } else { value }
}

fn retarget_dimension(
    value: &mut Animated<f32>,
    target: f32,
    animation: Option<Animation>,
) -> bool {
    if *value.target() == target {
        return false;
    }
    let current = value.value();
    let displayed = current.max(0.0);
    if displayed != current {
        // An interrupted spring starts from the visible edge, not its hidden
        // negative overshoot. Equal targets above deliberately retain timing.
        *value = Animated::new(displayed);
    }
    let animation = if displayed.is_finite() && target.is_finite() {
        animation
    } else {
        None
    };
    value.retarget(target, animation);
    // Rebasing can already equal the new target (zero); stopping the previous
    // transition is still a change even if retarget itself becomes a no-op.
    true
}

fn retarget_optional_dimension(
    value: &mut Option<Animated<f32>>,
    target: Option<f32>,
    animation: Option<Animation>,
) -> bool {
    match (value.as_mut(), target.map(explicit_dimension)) {
        (Some(value), Some(target)) => retarget_dimension(value, target, animation),
        (None, None) => false,
        (_, target) => {
            // Adding/removing an explicit constraint has no numeric starting
            // point. Snap that field without interrupting the other dimensions.
            *value = target.map(Animated::new);
            true
        }
    }
}

impl FrameRenderObject {
    /// Create a new FrameRenderObject
    pub fn new(
        width: Option<f32>,
        height: Option<f32>,
        min_width: f32,
        min_height: f32,
        max_width: f32,
        max_height: f32,
    ) -> Self {
        Self {
            values: FrameValues::new(width, height, min_width, min_height, max_width, max_height),
            animation: None,
            size: Size::ZERO,
        }
    }

    fn update_frame<V: View>(
        &mut self,
        view: &Frame<V>,
        animation: Option<Animation>,
    ) -> UpdateResult {
        let target = FrameValues::new(
            view.width,
            view.height,
            view.min_width,
            view.min_height,
            view.max_width,
            view.max_height,
        );
        if let Some(tracks) = self.animation.as_mut() {
            // Each track compares its destination, preserving unchanged targets
            // and their elapsed time even when a different field is retargeted.
            if !tracks.retarget(target, animation) {
                return UpdateResult::NoChange;
            }
            self.values = tracks.values();
            if !tracks.is_active() {
                self.animation = None;
            }
        } else {
            if self.values == target {
                return UpdateResult::NoChange;
            }
            if animation.is_some() {
                let mut tracks = FrameAnimationTracks::new(self.values);
                tracks.retarget(target, animation);
                self.values = tracks.values();
                // Zero-duration and entirely noninterpolable updates need no
                // retained allocation. Only actual motion owns boxed tracks.
                if tracks.is_active() {
                    self.animation = Some(Box::new(tracks));
                }
            } else {
                self.values = target;
            }
        }
        UpdateResult::Updated
    }

    /// Get the explicit width value
    pub fn width_value(&self) -> Option<f32> {
        self.values.width
    }

    /// Get the explicit height value
    pub fn height_value(&self) -> Option<f32> {
        self.values.height
    }

    /// Layout the frame using a child size
    pub fn layout_with_child(&mut self, child_size: Size, constraints: LayoutConstraints) -> Size {
        let min_w = self.values.min_width.max(constraints.min_width);
        let mut max_w = self.values.max_width.min(constraints.max_width);
        let min_h = self.values.min_height.max(constraints.min_height);
        let mut max_h = self.values.max_height.min(constraints.max_height);

        if min_w > max_w {
            max_w = min_w;
        }
        if min_h > max_h {
            max_h = min_h;
        }

        let width = if let Some(w) = self.width_value() {
            if w.is_finite() {
                w
            } else if constraints.max_width.is_finite() {
                constraints.max_width
            } else {
                child_size.width.clamp(min_w, max_w)
            }
        } else {
            child_size.width.clamp(min_w, max_w)
        };

        let height = if let Some(h) = self.height_value() {
            if h.is_finite() {
                h
            } else if constraints.max_height.is_finite() {
                constraints.max_height
            } else {
                child_size.height.clamp(min_h, max_h)
            }
        } else {
            child_size.height.clamp(min_h, max_h)
        };

        self.size = Size { width, height };
        self.size
    }
}

impl ElementRenderObject for FrameRenderObject {
    fn update_needs_layout(&self) -> bool {
        true
    }

    fn has_visual_animation(&self) -> bool {
        self.animation.is_some()
    }

    fn advance_visual_animation(&mut self, delta: Duration) -> bool {
        let Some(tracks) = self.animation.as_mut() else {
            return false;
        };
        let changed = tracks.advance(delta);
        self.values = tracks.values();
        if !tracks.is_active() {
            self.animation = None;
        }
        changed
    }

    fn finish_visual_animation(&mut self) -> bool {
        let Some(mut tracks) = self.animation.take() else {
            return false;
        };
        let changed = tracks.finish();
        self.values = tracks.values();
        changed
    }

    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        if crate::debug::is_enabled() {
            crate::logln!(
                "[FrameRenderObject::layout] START: constraints=({:?}, {:?}) -> ({:?}, {:?}), self.width={:?}, self.height={:?}",
                constraints.min_width,
                constraints.min_height,
                constraints.max_width,
                constraints.max_height,
                self.width_value(),
                self.height_value()
            );
        }
        // Determine the size based on explicit width/height and constraints
        let width = if let Some(w) = self.width_value() {
            if w.is_finite() {
                w
            } else if constraints.is_tight_width() {
                constraints.max_width
            } else {
                constraints
                    .min_width
                    .max(self.values.min_width)
                    .min(constraints.max_width.min(self.values.max_width))
            }
        } else {
            constraints
                .min_width
                .max(self.values.min_width)
                .min(constraints.max_width.min(self.values.max_width))
        };

        let height = if let Some(h) = self.height_value() {
            if h.is_finite() {
                h
            } else if constraints.is_tight_height() {
                constraints.max_height
            } else {
                constraints
                    .min_height
                    .max(self.values.min_height)
                    .min(constraints.max_height.min(self.values.max_height))
            }
        } else {
            constraints
                .min_height
                .max(self.values.min_height)
                .min(constraints.max_height.min(self.values.max_height))
        };

        self.size = Size { width, height };
        if crate::debug::is_enabled() {
            crate::logln!(
                "[FrameRenderObject::layout] FINAL: size={}x{}",
                width,
                height
            );
        }
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let (min_w, max_w) = match self.width_value() {
            Some(w) if w.is_finite() => (w, w),
            Some(_) if constraints.max_width.is_finite() => {
                (constraints.max_width, constraints.max_width)
            }
            Some(_) => (constraints.min_width, constraints.max_width),
            None => (constraints.min_width, constraints.max_width),
        };
        let (min_h, max_h) = match self.height_value() {
            Some(h) if h.is_finite() => (h, h),
            Some(_) if constraints.max_height.is_finite() => {
                (constraints.max_height, constraints.max_height)
            }
            Some(_) => (constraints.min_height, constraints.max_height),
            None => (constraints.min_height, constraints.max_height),
        };

        let child_constraints = LayoutConstraints {
            min_width: min_w,
            max_width: max_w,
            min_height: min_h,
            max_height: max_h,
        };

        let mut child_max = Size::ZERO;
        for child in children {
            let child_size = child.layout(child_constraints);
            child_max.width = child_max.width.max(child_size.width);
            child_max.height = child_max.height.max(child_size.height);
            // Don't override position - let parent (HStack/VStack) control it
        }

        self.layout_with_child(child_max, constraints)
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: Point) -> bool {
        let bounds = crate::geometry::Rect {
            origin: Point::ZERO,
            size: self.size,
        };
        bounds.contains(point)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        // Modifier doesn't directly render - child handles its own rendering
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Transaction, with_animation, with_transaction};
    use crate::views::Spacer;

    fn render_for(view: &Frame<Spacer>) -> FrameRenderObject {
        FrameRenderObject::new(
            view.width,
            view.height,
            view.min_width,
            view.min_height,
            view.max_width,
            view.max_height,
        )
    }

    #[test]
    fn frame_retargets_from_presentation_and_ignores_duplicate_targets() {
        let initial = Frame::new(Spacer::new(), 20.0, 40.0);
        let target = Frame::new(Spacer::new(), 100.0, 120.0);
        let mut render = render_for(&initial);
        let animation = Some(Animation::linear(Duration::from_secs(1)));

        assert!(!render.has_visual_animation());
        assert!(matches!(
            render.update_frame(&initial, animation),
            UpdateResult::NoChange
        ));
        assert!(matches!(
            render.update_frame(&target, animation),
            UpdateResult::Updated
        ));
        assert!(!render.advance_visual_animation(Duration::ZERO));
        assert_eq!(render.width_value(), Some(20.0));
        assert!(render.advance_visual_animation(Duration::from_millis(250)));
        assert_eq!(
            render.layout(LayoutConstraints::unconstrained()),
            Size::new(40.0, 60.0)
        );
        assert_eq!(render.values.min_width, 40.0);
        assert_eq!(render.values.max_width, 40.0);
        assert_eq!(render.values.min_height, 60.0);
        assert_eq!(render.values.max_height, 60.0);

        assert!(matches!(
            render.update_frame(&target, None),
            UpdateResult::NoChange
        ));
        assert!(render.has_visual_animation());
        render.advance_visual_animation(Duration::from_millis(250));
        assert_eq!(render.width_value(), Some(60.0));
        assert_eq!(render.height_value(), Some(80.0));
        render.update_frame(&initial, animation);
        assert_eq!(render.width_value(), Some(60.0));
        render.advance_visual_animation(Duration::from_millis(500));
        assert_eq!(render.width_value(), Some(40.0));
        assert_eq!(render.height_value(), Some(60.0));
        assert!(render.finish_visual_animation());
        assert_eq!(render.width_value(), Some(20.0));
        assert_eq!(render.height_value(), Some(40.0));
        assert!(!render.has_visual_animation());
        assert!(!render.finish_visual_animation());
    }

    #[test]
    fn optional_and_infinite_dimensions_snap_without_interrupting_finite_fields() {
        let mut render = render_for(&Frame::new(Spacer::new(), 40.0, 20.0));
        let animation = Some(Animation::linear(Duration::from_secs(1)));
        render.update_frame(&Frame::new(Spacer::new(), f32::INFINITY, 60.0), animation);
        assert_eq!(render.width_value(), Some(f32::INFINITY));
        assert_eq!(render.height_value(), Some(20.0));
        render.advance_visual_animation(Duration::from_millis(500));
        assert_eq!(
            render.layout_with_child(Size::ZERO, LayoutConstraints::loose(400.0, 300.0)),
            Size::new(400.0, 40.0)
        );

        render.update_frame(&Frame::height(Spacer::new(), 100.0), animation);
        assert_eq!(render.width_value(), None);
        assert_eq!(render.height_value(), Some(40.0));
        render.advance_visual_animation(Duration::from_millis(500));
        assert_eq!(render.height_value(), Some(70.0));
        render.update_frame(&Frame::new(Spacer::new(), 30.0, 100.0), animation);
        assert_eq!(render.width_value(), Some(30.0));
        assert_eq!(render.height_value(), Some(70.0));
        render.advance_visual_animation(Duration::from_millis(250));
        assert_eq!(render.height_value(), Some(85.0));
        render.finish_visual_animation();
        assert_eq!(
            render.layout(LayoutConstraints::unconstrained()),
            Size::new(30.0, 100.0)
        );
    }

    #[test]
    fn nan_inputs_are_stable_and_nonfinite_bounds_never_interpolate() {
        let view = Frame::new(Spacer::new(), f32::NAN, f32::NAN)
            .min_width(f32::NAN)
            .min_height(f32::NAN)
            .max_width(f32::NAN)
            .max_height(f32::NAN);
        let mut render = render_for(&view);
        let animation = Some(Animation::linear(Duration::from_secs(1)));
        assert!(matches!(
            render.update_frame(&view, animation),
            UpdateResult::NoChange
        ));
        assert_eq!(
            render.layout_with_child(Size::new(10.0, 20.0), LayoutConstraints::unconstrained()),
            Size::new(10.0, 20.0)
        );
        assert!(!render.has_visual_animation());

        let target = Frame::new(Spacer::new(), 30.0, 40.0);
        render.update_frame(&target, animation);
        assert_eq!(render.width_value(), Some(30.0));
        assert_eq!(render.height_value(), Some(40.0));
        assert_eq!(render.values.max_width, 30.0);
        assert_eq!(render.values.max_height, 40.0);
        render.advance_visual_animation(Duration::from_millis(500));
        assert_eq!(render.values.min_width, 15.0);
        assert_eq!(render.values.min_height, 20.0);
        assert_eq!(
            render.layout(LayoutConstraints::unconstrained()),
            Size::new(30.0, 40.0)
        );

        // A single-axis fill must also remain safe when measured unconstrained.
        let mut fill = Frame::width(Spacer::new(), f32::INFINITY).create_element();
        assert_eq!(fill.layout(LayoutConstraints::unconstrained()), Size::ZERO);
    }

    #[test]
    fn view_updates_use_the_effective_transaction_and_mount_without_motion() {
        let animation = Animation::linear(Duration::from_secs(1));
        let mut element = with_animation(animation, || {
            Frame::new(Spacer::new(), 10.0, 20.0).create_element()
        });
        assert!(!element.render_object().unwrap().has_visual_animation());
        with_animation(animation, || {
            element.update(&Frame::new(Spacer::new(), 30.0, 40.0))
        });
        assert!(element.render_object().unwrap().has_visual_animation());
        with_transaction(
            Transaction {
                animation: Some(animation),
                disables_animations: true,
            },
            || {
                element.update(&Frame::new(Spacer::new(), 50.0, 60.0));
            },
        );
        let render = element
            .render_object()
            .unwrap()
            .as_any()
            .downcast_ref::<FrameRenderObject>()
            .unwrap();
        assert_eq!(render.width_value(), Some(50.0));
        assert_eq!(render.height_value(), Some(60.0));
        assert!(!render.has_visual_animation());
    }

    #[test]
    fn static_frames_stay_compact_and_release_completed_transition_storage() {
        // Static nodes retain the six values, layout size and one pointer, with
        // at most one pointer's worth of padding on supported architectures.
        assert!(
            core::mem::size_of::<FrameRenderObject>()
                <= core::mem::size_of::<FrameValues>()
                    + core::mem::size_of::<Size>()
                    + 2 * core::mem::size_of::<usize>()
        );
        let mut render = render_for(&Frame::height(Spacer::new(), 10.0));
        assert!(render.animation.is_none());
        let animation = Some(Animation::linear(Duration::from_secs(1)));

        // Changing only an optional/nonfinite field requires no transition.
        render.update_frame(&Frame::new(Spacer::new(), f32::INFINITY, 10.0), animation);
        assert_eq!(render.width_value(), Some(f32::INFINITY));
        assert!(render.animation.is_none());
        render.update_frame(&Frame::new(Spacer::new(), 20.0, 30.0), None);
        assert!(render.animation.is_none());
        render.update_frame(
            &Frame::new(Spacer::new(), 40.0, 50.0),
            Some(Animation::linear(Duration::ZERO)),
        );
        assert_eq!(render.width_value(), Some(40.0));
        assert_eq!(render.height_value(), Some(50.0));
        assert!(render.animation.is_none());

        render.update_frame(&Frame::new(Spacer::new(), 60.0, 70.0), animation);
        assert!(render.animation.is_some());
        render.advance_visual_animation(Duration::from_secs(1));
        assert_eq!(render.width_value(), Some(60.0));
        assert_eq!(render.height_value(), Some(70.0));
        assert!(render.animation.is_none());
        assert!(!render.advance_visual_animation(Duration::from_secs(1)));

        render.update_frame(&Frame::new(Spacer::new(), 80.0, 90.0), animation);
        assert!(render.animation.is_some());
        assert!(render.finish_visual_animation());
        assert!(render.animation.is_none());
        assert!(!render.finish_visual_animation());
        render.update_frame(&Frame::new(Spacer::new(), 100.0, 110.0), animation);
        render.update_frame(&Frame::new(Spacer::new(), 120.0, 130.0), None);
        assert!(render.animation.is_none());
        assert_eq!(render.width_value(), Some(120.0));
        assert_eq!(render.height_value(), Some(130.0));
    }

    #[test]
    fn retargeting_one_axis_preserves_the_other_axis_deadline() {
        let mut render = render_for(&Frame::new(Spacer::new(), 20.0, 40.0));
        let animation = Some(Animation::linear(Duration::from_secs(1)));
        render.update_frame(&Frame::new(Spacer::new(), 100.0, 120.0), animation);
        render.advance_visual_animation(Duration::from_millis(250));
        assert_eq!(render.width_value(), Some(40.0));
        assert_eq!(render.height_value(), Some(60.0));

        let target = Frame::new(Spacer::new(), 60.0, 120.0);
        render.update_frame(&target, animation);
        render.advance_visual_animation(Duration::from_millis(250));
        assert_eq!(render.width_value(), Some(45.0));
        assert_eq!(render.height_value(), Some(80.0));
        assert!(matches!(
            render.update_frame(&target, None),
            UpdateResult::NoChange
        ));
        render.advance_visual_animation(Duration::from_millis(500));
        assert_eq!(render.width_value(), Some(55.0));
        assert_eq!(render.height_value(), Some(120.0));
        assert!(render.has_visual_animation());
        render.advance_visual_animation(Duration::from_millis(250));
        assert_eq!(render.width_value(), Some(60.0));
        assert_eq!(render.height_value(), Some(120.0));
        assert!(!render.has_visual_animation());
        assert!(render.animation.is_none());
    }

    #[test]
    fn spring_to_zero_keeps_presented_bounds_child_constraints_and_layout_nonnegative() {
        for explicit in [true, false] {
            let mut initial = Frame::new(Spacer::new(), 100.0, 100.0);
            let mut target = Frame::new(Spacer::new(), 0.0, 0.0);
            if !explicit {
                initial.width = None;
                initial.height = None;
                target.width = None;
                target.height = None;
            }
            let mut render = render_for(&initial);
            let mut children = vec![Spacer::new().create_element()];
            let constraints = LayoutConstraints::loose(200.0, 200.0);
            render.update_frame(
                &target,
                Some(Animation::spring(Duration::from_secs(1), 0.5)),
            );
            let mut saw_clamped_overshoot = false;
            for _ in 0..100 {
                render.advance_visual_animation(Duration::from_millis(10));
                let values = render.values;
                for value in [values.width, values.height].into_iter().flatten() {
                    assert!(value.is_finite() && value >= 0.0);
                }
                for bound in [
                    values.min_width,
                    values.min_height,
                    values.max_width,
                    values.max_height,
                ] {
                    assert!(bound.is_finite() && bound >= 0.0);
                }
                if !explicit {
                    assert_eq!(values.width, None);
                    assert_eq!(values.height, None);
                }
                saw_clamped_overshoot |= values.max_width == 0.0 && render.has_visual_animation();

                let size = render.layout_with_children(constraints, &mut children);
                assert!(size.width.is_finite() && size.width >= 0.0);
                assert!(size.height.is_finite() && size.height >= 0.0);
                assert_eq!(render.layout(constraints), size);
                let child_constraints = children[0].last_layout_constraints().unwrap();
                for bound in [
                    child_constraints.min_width,
                    child_constraints.max_width,
                    child_constraints.min_height,
                    child_constraints.max_height,
                ] {
                    assert!(bound.is_finite() && bound >= 0.0);
                }
                assert!(child_constraints.min_width <= child_constraints.max_width);
                assert!(child_constraints.min_height <= child_constraints.max_height);
            }
            assert!(
                saw_clamped_overshoot,
                "exercise the spring below zero before completion"
            );
            assert_eq!(render.size(), Size::ZERO);
            assert!(!render.has_visual_animation());
        }
    }

    #[test]
    fn interrupted_spring_retargets_from_the_clamped_presentation() {
        let mut render = render_for(&Frame::new(Spacer::new(), 100.0, 100.0));
        let zero = Frame::new(Spacer::new(), 0.0, 0.0);
        let spring = Some(Animation::spring(Duration::from_secs(1), 0.5));
        render.update_frame(&zero, spring);
        render.advance_visual_animation(Duration::from_millis(250));
        assert_eq!(render.width_value(), Some(0.0));
        assert_eq!(render.height_value(), Some(0.0));
        assert!(matches!(
            render.update_frame(&zero, None),
            UpdateResult::NoChange
        ));
        assert!(render.has_visual_animation());

        let linear = Some(Animation::linear(Duration::from_secs(1)));
        render.update_frame(&Frame::new(Spacer::new(), 60.0, 80.0), linear);
        assert_eq!(render.width_value(), Some(0.0));
        assert_eq!(render.height_value(), Some(0.0));
        render.advance_visual_animation(Duration::from_millis(500));
        assert_eq!(render.width_value(), Some(30.0));
        assert_eq!(render.height_value(), Some(40.0));
        assert_eq!(render.values.min_width, 30.0);
        assert_eq!(render.values.max_width, 30.0);
        assert_eq!(render.values.min_height, 40.0);
        assert_eq!(render.values.max_height, 40.0);

        // A changed target equal to the clamped presentation must also stop
        // the old transition and report the change.
        let mut render = render_for(&Frame::new(Spacer::new(), 100.0, 100.0));
        render.update_frame(&Frame::new(Spacer::new(), 1.0, 1.0), spring);
        render.advance_visual_animation(Duration::from_millis(250));
        assert_eq!(render.width_value(), Some(0.0));
        assert!(matches!(
            render.update_frame(&zero, linear),
            UpdateResult::Updated
        ));
        assert_eq!(render.width_value(), Some(0.0));
        assert_eq!(render.height_value(), Some(0.0));
        assert!(!render.has_visual_animation());
    }

    #[test]
    fn nonnegative_presentation_preserves_optional_and_unbounded_dimensions() {
        let mut render = FrameRenderObject::new(
            None,
            Some(f32::INFINITY),
            -10.0,
            -20.0,
            f32::INFINITY,
            f32::INFINITY,
        );
        assert_eq!(render.width_value(), None);
        assert_eq!(render.height_value(), Some(f32::INFINITY));
        assert_eq!(render.values.min_width, 0.0);
        assert_eq!(render.values.min_height, 0.0);
        assert_eq!(render.values.max_width, f32::INFINITY);
        assert_eq!(render.values.max_height, f32::INFINITY);
        assert_eq!(
            render.layout_with_child(Size::new(30.0, 40.0), LayoutConstraints::unconstrained()),
            Size::new(30.0, 40.0)
        );
    }
}
