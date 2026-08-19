use std::hint::black_box;
use std::time::{Duration, Instant};

use scarlet_ui_core::color::Color;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::geometry::{Point, Rect, Size};
use scarlet_ui_core::renderer::{BackendFrame, CpuPaintBackend, PaintBackend, PaintContext};
use scarlet_ui_renderer_sgfx::WgpuSgfxSession;

const LOGICAL_WIDTH: u32 = 860;
const LOGICAL_HEIGHT: u32 = 560;
const SCALE_MILLI: u32 = 2_000;
const DEFAULT_WARMUP_FRAMES: usize = 30;
const DEFAULT_SAMPLE_FRAMES: usize = 120;

#[derive(Clone, Copy)]
struct Scenario<'a> {
    name: &'a str,
    paint: &'a PaintContext<'static>,
    logical_damage: Option<&'a [Rect]>,
    physical_damage: Option<&'a [DamageRect]>,
}

struct SampleStats {
    samples: Vec<Duration>,
}

impl SampleStats {
    fn new(samples: Vec<Duration>) -> Self {
        Self { samples }
    }

    fn mean_ms(&self) -> f64 {
        self.samples.iter().map(Duration::as_secs_f64).sum::<f64>() * 1_000.0
            / self.samples.len() as f64
    }

    fn percentile_ms(&self, percentile: f64) -> f64 {
        let mut samples = self.samples.clone();
        samples.sort_unstable();
        let index = ((samples.len() - 1) as f64 * percentile).round() as usize;
        samples[index].as_secs_f64() * 1_000.0
    }

    fn min_ms(&self) -> f64 {
        self.samples
            .iter()
            .min()
            .expect("benchmark samples")
            .as_secs_f64()
            * 1_000.0
    }

    fn max_ms(&self) -> f64 {
        self.samples
            .iter()
            .max()
            .expect("benchmark samples")
            .as_secs_f64()
            * 1_000.0
    }
}

fn main() {
    let warmup_frames = env_usize("SCARLET_UI_BENCH_WARMUP", DEFAULT_WARMUP_FRAMES);
    let sample_frames = env_usize("SCARLET_UI_BENCH_SAMPLES", DEFAULT_SAMPLE_FRAMES);
    let size = Size::new(LOGICAL_WIDTH as f32, LOGICAL_HEIGHT as f32);
    let physical_width = physical_dimension(LOGICAL_WIDTH);
    let physical_height = physical_dimension(LOGICAL_HEIGHT);
    let background = Color::rgb(248u8, 249u8, 251u8);
    let full_paint = widget_factory_paint();
    let partial_paint = changed_row_paint();
    let logical_damage = [Rect::from_xywh(222.0, 244.0, 610.0, 44.0)];
    let physical_damage = [(
        scale_u32(222),
        scale_u32(244),
        scale_u32(610),
        scale_u32(44),
    )];
    let full = Scenario {
        name: "full repaint",
        paint: &full_paint,
        logical_damage: None,
        physical_damage: None,
    };
    let partial = Scenario {
        name: "44px row damage",
        paint: &partial_paint,
        logical_damage: Some(&logical_damage),
        physical_damage: Some(&physical_damage),
    };

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("a WGPU adapter is required for this benchmark");
    let adapter_info = adapter.get_info();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .expect("WGPU device creation");

    let mut cpu = CpuPaintBackend::new(size, SCALE_MILLI, background);
    let mut gpu = WgpuSgfxSession::new(device, queue, physical_width, physical_height, false)
        .expect("SGFX/WGPU session creation");

    // Establish an initial retained image before measuring partial damage and
    // populate the font and glyph-atlas caches before timed warmup begins.
    render_cpu_once(&mut cpu, full, background);
    render_gpu_once(&mut gpu, full, background);

    println!(
        "ScarletUI paint-backend benchmark: {}x{} logical, {}x{} physical ({}x)",
        LOGICAL_WIDTH,
        LOGICAL_HEIGHT,
        physical_width,
        physical_height,
        SCALE_MILLI as f32 / 1_000.0,
    );
    println!(
        "WGPU adapter: {} ({:?}, {:?})",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    );
    println!(
        "commands: full={}, partial={}; warmup={}, samples={}",
        full_paint.commands().len(),
        partial_paint.commands().len(),
        warmup_frames,
        sample_frames,
    );
    println!(
        "{:<18} {:<13} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "scenario", "backend", "mean ms", "p50 ms", "p95 ms", "min ms", "max ms"
    );

    for scenario in [full, partial] {
        let cpu_stats = benchmark_cpu(&mut cpu, scenario, background, warmup_frames, sample_frames);
        let gpu_submit_stats =
            benchmark_gpu_submit(&mut gpu, scenario, background, warmup_frames, sample_frames);
        let gpu_stats = benchmark_gpu(&mut gpu, scenario, background, warmup_frames, sample_frames);
        print_stats(scenario.name, "CPU", &cpu_stats);
        print_stats(scenario.name, "WGPU submit", &gpu_submit_stats);
        print_stats(scenario.name, "WGPU complete", &gpu_stats);
        println!(
            "  -> SGFX/WGPU raster completion: {:.2}x {} than CPU\n",
            ratio(cpu_stats.mean_ms(), gpu_stats.mean_ms()),
            if gpu_stats.mean_ms() <= cpu_stats.mean_ms() {
                "faster"
            } else {
                "slower"
            },
        );
    }

    println!(
        "Times exclude layout/paint-list construction and native surface presentation; WGPU includes device.poll(Wait)."
    );
}

