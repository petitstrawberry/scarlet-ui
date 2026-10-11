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
