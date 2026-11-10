mod scene;

use anyhow::{Context, Result, bail, ensure};
use bench_core::{
    AdapterEvent, FrameSample, Framework, MeasurementMode, PipelineSample, SamplePhase, Scenario,
    Workload,
};
use clap::{Parser, ValueEnum};
use scarlet_ui::{
    WinitBackend,
    event::{Event, WindowEvent},
    geometry::Size,
    pipeline::RenderingPipeline,
    platform::{PlatformBackend, PlatformWindow, WindowCreateRequest, WindowPlacement},
    prelude::*,
    renderer::{PresentedFrame, RendererBackendKind},
};
use std::{
    io::BufRead,
    time::{Duration, Instant},
};

const FONT: &[u8] = include_bytes!("../../../assets/IBMPlexSans-Regular.ttf");

#[derive(Parser)]
struct Args {
    #[arg(long)]
    scenario: String,
    #[arg(long, value_enum)]
    mode: Mode,
}

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Headline,
    Diagnostic,
}

fn ui_error(error: scarlet_ui::Error) -> anyhow::Error {
    anyhow::anyhow!("ScarletUI: {error:?}")
}

fn events(
    window: &mut dyn PlatformWindow,
    pipeline: &mut RenderingPipeline,
    suspended: &mut bool,
) -> Result<()> {
    while let Some(event) = window.poll_event() {
        match &event {
            Event::Quit | Event::Window(WindowEvent::CloseRequested) => {
                bail!("benchmark window closed before completion")
            }
            Event::Resize { width, height } if *width > 0 && *height > 0 => {
                window.apply_resize(*width, *height).map_err(ui_error)?;
                pipeline.resize(Size::new(*width as f32, *height as f32));
            }
            Event::WindowSuspendedChanged { suspended: value } => *suspended = *value,
            _ => {
                pipeline.handle_event(&event);
            }
        }
    }
    if pipeline.scale_milli() != window.output_scale_milli() {
        pipeline.set_scale_milli(window.output_scale_milli());
    }
    // A manually pumped AppKit loop can coalesce configure notifications.
    // Reconcile against the native client extent, never the requested size.
    let (width, height) = window.physical_size();
    let scale = window.output_scale_milli() as f32 / 1000.0;
    let observed = Size::new(
        (width as f32 / scale).round(),
        (height as f32 / scale).round(),
    );
    if observed != pipeline.window_size() {
        pipeline.resize(observed);
    }
    Ok(())
}

fn run() -> Result<()> {
    let args = Args::parse();
    let spec: Scenario = serde_json::from_str(&args.scenario).context("invalid scenario JSON")?;
    spec.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        spec.width > 0 && spec.height > 0,
        "window dimensions must be positive"
    );
    let placement = match std::env::var("SCARLET_UI_BENCH_POSITION") {
        Ok(position) => {
            let (x, y) = position.split_once(',').context("position must be x,y")?;
            WindowPlacement::At {
                x: x.parse()?,
                y: y.parse()?,
            }
        }
        Err(std::env::VarError::NotPresent) => WindowPlacement::Default,
        Err(error) => return Err(error.into()),
    };
    let mode = match args.mode {
        Mode::Headline => MeasurementMode::Headline,
        Mode::Diagnostic => MeasurementMode::Diagnostic,
    };
    bench_core::emit_event(&AdapterEvent::Hello {
        protocol_version: bench_core::PROTOCOL_VERSION,
        framework: Framework::Scarletui,
        framework_version: "ScarletUI 1.0.0 (local checkout, winit/SGFX)".into(),
        mode,
        pid: std::process::id(),
        capabilities: vec![
            "frame_timing".into(),
            "stimulus_to_frame_latency".into(),
            "window_resize".into(),
            "sgfx_submission_boundary".into(),
        ],
    })?;
    let mut start = String::new();
    std::io::stdin().lock().read_line(&mut start)?;
    ensure!(start.trim() == "start", "expected start command");
    bench_core::emit_event(&AdapterEvent::Ready)?;
    let epoch = Instant::now();
    scarlet_ui::debug::set_enabled(false);
    scarlet_ui::graphics::set_default_font(FONT)
        .map_err(|_| anyhow::anyhow!("invalid benchmark font"))?;
    let document = scene::Document::new(spec.clone());
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(
        Window::new("gui-bench: ScarletUI", document.clone().padding(8.0).clip())
            .decorated(false)
            .background_color(Color::rgb(11, 16, 32))
            .size(Size::new(spec.width as f32, spec.height as f32))
            .create_element(),
    );
    let info = pipeline.layout_initial();
    let mut backend = WinitBackend::new();
    let mut window = backend
        .create_window(WindowCreateRequest {
            app_id: "org.scarlet-ui.gui-bench".into(),
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
            placement,
            window_geometry_insets: scarlet_ui::geometry::EdgeInsets::ZERO,
        })
        .map_err(ui_error)?;
    ensure!(
        window.renderer_backend() == RendererBackendKind::Sgfx,
        "benchmark requires SGFX; CPU fallback is not comparable"
    );
    pipeline.set_scale_milli(window.output_scale_milli());
    let (width, height) = window.physical_size();
    let scale = window.output_scale_milli() as f32 / 1000.0;
    pipeline.resize(Size::new(width as f32 / scale, height as f32 / scale));
    pipeline.set_paint_backend(
        window
            .take_paint_backend()
            .map_err(ui_error)?
            .context("missing SGFX paint backend")?,
    );
    let result = measure(&spec, epoch, &document, &mut pipeline, window.as_mut());
    pipeline.teardown(); // Drop GPU paint resources before the native window.
    let close = window.close();
    result?;
    close.map_err(ui_error)?;
    Ok(())
}