fn benchmark_cpu(
    backend: &mut CpuPaintBackend,
    scenario: Scenario<'_>,
    background: Color,
    warmup_frames: usize,
    sample_frames: usize,
) -> SampleStats {
    for _ in 0..warmup_frames {
        render_cpu_once(backend, scenario, background);
    }
    let mut samples = Vec::with_capacity(sample_frames);
    for _ in 0..sample_frames {
        let started = Instant::now();
        render_cpu_once(backend, scenario, background);
        samples.push(started.elapsed());
    }
    SampleStats::new(samples)
}

fn benchmark_gpu(
    session: &mut WgpuSgfxSession,
    scenario: Scenario<'_>,
    background: Color,
    warmup_frames: usize,
    sample_frames: usize,
) -> SampleStats {
    for _ in 0..warmup_frames {
        render_gpu_once(session, scenario, background);
    }
    let mut samples = Vec::with_capacity(sample_frames);
    for _ in 0..sample_frames {
        let started = Instant::now();
        render_gpu_once(session, scenario, background);
        samples.push(started.elapsed());
    }
    SampleStats::new(samples)
}

fn benchmark_gpu_submit(
    session: &mut WgpuSgfxSession,
    scenario: Scenario<'_>,
    background: Color,
    warmup_frames: usize,
    sample_frames: usize,
) -> SampleStats {
    for _ in 0..warmup_frames {
        render_gpu_once(session, scenario, background);
    }
    let mut samples = Vec::with_capacity(sample_frames);
    for _ in 0..sample_frames {
        let started = Instant::now();
        session
            .render_with_damage(
                scenario.paint,
                background,
                SCALE_MILLI,
                scenario.physical_damage,
            )
            .expect("SGFX/WGPU render submission");
        samples.push(started.elapsed());
        let _ = session.raw_device().poll(wgpu::Maintain::Wait);
    }
    SampleStats::new(samples)
}

fn render_cpu_once(backend: &mut CpuPaintBackend, scenario: Scenario<'_>, background: Color) {
    let frame = backend
        .render(
            scenario.paint,
            background,
            scenario.logical_damage,
            scenario.physical_damage,
        )
        .expect("CPU render");
    let BackendFrame::Cpu { buffer } = frame else {
        panic!("CPU backend returned an external frame");
    };
    black_box(buffer.as_slice().get(buffer.as_slice().len() / 2));
}

fn render_gpu_once(session: &mut WgpuSgfxSession, scenario: Scenario<'_>, background: Color) {
    session
        .render_with_damage(
            scenario.paint,
            background,
            SCALE_MILLI,
            scenario.physical_damage,
        )
        .expect("SGFX/WGPU render");
    let _ = session.raw_device().poll(wgpu::Maintain::Wait);
    black_box(session.image());
}

fn print_stats(scenario: &str, backend: &str, stats: &SampleStats) {
    println!(
        "{:<18} {:<13} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>9.3}",
        scenario,
        backend,
        stats.mean_ms(),
        stats.percentile_ms(0.50),
        stats.percentile_ms(0.95),
        stats.min_ms(),
        stats.max_ms(),
    );
}

