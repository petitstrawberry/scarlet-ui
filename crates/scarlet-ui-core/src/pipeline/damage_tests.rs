//! Match GPU damage semantics: physical damage controls the cleared area.
use super::*;
use crate::color::Color;
use crate::element::{ComponentElement, Element};
use crate::geometry::Size;
use crate::state::{Listenable, State, generate_state_id};
use crate::view::{View, ViewExt};
use crate::views::{HStack, Rectangle, VStack, Window};
use core::any::Any;

struct PhysicalDamageBackend {
    cpu: CpuPaintBackend,
    scale: f32,
}

#[test]
fn coalesced_damage_rectangles_never_overlap_after_expansion() {
    let mut seed = 12345u32;
    for _ in 0..1000 {
        let mut rectangles = Vec::new();
        for _ in 0..8 {
            let mut next = || {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                seed >> 16
            };
            rectangles.push((next() % 100, next() % 100, next() % 20 + 1, next() % 20 + 1));
        }
        RenderingPipeline::coalesce_damage_rects(&mut rectangles);
        assert!(rectangles.len() <= MAX_PRESENT_DAMAGE_RECTS);
        for (index, a) in rectangles.iter().enumerate() {
            for b in &rectangles[index + 1..] {
                assert!(
                    !RenderingPipeline::damage_rects_touch_or_overlap(*a, *b),
                    "Expanded damage must not blend alpha twice: {rectangles:?}"
                );
            }
        }
    }
}
impl PaintBackend for PhysicalDamageBackend {
    fn resize(&mut self, size: Size, scale_milli: u32) {
        self.scale = scale_milli.max(1) as f32 / 1000.0;
        self.cpu.resize(size, scale_milli);
    }
    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        background: Color,
        _: Option<&[Rect]>,
        physical: Option<&[DamageRect]>,
    ) -> crate::error::Result<BackendFrame<'a>> {
        let logical: Option<Vec<_>> = physical.map(|areas| {
            areas
                .iter()
                .map(|&(x, y, w, h)| {
                    Rect::from_xywh(
                        x as f32 / self.scale,
                        y as f32 / self.scale,
                        w as f32 / self.scale,
                        h as f32 / self.scale,
                    )
                })
                .collect()
        });
        self.cpu
            .render(context, background, logical.as_deref(), physical)
    }
}

#[derive(Clone)]
struct Pane {
    color: State<Color>,
    size: Size,
}
impl View for Pane {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(self.clone(), |pane| {
            Box::new(
                Rectangle::new()
                    .fill(pane.color.get())
                    .frame(pane.size.width, pane.size.height),
            )
        }))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.color]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn touching_queue_and_transport_damage_preserves_unchanged_content() {
    for scale in [1000, 2000] {
        let right = State::new(generate_state_id(), Color::RED);
        let bottom = State::new(generate_state_id(), Color::BLUE);
        let static_color = Color::rgb(20, 180, 60);
        let content = VStack::new((
            HStack::new((
                Rectangle::new().fill(static_color).frame(70., 80.),
                Pane {
                    color: right.clone(),
                    size: Size::new(30., 80.),
                },
            ))
            .spacing(0.),
            Pane {
                color: bottom.clone(),
                size: Size::new(100., 20.),
            },
        ))
        .spacing(0.)
        .frame(100., 100.);
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_scale_milli(scale);
        pipeline.set_root(
            Window::new("Damage", content)
                .decorated(false)
                .size(Size::new(100., 100.))
                .create_element(),
        );
        pipeline.layout_initial();
        pipeline.set_paint_backend(Box::new(PhysicalDamageBackend {
            cpu: CpuPaintBackend::new(Size::new(100., 100.), scale, Color::WHITE),
            scale: scale as f32 / 1000.,
        }));
        let point = 10 * scale / 1000;
        let initial = match pipeline.render_for_present().unwrap() {
            PresentedFrame::Cpu { buffer, .. } => buffer.get_pixel(point, point),
            _ => panic!("Initial scene did not render"),
        };
        assert_eq!(initial, Some(static_color.to_bgra()));
        right.set(Color::rgb(200, 100, 20));
        bottom.set(Color::rgb(100, 20, 200));
        let updated = match pipeline.render_for_present().unwrap() {
            PresentedFrame::Cpu { buffer, .. } => buffer.get_pixel(point, point),
            _ => panic!("Changed scene did not render"),
        };
        assert_eq!(
            updated, initial,
            "Static content was cleared at scale {scale}"
        );
    }
}

