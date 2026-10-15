//! External GPU images placed directly in the ScarletUI paint order.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use scarlet_ui_core::color::Color;
use scarlet_ui_core::element::{Element, ElementRenderObject, LayoutConstraints, UpdateResult};
use scarlet_ui_core::geometry::{Point, Rect, Size};
use scarlet_ui_core::renderer::PaintContext;
use scarlet_ui_core::state::{InvalidationKind, Listenable, State, SubscriptionId};
use scarlet_ui_core::view::View;

use crate::SgfxTexture;

/// ScarletUI view that composites an imported GPU image directly.
///
/// This is the lightweight path for an image produced by Vulkan, a video
/// decoder, or another GPU API. Unlike [`crate::SgfxCanvas`], it does not
/// allocate or render an intermediate canvas target. The active platform
/// backend must know how to import the [`SgfxTexture`]'s external image.
#[derive(Clone, Debug)]
pub struct ExternalGpuSurface {
    size: Size,
    source: ExternalGpuSurfaceSource,
    placeholder: Color,
    damage: Option<State<Option<Rect>>>,
}

#[derive(Clone, Debug)]
enum ExternalGpuSurfaceSource {
    Snapshot(Arc<SgfxTexture>),
    State(ExternalTextureState),
}

// New image contents are read at paint time and leave element geometry unchanged.
#[derive(Clone, Debug)]
struct ExternalTextureState(State<Arc<SgfxTexture>>);

impl Listenable for ExternalTextureState {
    fn subscribe_any(&self, callback: Arc<dyn Fn() + Send + Sync>) -> SubscriptionId {
        self.0.subscribe_any(callback)
    }

    fn unsubscribe(&self, id: SubscriptionId) -> bool {
        self.0.unsubscribe(id)
    }

    fn invalidation_kind(&self) -> InvalidationKind {
        InvalidationKind::SelfPaint
    }
}

impl ExternalGpuSurfaceSource {
    fn texture(&self) -> Arc<SgfxTexture> {
        match self {
            Self::Snapshot(texture) => Arc::clone(texture),
            Self::State(texture) => texture.0.get(),
        }
    }
}

impl ExternalGpuSurface {
    /// Create a surface backed by one external texture.
    ///
    /// Use [`Self::from_state`] when the producer updates the contents or
    /// rotates through multiple images.
    pub fn new(width: f32, height: f32, texture: Arc<SgfxTexture>) -> Self {
        Self {
            size: Size::new(width, height),
            source: ExternalGpuSurfaceSource::Snapshot(texture),
            placeholder: Color::rgb(0.025, 0.035, 0.055),
            damage: None,
        }
    }

    /// Create a reactive surface backed by an external texture state.
    ///
    /// Set the state after the producer has finished a new image. Setting the
    /// same texture again is valid and invalidates the surface; alternating
    /// texture objects supports image queues once the platform supplies the
    /// corresponding producer/consumer synchronization contract.
    pub fn from_state(width: f32, height: f32, texture: State<Arc<SgfxTexture>>) -> Self {
        Self {
            size: Size::new(width, height),
            source: ExternalGpuSurfaceSource::State(ExternalTextureState(texture)),
            placeholder: Color::rgb(0.025, 0.035, 0.055),
            damage: None,
        }
    }

    /// Set the color used when the active paint backend cannot draw the image.
    pub fn placeholder(mut self, color: Color) -> Self {
        self.placeholder = color;
        self
    }

