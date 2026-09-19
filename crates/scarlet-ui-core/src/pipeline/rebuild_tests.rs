//! Reconciliation regressions: retain unchanged pixels and repair alpha damage.

use super::*;
use crate::element::ComponentElement;
use crate::prelude::*;
use crate::state::{Listenable, State, generate_state_id};
use crate::views::{BitmapImage, Image, ImageFit};
use core::any::Any;

#[derive(Clone)]
struct Model {
    selected: usize,
    card_width: f32,
    inset: f32,
    gap: f32,
    radius: f32,
    title: &'static str,
    artwork: BitmapImage,
}

#[derive(Clone)]
struct Preview {
    state: State<Model>,
    clear: Color,
}

impl Preview {
    fn body(&self) -> impl View + Clone + use<> {
        let model = self.state.get();
        let card = |index| {
            VStack::new((
                Image::from_bitmap(model.artwork.clone())
                    .fit_mode(ImageFit::Cover)
                    .frame(model.card_width, 60.0),
                HStack::new((
                    IconView::new(Icon::Folder),
                    Text::new(model.title).font_size(14.0).color(Color::WHITE),
                ))
                .spacing(4.0)
                .frame(model.card_width, 24.0),
            ))
            .spacing(0.0)
            .frame(model.card_width, 84.0)
            .clip_radius(model.radius)
            .repaint_boundary()
            .border_rounded(
                if model.selected == index {
                    Color::RED
                } else {
                    Color::WHITE
                },
                2.0,
                model.radius,
            )
            .repaint_boundary()
        };
        Window::new(
            "Retained selection",
            ZStack::new((
                Spacer::new()
                    .frame(360.0, 180.0)
                    .background(Color::rgba(20, 40, 80, 140)),
                HStack::new((card(0), card(1)))
                    .spacing(model.gap)
                    .padding(model.inset)
                    .alignment(Alignment::Leading)
                    .frame(360.0, 180.0),
                // Overlap a transparent section and a child repaint boundary.
                Spacer::new()
                    .frame(50.0, 40.0)
                    .background(Color::rgba(90, 140, 40, 100)),
            ))
            .frame(360.0, 180.0)
            .repaint_boundary(),
        )
        .decorated(false)
        .background_color(self.clear)
        .size(Size::new(360.0, 180.0))
    }
}

impl View for Preview {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(self.clone(), |v| {
            Box::new(v.body())
        }))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.state]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn model() -> Model {
    Model {
        selected: 0,
        card_width: 90.0,
        inset: 12.0,
        gap: 10.0,
        radius: 10.0,
        title: "App",
        artwork: BitmapImage::from_bgra(vec![0xffff0050, 0x884000ff, 0, 0xff22aa88], 2, 2),
    }
}

fn pipeline(view: &Preview, scale: u32) -> RenderingPipeline {
    let mut p = RenderingPipeline::new();
    p.set_root(view.create_element());
    p.set_scale_milli(scale);
    p.layout_initial();
    p
}

fn pixels(p: &mut RenderingPipeline) -> Vec<u32> {
    p.render_with_damage()
        .expect("expected a CPU frame")
        .0
        .as_slice()
        .to_vec()
}

#[test]
fn unchanged_parent_rebuild_retains_images_text_icons_and_layout() {
    let state = State::new(generate_state_id(), model());
    let view = Preview {
        state: state.clone(),
        clear: Color::TRANSPARENT,
    };
    let mut p = pipeline(&view, 2000);
    let _ = pixels(&mut p);
    p.reset_paint_test_counters();
    state.set(state.get());
    let _ = p.render_for_present().unwrap();
    assert!(p.pipeline_owner.last_paint_ids().is_empty());
    assert_eq!(p.paint_test_counters.boundary_rebuilds, 0);
}

#[test]
fn expose_requests_a_complete_frame_without_repainting_retained_artwork() {
    let view = Preview {
        state: State::new(generate_state_id(), model()),
        clear: Color::TRANSPARENT,
    };
    let mut p = pipeline(&view, 2000);
    let before = pixels(&mut p);
    assert!(!p.has_dirty());
    p.reset_paint_test_counters();
    p.request_redraw();
    assert!(p.has_dirty());
    let (frame, damage) = p.render_with_damage().unwrap();
    assert!(damage.is_none());
    assert_eq!(frame.as_slice(), before);
    assert_eq!(p.paint_test_counters.boundary_rebuilds, 0);
    assert!(!p.has_dirty());
}

#[test]
fn selection_repairs_only_damage_and_matches_full_render_with_overlapping_alpha() {
    for clear in [Color::TRANSPARENT, Color::BLACK] {
        for scale in [1000, 1500, 2000] {
            let state = State::new(generate_state_id(), model());
            let view = Preview {
                state: state.clone(),
                clear,
            };
            let mut p = pipeline(&view, scale);
            let before = pixels(&mut p);
            p.reset_paint_test_counters();
            state.update(|m| m.selected = 1);
            let (frame, damage) = p.render_with_damage().unwrap();
            let after = frame.as_slice().to_vec();
            let area: u64 = damage
                .expect("selection must not redraw the whole window")
                .iter()
                .map(|r| r.2 as u64 * r.3 as u64)
                .sum();
            assert!(area < frame.width() as u64 * frame.height() as u64);
            assert_ne!(before, after);
            assert_eq!(
                p.paint_test_counters.boundary_rebuilds, 2,
                "only the two outline layers should change"
            );
            let mut fresh = pipeline(&view, scale);
            let expected = pixels(&mut fresh);
            assert!(
                after == expected,
                "partial alpha repair clear={clear:?} at scale {scale}: {:?}; damage={:?} {:?}",
                after
                    .iter()
                    .zip(&expected)
                    .enumerate()
                    .find(|(_, (a, b))| a != b),
                p.dirty_scratch.rects,
                p.paint_damage
            );
        }
    }
}

#[test]
fn retained_layout_and_artwork_still_update_and_match_fresh_frames() {
    let state = State::new(generate_state_id(), model());
    let view = Preview {
        state: state.clone(),
        clear: Color::TRANSPARENT,
    };
    let mut p = pipeline(&view, 2000);
    let _ = pixels(&mut p);
    for step in 0..5 {
        state.update(|m| match step {
            0 => {
                m.card_width = 105.0;
                m.inset = 8.0;
                m.gap = 22.0;
            }
            1 => {
                m.radius = 2.0;
                m.title = "Updated";
            }
            2 => {
                m.artwork = BitmapImage::from_bgra(vec![0xff00ff00; 6], 3, 2);
            }
            3 => {
                m.card_width = 76.0;
                m.inset = 14.0;
                m.gap = 5.0;
            }
            _ => {
                m.selected = 1;
            }
        });
        let after = pixels(&mut p);
        let mut fresh = pipeline(&view, 2000);
        let expected = pixels(&mut fresh);
        assert!(
            after == expected,
            "retained update step {step}: {:?}",
            after
                .iter()
                .zip(&expected)
                .enumerate()
                .find(|(_, (a, b))| a != b)
        );
    }
}