#[test]
fn clean_deferral_keeps_damage_relative_to_last_accepted_scene() {
    use core::cell::{Cell, RefCell};
    use std::rc::Rc;
    struct DeferredBackend {
        blocked: Rc<Cell<bool>>,
        damage: Rc<RefCell<Option<Vec<DamageRect>>>>,
        renderer: CpuPaintRenderer,
    }
    impl PaintBackend for DeferredBackend {
        fn supports_retained_display_lists(&self) -> bool {
            true
        }
        fn resize(&mut self, size: Size, scale: u32) {
            self.renderer.resize(size, scale);
        }
        fn render<'a>(
            &'a mut self,
            ctx: &PaintContext<'_>,
            background: Color,
            logical: Option<&[Rect]>,
            physical: Option<&[DamageRect]>,
        ) -> crate::Result<BackendFrame<'a>> {
            *self.damage.borrow_mut() = physical.map(<[DamageRect]>::to_vec);
            if self.blocked.get() {
                return Err(crate::Error::RenderDeferred);
            }
            self.renderer.set_background_color(background);
            self.renderer.execute_with_damage(ctx, logical);
            let mut reference = CpuPaintRenderer::new(Size::new(200., 100.), 1000, background);
            reference.execute_with_damage(ctx, None);
            assert_eq!(
                self.renderer.buffer().data(),
                reference.buffer().data(),
                "retry must repair every change since the last accepted frame"
            );
            Ok(BackendFrame::External)
        }
    }
    let a = State::new(generate_state_id(), Color::BLACK);
    let b = State::new(generate_state_id(), Color::BLACK);
    let blocked = Rc::new(Cell::new(false));
    let damage = Rc::new(RefCell::new(None));
    let mut p = RenderingPipeline::new();
    p.resize(Size::new(200., 100.));
    p.set_root(
        Window::new(
            "Deferred",
            HStack::new((
                Pane {
                    color: a.clone(),
                    size: Size::new(30., 30.),
                },
                Pane {
                    color: b.clone(),
                    size: Size::new(30., 30.),
                },
            ))
            .spacing(40.),
        )
        .decorated(false)
        .size(Size::new(200., 100.))
        .create_element(),
    );
    p.set_paint_backend(Box::new(DeferredBackend {
        blocked: blocked.clone(),
        damage: damage.clone(),
        renderer: CpuPaintRenderer::new(Size::new(200., 100.), 1000, Color::WHITE),
    }));
    p.layout_initial();
    p.render_for_present().unwrap();
    let accepted = p.last_direct_scene.as_ref().unwrap().identity();
    blocked.set(true);
    a.set(Color::RED);
    assert!(matches!(
        p.render_for_present(),
        Err(crate::Error::RenderDeferred)
    ));
    assert_eq!(p.last_direct_scene.as_ref().unwrap().identity(), accepted);
    assert!(
        !p.paint_needs_full,
        "clean admission pressure must not force full repair"
    );
    b.set(Color::BLUE);
    assert!(matches!(
        p.render_for_present(),
        Err(crate::Error::RenderDeferred)
    ));
    assert_eq!(p.last_direct_scene.as_ref().unwrap().identity(), accepted);
    assert!(!p.has_dirty());
    blocked.set(false);
    assert!(matches!(
        p.render_for_present().unwrap(),
        PresentedFrame::External
    ));
    assert!(
        damage.borrow().is_some(),
        "retry must retain partial damage"
    );
    assert!(!p.direct_paint_deferred);
    assert!(matches!(
        p.render_for_present().unwrap(),
        PresentedFrame::Idle
    ));

    // A real expose/backing repair remains full even across clean deferral.
    blocked.set(true);
    p.request_redraw();
    assert!(matches!(
        p.render_for_present(),
        Err(crate::Error::RenderDeferred)
    ));
    assert!(p.paint_needs_full);
    blocked.set(false);
    assert!(matches!(
        p.render_for_present().unwrap(),
        PresentedFrame::External
    ));
    assert!(damage.borrow().is_none());
}