fn measure(
    spec: &Scenario,
    epoch: Instant,
    document: &scene::Document,
    pipeline: &mut RenderingPipeline,
    window: &mut dyn PlatformWindow,
) -> Result<()> {
    let total = u64::from(spec.warmup_frames) + u64::from(spec.sample_frames);
    let passive = matches!(spec.workload, Workload::Idle | Workload::ColdStart);
    let mut last_frame = None::<Instant>;
    let mut suspended = false;
    for sequence in 0..=total {
        events(window, pipeline, &mut suspended)?;
        let stimulus = if sequence == 0 {
            Some(epoch)
        } else if passive {
            None
        } else {
            Some(Instant::now())
        };
        let mut requested = None;
        if sequence > 0 {
            document.stimulate(sequence);
            let previous = sequence - 1;
            requested = match spec.workload {
                Workload::ResizeStep => Some(if (previous / 8) % 2 == 0 {
                    (1200, 800)
                } else {
                    (900, 620)
                }),
                Workload::ResizeStorm => Some((
                    840 + (previous % 120) as u32 * 3,
                    560 + (previous % 120) as u32 * 2,
                )),
                _ => None,
            };
            if let Some((width, height)) = requested {
                window.resize(width, height).map_err(ui_error)?;
                // Observe the OS configure; resize()'s cached requested size is
                // not evidence that the compositor granted it.
                let deadline = Instant::now() + Duration::from_millis(250);
                while Instant::now() < deadline {
                    events(window, pipeline, &mut suspended)?;
                    let size = pipeline.window_size();
                    if (size.width as u32, size.height as u32) == (width, height) {
                        break;
                    }
                    window.wait_for_event(Duration::from_millis(1));
                }
            }
            if passive || requested.is_some() {
                pipeline.request_redraw();
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut retried_redraw = false;
        let accepted = loop {
            let accepted = !suspended
                && match pipeline.render_for_present().map_err(ui_error)? {
                    PresentedFrame::External => true,
                    PresentedFrame::Cpu { .. } => bail!("unexpected CPU presentation"),
                    PresentedFrame::Idle => false,
                };
            if accepted || Instant::now() >= deadline {
                break accepted;
            }
            // Offscreen/no-op mutations can legitimately coalesce. A retained
            // redraw schedules a submission without forcing a layout/rebuild.
            pipeline.request_redraw();
            // The first retry already has runnable work. Pumping AppKit twice
            // per sample (or waiting here) adds latency unrelated to rendering.
            if suspended || retried_redraw {
                window.wait_for_event(Duration::from_millis(1));
                events(window, pipeline, &mut suspended)?;
            }
            retried_redraw = true;
        };
        let now = Instant::now();
        let size = pipeline.window_size();
        let observed = (size.width.round() as u32, size.height.round() as u32);
        let resize_granted = requested.is_none_or(|size| size == observed);
        let sample = FrameSample {
            sequence,
            phase: if sequence == 0 {
                SamplePhase::Startup
            } else if sequence <= u64::from(spec.warmup_frames) {
                SamplePhase::Warmup
            } else {
                SamplePhase::Measurement
            },
            timestamp_ns: now.duration_since(epoch).as_nanos() as u64,
            frame_time_ns: last_frame.map(|last| now.duration_since(last).as_nanos() as u64),
            stimulus_latency_ns: stimulus.map(|start| now.duration_since(start).as_nanos() as u64),
            observed_width: Some(observed.0),
            observed_height: Some(observed.1),
            output_scale_milli: Some(window.output_scale_milli()),
            pipeline: PipelineSample::default(), // No fabricated internal/GPU timings.
            framework_memory: None,
            valid: accepted && resize_granted,
            note: if !accepted {
                Some("no SGFX submission (suspended or idle)".into())
            } else if !resize_granted {
                Some("requested resize not observed before submission".into())
            } else {
                None
            },
        };
        bench_core::emit_event(&AdapterEvent::Frame { sample })?;
        ensure!(accepted, "no frame accepted within five seconds");
        last_frame = Some(now);
    }
    bench_core::emit_event(&AdapterEvent::Complete { frames: total + 1 })?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        let _ = bench_core::emit_event(&AdapterEvent::Error {
            message: format!("{error:#}"),
        });
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