    /// Producer damage in local logical coordinates. Set this before changing
    /// the texture state, unioning skipped updates until presentation succeeds.
    /// `None` requests a full repaint. Geometry changes retain full invalidation.
    pub fn damage(mut self, damage: State<Option<Rect>>) -> Self {
        self.damage = Some(damage);
        self
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ExternalGpuSurfacePaint {
    pub(crate) texture: Arc<SgfxTexture>,
}

impl View for ExternalGpuSurface {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui_core::element::RenderElement::new(
            self.clone(),
            ExternalGpuSurfaceRenderObject {
                size: self.size,
                source: self.source.clone(),
                placeholder: self.placeholder,
                damage: self.damage.clone(),
            },
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        match &self.source {
            ExternalGpuSurfaceSource::Snapshot(_) => Vec::new(),
            ExternalGpuSurfaceSource::State(texture) => alloc::vec![texture as &dyn Listenable],
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object used by [`ExternalGpuSurface`].
pub struct ExternalGpuSurfaceRenderObject {
    size: Size,
    source: ExternalGpuSurfaceSource,
    placeholder: Color,
    damage: Option<State<Option<Rect>>>,
}

impl ElementRenderObject for ExternalGpuSurfaceRenderObject {
    fn self_paint_bounds(&self, origin: Point) -> Option<Rect> {
        let damage = self.damage.as_ref()?.get()?;
        if !damage.origin.x.is_finite()
            || !damage.origin.y.is_finite()
            || !damage.size.width.is_finite()
            || !damage.size.height.is_finite()
            || damage.size.width < 0.0
            || damage.size.height < 0.0
        {
            return None;
        }
        let x = damage.origin.x.max(0.0).min(self.size.width);
        let y = damage.origin.y.max(0.0).min(self.size.height);
        let right = (damage.origin.x + damage.size.width)
            .max(x)
            .min(self.size.width);
        let bottom = (damage.origin.y + damage.size.height)
            .max(y)
            .min(self.size.height);
        Some(Rect::from_xywh(
            origin.x + x,
            origin.y + y,
            right - x,
            bottom - y,
        ))
    }
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let width = self
            .size
            .width
            .clamp(constraints.min_width, constraints.max_width);
        let height = self
            .size
            .height
            .clamp(constraints.min_height, constraints.max_height);
        self.size = Size::new(width, height);
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn render(&mut self) {}

    fn paint<'a>(&'a self, ctx: &mut PaintContext<'a>, origin: Point) -> bool {
        let rect = Rect::new(origin, self.size);
        ctx.fill_rect(rect, self.placeholder);
        ctx.draw_extension(
            rect,
            Arc::new(ExternalGpuSurfacePaint {
                texture: self.source.texture(),
            }),
        );
        true
    }

    fn emits_paint_extension(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(surface) = new_view.as_any().downcast_ref::<ExternalGpuSurface>() else {
            return UpdateResult::Replaced;
        };
        self.size = surface.size;
        self.source = surface.source.clone();
        self.placeholder = surface.placeholder;
        self.damage = surface.damage.clone();
        UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use scarlet_ui_core::renderer::PaintCommand;
    use scarlet_ui_core::state::generate_state_id;

    #[test]
    fn producer_damage_clips_to_the_surface_and_full_damage_is_explicit() {
        #[derive(Debug)]
        struct Image;
        let texture = State::new(
            generate_state_id(),
            SgfxTexture::external_bgra8(320, 240, Arc::new(Image)),
        );
        let damage = State::new(
            generate_state_id(),
            Some(Rect::from_xywh(20.0, 30.0, 40.0, 50.0)),
        );
        let view = ExternalGpuSurface::from_state(320.0, 240.0, texture).damage(damage.clone());
        let mut element = view.create_element();
        element.layout(LayoutConstraints::tight(320.0, 240.0));
        let render = element.render_object().unwrap();
        assert_eq!(
            render.self_paint_bounds(Point::new(5.0, 7.0)),
            Some(Rect::from_xywh(25.0, 37.0, 40.0, 50.0))
        );
        damage.set(Some(Rect::from_xywh(-10.0, 200.0, 100.0, 100.0)));
        assert_eq!(
            render.self_paint_bounds(Point::ZERO),
            Some(Rect::from_xywh(0.0, 200.0, 90.0, 40.0))
        );
        damage.set(None);
        assert_eq!(render.self_paint_bounds(Point::ZERO), None);
    }

    #[test]
    fn texture_updates_repaint_existing_surface_without_canvas_or_layout_reset() {
        #[derive(Debug)]
        struct Image;
        let first = SgfxTexture::external_bgra8(16, 16, Arc::new(Image));
        let next = SgfxTexture::external_bgra8(16, 16, Arc::new(Image));
        let state = State::new(generate_state_id(), first);
        let view = ExternalGpuSurface::from_state(800.0, 600.0, state.clone());
        let listenables = view.listenables();
        assert_eq!(listenables.len(), 1);
        assert_eq!(
            listenables[0].invalidation_kind(),
            InvalidationKind::SelfPaint
        );
        let notifications = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&notifications);
        let subscription = listenables[0].subscribe_any(Arc::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        }));
        let mut element = view.create_element();
        element.layout(LayoutConstraints::tight(320.0, 240.0));
        state.set(Arc::clone(&next));
        assert_eq!(notifications.load(Ordering::SeqCst), 1);
        let render = element.render_object().unwrap();
        assert_eq!(render.size(), Size::new(320.0, 240.0));
        let mut paint = PaintContext::new();
        assert!(render.paint(&mut paint, Point::ZERO));
        let payloads: Vec<_> = paint
            .commands()
            .iter()
            .filter_map(|command| {
                let PaintCommand::Extension { payload, .. } = command else {
                    return None;
                };
                Some(payload)
            })
            .collect();
        assert_eq!(payloads.len(), 1);
        assert!(
            payloads[0]
                .as_ref()
                .as_any()
                .downcast_ref::<crate::canvas::SgfxCanvasPaint>()
                .is_none()
        );
        let surface = payloads[0]
            .as_ref()
            .as_any()
            .downcast_ref::<ExternalGpuSurfacePaint>()
            .unwrap();
        assert!(Arc::ptr_eq(&surface.texture, &next));
        assert!(listenables[0].unsubscribe(subscription));
        state.set(next);
        assert_eq!(notifications.load(Ordering::SeqCst), 1);
    }
}