fn ratio(cpu_ms: f64, gpu_ms: f64) -> f64 {
    if cpu_ms >= gpu_ms {
        cpu_ms / gpu_ms.max(f64::EPSILON)
    } else {
        gpu_ms / cpu_ms.max(f64::EPSILON)
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn physical_dimension(logical: u32) -> u32 {
    scale_u32(logical)
}

fn scale_u32(logical: u32) -> u32 {
    ((u64::from(logical) * u64::from(SCALE_MILLI)) / 1_000) as u32
}

fn widget_factory_paint() -> PaintContext<'static> {
    let mut paint = PaintContext::new();
    let text = Color::rgb(31u8, 35u8, 43u8);
    let muted = Color::rgb(102u8, 110u8, 124u8);
    let border = Color::rgb(211u8, 216u8, 225u8);
    let accent = Color::rgb(70u8, 112u8, 224u8);

    paint.fill_rect(
        Rect::from_xywh(0.0, 0.0, LOGICAL_WIDTH as f32, LOGICAL_HEIGHT as f32),
        Color::rgb(248u8, 249u8, 251u8),
    );
    paint.fill_rect(
        Rect::from_xywh(0.0, 0.0, LOGICAL_WIDTH as f32, 38.0),
        Color::rgb(235u8, 237u8, 242u8),
    );
    paint.draw_text(
        Point::new(18.0, 10.0),
        "ScarletUI Widget Factory",
        text,
        15.0,
    );
    paint.fill_rect(
        Rect::from_xywh(0.0, 38.0, 190.0, LOGICAL_HEIGHT as f32 - 38.0),
        Color::rgb(241u8, 243u8, 247u8),
    );

    for (index, label) in ["Overview", "Controls", "Inputs", "Display", "SGFX"]
        .iter()
        .enumerate()
    {
        let y = 58.0 + index as f32 * 42.0;
        if index == 0 {
            paint.fill_rounded_rect(
                Rect::from_xywh(12.0, y, 166.0, 34.0),
                8.0,
                Color::rgb(218u8, 226u8, 247u8),
            );
        }
        paint.draw_text(Point::new(28.0, y + 9.0), *label, text, 14.0);
    }

    paint.push_clip(Rect::from_xywh(190.0, 38.0, 670.0, 522.0));
    paint.draw_text(Point::new(222.0, 66.0), "Widget Factory", text, 28.0);
    paint.draw_text(
        Point::new(222.0, 101.0),
        "PaintCommand default rendering",
        muted,
        15.0,
    );

    for index in 0..10 {
        let y = 134.0 + index as f32 * 40.0;
        let row = Rect::from_xywh(214.0, y, 620.0, 34.0);
        paint.fill_rounded_rect(row, 7.0, Color::WHITE);
        paint.stroke_rounded_rect(row, 7.0, 1.0, border);
        paint.draw_text(
            Point::new(228.0, y + 9.0),
            format!("Factory control {:02}", index + 1),
            text,
            14.0,
        );
        let track = Rect::from_xywh(482.0, y + 12.0, 316.0, 10.0);
        paint.fill_rounded_rect(track, 5.0, Color::rgb(226u8, 230u8, 237u8));
        paint.fill_rounded_rect(
            Rect::from_xywh(
                track.origin.x,
                track.origin.y,
                track.size.width * (0.18 + index as f32 * 0.071),
                track.size.height,
            ),
            5.0,
            accent,
        );
    }

    paint.push_rounded_clip(Rect::from_xywh(604.0, 468.0, 230.0, 72.0), 9.0);
    paint.fill_rect(
        Rect::from_xywh(604.0, 468.0, 300.0, 110.0),
        Color::rgb(235u8, 241u8, 252u8),
    );
    for index in 0..5 {
        paint.draw_text(
            Point::new(618.0, 480.0 + index as f32 * 20.0),
            format!("Nested scroll track {}", index + 1),
            muted,
            13.0,
        );
    }
    paint.pop_clip();
    paint.pop_clip();
    paint
}

fn changed_row_paint() -> PaintContext<'static> {
    let mut paint = PaintContext::new();
    let row = Rect::from_xywh(214.0, 244.0, 620.0, 34.0);
    paint.fill_rounded_rect(row, 7.0, Color::WHITE);
    paint.stroke_rounded_rect(row, 7.0, 1.0, Color::rgb(211u8, 216u8, 225u8));
    paint.draw_text(
        Point::new(228.0, 253.0),
        "Factory control 04",
        Color::rgb(31u8, 35u8, 43u8),
        14.0,
    );
    paint.fill_rounded_rect(
        Rect::from_xywh(482.0, 256.0, 316.0, 10.0),
        5.0,
        Color::rgb(226u8, 230u8, 237u8),
    );
    paint.fill_rounded_rect(
        Rect::from_xywh(482.0, 256.0, 214.0, 10.0),
        5.0,
        Color::rgb(70u8, 112u8, 224u8),
    );
    paint
}
