//! Native renderer stress probe, including submission and presentation backpressure.
//! `cargo run --release -p scarlet-ui --example scroll-stress --features platform-winit`
//! Advances the standard ListView's selection/scroll target once per accepted frame.
//! Measures accepted frame intervals, not display scanout or input latency.
use scarlet_ui::{WindowContext, prelude::*, views::BitmapImage};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
#[derive(Clone)]
struct Stress {
    items: State<Vec<usize>>,
    selected: State<Option<usize>>,
    covers: Rc<Vec<BitmapImage>>,
    metrics: Rc<RefCell<Metrics>>,
}
struct Metrics {
    startup: Instant,
    previous: Option<Instant>,
    samples: Vec<Duration>,
    frames: usize,
    finished: bool,
}
impl Application for Stress {
    fn scenes(&self) -> impl Scene {
        let covers = self.covers.clone();
        Window::new("ScarletUI scroll stress — 50,000 rows", ListView::new(self.items.clone(), self.selected.clone(), 42., move |index, _, _| {
            scarlet_ui::hstack! {
                Image::from_bitmap(covers[index % covers.len()].clone()).frame(36., 36.),
                Text::new(format!("Track {index:05} {}", (0..20).map(|column| char::from_u32(0x4e00 + ((index * 20 + column) % 700) as u32).unwrap()).collect::<String>())).font_size(13.).frame(500., 38.),
                Text::new("Cadence Ensemble").font_size(12.).frame(280., 38.),
                Text::new("Small moments").font_size(12.).frame(240., 38.),
            }.spacing(8.).padding(2.)
        }).frame(1200., 800.)).scene_key("stress").size(Size::new(1200., 850.))
    }
    fn on_idle(&mut self) {
        let m = self.metrics.borrow();
        if m.finished {
            scarlet_ui::dismiss_window("stress");
        } else if m.previous.is_none() && m.startup.elapsed() > Duration::from_secs(3) {
            self.selected.set(Some(30));
        } else if m.startup.elapsed() > Duration::from_secs(30) {
            panic!("Scroll stress did not finish within 30 seconds");
        }
    }
    fn on_frame_presented(&mut self, _: &WindowContext) {
        let mut m = self.metrics.borrow_mut();
        if m.startup.elapsed() < Duration::from_secs(3) {
            return;
        }
        let now = Instant::now();
        if let Some(previous) = m.previous {
            m.samples.push(now.duration_since(previous));
        }
        m.previous = Some(now);
        m.frames += 1;
        if m.frames < 300 {
            self.selected.set(Some(30 + m.frames));
            return;
        }
        let warm = &mut m.samples[30..];
        let seconds: f64 = warm.iter().map(Duration::as_secs_f64).sum();
        let rate = warm.len() as f64 / seconds;
        warm.sort_unstable();
        println!(
            "accepted frames/s={rate:.1}; interval median={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms; warm samples={}",
            warm[warm.len() / 2].as_secs_f64() * 1000.,
            warm[warm.len() * 95 / 100].as_secs_f64() * 1000.,
            warm[warm.len() * 99 / 100].as_secs_f64() * 1000.,
            warm.last().unwrap().as_secs_f64() * 1000.,
            warm.len()
        );
        m.finished = true;
    }
}
impl View for Stress {
    fn create_element(&self) -> Box<dyn Element> {
        Text::new("Scroll stress").create_element()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
fn main() {
    let covers = (0..64)
        .map(|i| {
            BitmapImage::from_bgra(
                vec![Color::rgb((i * 3) as u8, 50, 100).to_bgra(); 256 * 256],
                256,
                256,
            )
        })
        .collect();
    let mut app = Stress {
        items: State::new(scarlet_ui::generate_state_id(), (0..50_000).collect()),
        selected: State::new(scarlet_ui::generate_state_id(), None),
        covers: Rc::new(covers),
        metrics: Rc::new(RefCell::new(Metrics {
            startup: Instant::now(),
            previous: None,
            samples: Vec::new(),
            frames: 0,
            finished: false,
        })),
    };
    app.run().unwrap();
}
