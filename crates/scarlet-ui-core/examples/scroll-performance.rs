//! Deterministic release benchmark: input, virtualization, layout and paint.
//! The command backend excludes GPU execution/presentation; this is not FPS.
use scarlet_ui_core::pipeline::RenderingPipeline;
use scarlet_ui_core::{
    event::{Event, MouseEvent, ScrollSource, WheelPhase},
    prelude::*,
    renderer::{BackendFrame, PaintBackend, PaintContext},
};
use std::{cell::Cell, rc::Rc, time::Instant};
struct CommandBackend(Rc<Cell<usize>>);
impl PaintBackend for CommandBackend {
    fn resize(&mut self, _: Size, _: u32) {}
    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        _: Color,
        _: Option<&[Rect]>,
        _: Option<&[scarlet_ui_core::compositor::DamageRect]>,
    ) -> scarlet_ui_core::Result<BackendFrame<'a>> {
        self.0.set(context.commands().len());
        Ok(BackendFrame::External)
    }
}
fn measure(view: impl View + Clone, name: &str) {
    let commands = Rc::new(Cell::new(0));
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_paint_backend(Box::new(CommandBackend(commands.clone())));
    pipeline.set_root(
        Window::new("Scroll benchmark", view)
            .decorated(false)
            .size(Size::new(1200., 800.))
            .create_element(),
    );
    pipeline.resize(Size::new(1200., 800.));
    pipeline.layout_initial();
    pipeline.render_for_present().unwrap();
    let mut times = Vec::new();
    for i in 0..600 {
        let started = Instant::now();
        pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            x: 700,
            y: 400,
            delta_x: 0,
            delta_y: if i < 300 { -160 } else { 160 },
            phase: WheelPhase::Moved,
            source: ScrollSource::Trackpad,
        }));
        pipeline.render_for_present().unwrap();
        times.push(started.elapsed().as_micros());
    }
    times.sort_unstable();
    println!(
        "{name}: median={}us p95={}us p99={}us max={}us commands={}",
        times[300],
        times[570],
        times[594],
        times[599],
        commands.get()
    );
    pipeline.teardown();
}
fn main() {
    for count in [500, 50_000] {
        let items = State::new(
            scarlet_ui_core::state::generate_state_id(),
            (0..count).collect(),
        );
        measure(
            ListView::new(
                items.clone(),
                State::new(scarlet_ui_core::state::generate_state_id(), None),
                38.,
                |i, _, _| {
                    scarlet_ui_core::hstack! {
                Text::new(format!("Track {i}: 静かな街を歩く")).font_size(13.).frame(540., 38.),
                Text::new("Cadence Ensemble").font_size(12.).frame(300., 38.),
                Text::new("Small moments").font_size(12.).frame(250., 38.),
            }.spacing(8.).padding(2.)
                },
            )
            .frame(1200., 800.),
            &format!("list/{count}"),
        );
        measure(
            GridView::new(
                items,
                State::new(scarlet_ui_core::state::generate_state_id(), None),
                6,
                230.,
                |i, _, _| {
                    scarlet_ui_core::vstack! {
                Rectangle::new().fill(Color::rgb(140, 40, 60)).corner_radius(10.).frame(160., 160.),
                Text::new(format!("Album {i} 音楽")).font_size(13.).frame(160., 24.),
                Text::new("Artist name").font_size(11.).frame(160., 20.),
            }.spacing(4.)
                },
            )
            .frame(1200., 800.),
            &format!("grid/{count}"),
        );
    }
}
