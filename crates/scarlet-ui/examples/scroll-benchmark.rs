//! Repeatable native GPU scroll benchmark. Measures submission/presentation
//! backpressure, not physical display scanout or OS event-pump throughput. No application timer or selected
//! row changes drive scrolling: wheel events go through the normal dispatcher.
//! Run release with `--features platform-winit --example scroll-benchmark`.
//! Args: case (plain/artwork/complex), seconds, delta, update interval (0=static).
use scarlet_ui::{
    WinitBackend,
    event::{Event, MouseEvent, ScrollSource, WheelPhase},
    pipeline::RenderingPipeline,
    platform::{PlatformBackend, WindowCreateRequest},
    prelude::*,
    renderer::{PresentedFrame, RendererBackendKind},
    views::{BitmapImage, ProgressView},
};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};
#[derive(Clone)]
struct Row {
    id: usize,
    complexity: String,
    cover: BitmapImage,
}
impl View for Row {
    fn create_element(&self) -> Box<dyn Element> {
        let label = format!("Track {:05} — 静かな街を歩く", self.id);
        match self.complexity.as_str() {
            "plain" => scarlet_ui::hstack! {
                Text::new(label).font_size(13.).frame(540., 42.),
                Text::new("Cadence Ensemble").font_size(12.).frame(280., 42.),
                Text::new("Small moments").font_size(12.).frame(250., 42.),
            }.spacing(8.).create_element(),
            "artwork" => scarlet_ui::hstack! {
                Image::from_bitmap(self.cover.clone()).frame(36., 36.),
                Text::new(label).font_size(13.).frame(500., 42.),
                Text::new("Cadence Ensemble").font_size(12.).frame(280., 42.),
                Text::new("Small moments").font_size(12.).frame(220., 42.),
            }.spacing(8.).create_element(),
            "complex" => scarlet_ui::hstack! {
                Image::from_bitmap(self.cover.clone()).frame(36., 36.),
                scarlet_ui::vstack! { Text::new(label).font_size(13.), Text::new("Cadence Ensemble • Small moments").font_size(11.) }.spacing(2.).frame(510., 42.),
                ProgressView::new((self.id % 100) as f32 / 100.).frame(200., 12.),
                Text::new("03:42").font_size(12.).frame(80., 42.),
                Button::new("More").frame(90., 28.),
            }.spacing(12.).create_element(),
            _ => panic!("case must be plain/artwork/complex"),
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let case = args.get(1).cloned().unwrap_or("plain".into());
    let seconds: f64 = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(10.);
    let delta: i32 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(128);
    let update_every: usize = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(0);
    let covers: Rc<Vec<_>> = Rc::new(
        (0..64)
            .map(|i| {
                BitmapImage::from_bgra(
                    (0..256 * 256)
                        .map(|pixel| {
                            Color::rgb(
                                (i * 3) as u8,
                                ((pixel / 256 + i) % 256) as u8,
                                ((pixel % 256 + i * 7) % 256) as u8,
                            )
                            .to_bgra()
                        })
                        .collect(),
                    256,
                    256,
                )
            })
            .collect(),
    );
    let items = State::new(
        scarlet_ui::generate_state_id(),
        (0..50_000usize).collect::<Vec<_>>(),
    );
    let row_case = case.clone();
    let view = ListView::new(
        items.clone(),
        State::new(scarlet_ui::generate_state_id(), None),
        42.,
        move |_, id, _| Row {
            id,
            complexity: row_case.clone(),
            cover: covers[id % covers.len()].clone(),
        },
    )
    .item_key(|id| (*id).into())
    .frame(1200., 800.);
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(
        Window::new(format!("ScarletUI benchmark — {case}"), view)
            .decorated(false)
            .size(Size::new(1200., 800.))
            .create_element(),
    );
    let info = pipeline.layout_initial();
    let mut backend = WinitBackend::new();
    let mut window = backend
        .create_window(WindowCreateRequest {
            app_id: "org.scarlet-ui.scroll-benchmark".into(),
            title: info.title,
            size: info.size,
            size_limits: scarlet_ui::element::WindowSizeLimits {
                resizable: true,
                ..Default::default()
            },
            window_type: info.window_type,
            menu_titles: String::new(),
            focus_on_create: true,
            active_on_focus: true,
            opaque: true,
            decoration: scarlet_ui::platform::WindowDecoration::SYSTEM,
            placement: Default::default(),
            window_geometry_insets: scarlet_ui::geometry::EdgeInsets::ZERO,
        })
        .unwrap();
    assert_eq!(
        window.renderer_backend(),
        RendererBackendKind::Sgfx,
        "benchmark requires GPU rendering"
    );
    pipeline.set_scale_milli(window.output_scale_milli());
    pipeline.resize(window.size());
    pipeline.set_paint_backend(window.take_paint_backend().unwrap().expect("SGFX backend"));
    pipeline.render_for_present().unwrap();
    let began = Instant::now();
    let mut samples = Vec::new();
    let mut updates = Vec::new();
    let mut count = 0;
    let mut accepted = 0;
    let mut warmup_done = false;
    while began.elapsed() < Duration::from_secs_f64(seconds + 1.) {
        while let Some(event) = window.poll_event() {
            pipeline.handle_event(&event);
        }
        if !warmup_done && began.elapsed() >= Duration::from_secs(1) {
            println!("[ScrollBenchmark] warmup_complete");
            warmup_done = true;
        }
        let start = Instant::now();
        let updating = update_every > 0 && count % update_every == 0;
        if updating {
            let mut model = items.get();
            if (count / update_every) % 2 == 0 {
                model.insert(0, 50_000 + count);
            } else {
                model.remove(0);
            }
            items.set(model);
        }
        pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            x: 700,
            y: 400,
            delta_x: 0,
            delta_y: if (count / 300) % 2 == 0 {
                -delta
            } else {
                delta
            },
            phase: WheelPhase::Moved,
            source: ScrollSource::Trackpad,
        }));
        if matches!(
            pipeline.render_for_present().unwrap(),
            PresentedFrame::External
        ) {
            accepted += 1;
        }
        let time = start.elapsed().as_secs_f64() * 1000.;
        if warmup_done {
            samples.push(time);
            if updating {
                updates.push(time);
            }
        }
        count += 1;
    }
    let total: f64 = samples.iter().sum();
    let over8 = samples.iter().filter(|t| **t > 8.333).count();
    let over16 = samples.iter().filter(|t| **t > 16.667).count();
    samples.sort_by(f64::total_cmp);
    updates.sort_by(f64::total_cmp);
    println!(
        "{{\"case\":\"{case}\",\"retained\":{},\"scale_milli\":{},\"wheel_delta\":{delta},\"update_every\":{update_every},\"samples\":{},\"accepted_including_warmup\":{accepted},\"work_rate_per_s\":{:.2},\"median_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"over_8_33_percent\":{:.2},\"over_16_67_percent\":{:.2},\"update_p95_ms\":{:.3}}}",
        scarlet_ui::debug::retained_paint_enabled(),
        window.output_scale_milli(),
        samples.len(),
        samples.len() as f64 * 1000. / total,
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
        samples[samples.len() * 99 / 100],
        samples.last().unwrap(),
        over8 as f64 * 100. / samples.len() as f64,
        over16 as f64 * 100. / samples.len() as f64,
        if updates.is_empty() {
            0.
        } else {
            updates[updates.len() * 95 / 100]
        }
    );
    pipeline.teardown();
    window.close().unwrap();
}
