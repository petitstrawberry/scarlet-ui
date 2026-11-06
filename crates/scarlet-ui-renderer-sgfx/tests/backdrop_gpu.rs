//! Headless execution/readback coverage for the portable fixed-pipeline blur.
#![cfg(not(target_arch = "wasm32"))]
use scarlet_ui_core::{color::Color, geometry::Rect, renderer::PaintContext};
use scarlet_ui_renderer_sgfx::SgfxPaintEncoder;
use sgfx::ir::TextureFormat;
use sgfx_backend_wgpu::{Context, Device};
use std::time::Instant;

fn context() -> Option<Context> {
    let context = try_context();
    if context.is_none() {
        assert!(
            std::env::var_os("SCARLET_REQUIRE_GPU_TESTS").is_none(),
            "GPU comparison requested but no usable WGPU adapter"
        );
        eprintln!("SKIP: no usable WGPU adapter; backdrop GPU pixels unverified");
    }
    context
}

fn try_context() -> Option<Context> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }))
    .or_else(|| pollster::block_on(instance.request_adapter(&Default::default())))?;
    eprintln!("backdrop pixel-test adapter: {:?}", adapter.get_info());
    if !adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::VIEW_FORMATS)
    {
        eprintln!("SKIP: adapter lacks VIEW_FORMATS required by the existing SGFX backend");
        return None;
    }
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default(), None)).ok()?;
    Some(Device::new(device, queue).create_context())
}

fn scene(edge: f32, transparent: bool) -> PaintContext<'static> {
    let mut paint = PaintContext::new();
    paint.fill_rounded_rect(
        Rect::from_xywh(0., 0., edge, 96.),
        0.,
        if transparent {
            Color::rgba(1., 0., 0., 0.5)
        } else {
            Color::WHITE
        },
    );
    paint.push_rounded_clip(Rect::from_xywh(16., 16., 96., 64.), 8.);
    paint.backdrop_filter_rounded_rect(
        Rect::from_xywh(16., 16., 96., 64.),
        8.,
        12.,
        Color::rgba(0., 0., 0., 0.),
    );
    paint.pop_clip();
    // Later foreground must not be sampled or softened.
    paint.fill_rounded_rect(
        Rect::from_xywh(90., 28., 2., 40.),
        0.,
        Color::rgb(0., 1., 0.),
    );
    paint
}
fn rgba(bytes: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [bytes[at + 2], bytes[at + 1], bytes[at], bytes[at + 3]]
}

#[test]
fn real_backdrop_pixels_clips_foreground_updates_and_alpha_at_1x_2x() {
    let Some(context) = context() else {
        return;
    };
    for scale in [1, 2] {
        let (width, height) = (128 * scale, 96 * scale);
        let mut renderer = SgfxPaintEncoder::new(width, height, false).unwrap();
        renderer.set_analytic_glass(true);
        let target = renderer.target_texture(0).unwrap();
        let mut resources = context.create_resources(renderer.resource_table());
        let image = context
            .create_image(width, height, TextureFormat::Bgra8Unorm)
            .unwrap();
        resources.map_image(target, image).unwrap();
        let queue = context.create_queue();
        let mut render = |paint: &PaintContext<'_>, background: Color| {
            renderer
                .encode_frame(
                    &mut queue.executor(&mut resources),
                    0,
                    None,
                    paint,
                    background,
                    scale * 1000,
                    &[(40 * scale, 40 * scale, scale, scale)],
                )
                .unwrap();
            assert_eq!(renderer.backdrop_fallback_count(), 0);
            resources.read_texture(target).unwrap()
        };
        let started = Instant::now();
        let pixels = render(&scene(64., false), Color::BLACK);
        let cold = started.elapsed();
        let p = |x, y| rgba(&pixels, width, x * scale, y * scale);
        assert!(
            p(61, 45)[0] > 100 && p(61, 45)[0] < 245,
            "edge should be blurred: {:?}",
            p(61, 45)
        );
        assert_eq!(
            p(10, 45),
            [255, 255, 255, 255],
            "outside material is unchanged"
        );
        assert_eq!(p(90, 45), [0, 255, 0, 255], "foreground remains sharp");
        assert_eq!(p(92, 45), [0, 0, 0, 255], "foreground does not bleed");
        assert_eq!(
            p(16, 16),
            [255, 255, 255, 255],
            "rounded corner preserves backdrop"
        );
        let updated = render(&scene(40., false), Color::BLACK);
        assert!(
            rgba(&updated, width, 61 * scale, 45 * scale)[0] < 8,
            "blur must sample current frame, not retained pixels"
        );
        let alpha = render(&scene(64., true), Color::TRANSPARENT);
        let red = rgba(&alpha, width, 40 * scale, 45 * scale);
        assert!(
            (red[0] as i32 - 128).abs() <= 5 && (red[3] as i32 - 128).abs() <= 5,
            "premultiplied backdrop must not darken again: {red:?}"
        );
        assert_eq!(red[1], 0);
        assert_eq!(red[2], 0);

        let mut translucent = PaintContext::new();
        translucent.fill_rounded_rect(
            Rect::from_xywh(0., 0., 128., 96.),
            0.,
            Color::rgba(1., 0., 0., 0.5),
        );
        translucent.set_opacity(0.5);
        translucent.backdrop_filter_rounded_rect(
            Rect::from_xywh(16., 16., 96., 64.),
            8.,
            12.,
            Color::rgba(0., 0., 1., 0.25),
        );
        let tinted = render(&translucent, Color::TRANSPARENT);
        let tinted = rgba(&tinted, width, 64 * scale, 48 * scale);
        for (actual, expected) in tinted.into_iter().zip([112, 0, 32, 143]) {
            assert!(
                (actual as i32 - expected).abs() <= 5,
                "tint and opacity must compose in premultiplied space: {tinted:?}"
            );
        }

        let mut overlap = PaintContext::new();
        overlap.backdrop_filter_rounded_rect(
            Rect::from_xywh(16., 16., 96., 64.),
            8.,
            12.,
            Color::rgba(1., 0., 0., 0.5),
        );
        overlap.backdrop_filter_rounded_rect(
            Rect::from_xywh(40., 20., 60., 56.),
            8.,
            12.,
            Color::rgba(0., 0., 1., 0.5),
        );
        let overlap = render(&overlap, Color::WHITE);
        let overlap = rgba(&overlap, width, 70 * scale, 48 * scale);
        for (actual, expected) in overlap.into_iter().zip([128, 64, 191, 255]) {
            assert!(
                (actual as i32 - expected).abs() <= 5,
                "second material must sample the first material: {overlap:?}"
            );
        }
        let started = Instant::now();
        for _ in 0..6 {
            render(&scene(64., false), Color::BLACK);
        }
        eprintln!(
            "backdrop {scale}x: cold {:?}; warm frame+readback average {:?}",
            cold,
            started.elapsed() / 6
        );
    }
}

#[test]
fn refractive_edges_bend_live_pixels_keep_centers_and_foreground_stable() {
    use scarlet_ui_core::{geometry::Size, renderer::CpuPaintRenderer};
    let Some(context) = context() else {
        return;
    };
    for scale in [1, 2] {
        let (width, height) = (128 * scale, 96 * scale);
        let mut renderer = SgfxPaintEncoder::new(width, height, false).unwrap();
        renderer.set_analytic_glass(true);
        let target = renderer.target_texture(0).unwrap();
        let mut resources = context.create_resources(renderer.resource_table());
        resources
            .map_image(
                target,
                context
                    .create_image(width, height, TextureFormat::Bgra8Unorm)
                    .unwrap(),
            )
            .unwrap();
        let queue = context.create_queue();
        let scene = |strength, edge| {
            let mut p = PaintContext::new();
            p.fill_rect(Rect::from_xywh(0., 0., 128., edge), Color::WHITE);
            p.refractive_glass_rounded_rect(
                Rect::from_xywh(16., 16., 96., 64.),
                12.,
                3.,
                Color::rgba(1., 1., 1., 0.1),
                strength,
            );
            p.fill_rect(Rect::from_xywh(90., 28., 2., 40.), Color::rgb(0., 1., 0.));
            p
        };
        let mut render = |paint: &PaintContext<'_>| {
            renderer
                .encode_frame(
                    &mut queue.executor(&mut resources),
                    0,
                    None,
                    paint,
                    Color::BLACK,
                    scale * 1000,
                    &[(0, 0, width, height)],
                )
                .unwrap();
            resources.read_texture(target).unwrap()
        };
        let frosted = render(&scene(0., 20.));
        let optical = render(&scene(1., 20.));
        let at = |bytes: &[u8], x, y| rgba(bytes, width, x * scale, y * scale);
        assert!(
            at(&optical, 64, 18)[0] + 20 < at(&frosted, 64, 18)[0],
            "refraction must move the backdrop, not just tint it"
        );
        assert_eq!(at(&optical, 64, 48), at(&frosted, 64, 48), "flat center");
        assert_eq!(
            at(&optical, 90, 45),
            [0, 255, 0, 255],
            "foreground remains sharp"
        );
        assert_eq!(at(&optical, 8, 18), [255; 4], "outside material");
        let updated = render(&scene(1., 26.));
        assert!(
            at(&updated, 64, 18)[0] > at(&optical, 64, 18)[0] + 20,
            "samples current backdrop"
        );
        let mut cpu = CpuPaintRenderer::new(Size::new(128., 96.), scale * 1000, Color::BLACK);
        cpu.execute(&scene(1., 20.));
        let cpu: Vec<u8> = cpu
            .buffer()
            .as_slice()
            .iter()
            .flat_map(|p| p.to_le_bytes())
            .collect();
        for (x, y) in [(64, 18), (64, 48), (90, 45)] {
            for (a, b) in at(&cpu, x, y).into_iter().zip(at(&optical, x, y)) {
                assert!(
                    a.abs_diff(b) <= 24,
                    "CPU/GPU optical mismatch at {x},{y}: {a} vs {b}"
                );
            }
        }
        let started = Instant::now();
        for _ in 0..6 {
            render(&scene(1., 20.));
        }
        eprintln!(
            "refractive {scale}x warm frame+readback average {:?}",
            started.elapsed() / 6
        );
    }
}

#[test]
fn batched_materials_keep_independent_capture_and_work_buffer_contents() {
    let Some(context) = context() else {
        return;
    };
    let mut renderer = SgfxPaintEncoder::new(240, 100, false).unwrap();
    renderer.set_analytic_glass(true);
    let target = renderer.target_texture(0).unwrap();
    let mut resources = context.create_resources(renderer.resource_table());
    resources
        .map_image(
            target,
            context
                .create_image(240, 100, TextureFormat::Bgra8Unorm)
                .unwrap(),
        )
        .unwrap();
    let queue = context.create_queue();
    for swapped in [false, true] {
        let colors = if swapped {
            [Color::rgb(0., 0., 1.), Color::rgb(1., 0., 0.)]
        } else {
            [Color::rgb(1., 0., 0.), Color::rgb(0., 0., 1.)]
        };
        let mut paint = PaintContext::new();
        for (i, color) in colors.into_iter().enumerate() {
            paint.fill_rounded_rect(Rect::from_xywh(i as f32 * 120., 0., 120., 100.), 0., color);
        }
        for (rect, blur) in [
            (Rect::from_xywh(20., 20., 70., 60.), 4.),
            (Rect::from_xywh(145., 25., 80., 50.), 8.),
        ] {
            paint.refractive_glass_rounded_rect(rect, 12., blur, Color::TRANSPARENT, 1.);
        }
        renderer
            .encode_frame(
                &mut queue.executor(&mut resources),
                0,
                None,
                &paint,
                Color::BLACK,
                1000,
                &[(0, 0, 240, 100)],
            )
            .unwrap();
        let pixels = resources.read_texture(target).unwrap();
        let expected = if swapped {
            [[0, 0, 255, 255], [255, 0, 0, 255]]
        } else {
            [[255, 0, 0, 255], [0, 0, 255, 255]]
        };
        assert_eq!(rgba(&pixels, 240, 55, 50), expected[0]);
        assert_eq!(rgba(&pixels, 240, 180, 50), expected[1]);
        assert_eq!(renderer.backdrop_fallback_count(), 0);
    }
}

#[test]
fn retained_scroll_tiles_match_direct_pixels_including_local_glass() {
    use scarlet_ui_core::geometry::Point;
    scarlet_ui_core::graphics::set_default_font(include_bytes!(
        "../../../examples/web-counter/assets/DejaVuSans.ttf"
    ))
    .unwrap();
    let Some(context) = context() else {
        return;
    };
    for (scale, analytic) in [
        (1, false),
        (2, false),
        (3, false),
        (1, true),
        (2, true),
        (3, true),
    ] {
        let (width, height) = (240 * scale, 180 * scale);
        let mut cached = SgfxPaintEncoder::new(width, height, false).unwrap();
        cached.set_analytic_glass(analytic);
        let mut direct = SgfxPaintEncoder::new(width, height, false).unwrap();
        direct.set_analytic_glass(analytic);
        let cached_target = cached.target_texture(0).unwrap();
        let direct_target = direct.target_texture(0).unwrap();
        let mut cached_resources = context.create_resources(cached.resource_table());
        let mut direct_resources = context.create_resources(direct.resource_table());
        cached_resources
            .map_image(
                cached_target,
                context
                    .create_image(width, height, TextureFormat::Bgra8Unorm)
                    .unwrap(),
            )
            .unwrap();
        direct_resources
            .map_image(
                direct_target,
                context
                    .create_image(width, height, TextureFormat::Bgra8Unorm)
                    .unwrap(),
            )
            .unwrap();
        let queue = context.create_queue();
        for glass_height in [0., 70., 1100.] {
            let mut content = PaintContext::new();
            content.fill_rect(Rect::from_xywh(-100., -100., 440., 1400.), Color::WHITE);
            for row in 0..60 {
                content.fill_rounded_rect(
                    Rect::from_xywh(10.5, row as f32 * 20., 190., 12.),
                    4.,
                    Color::rgba(0.7, 0.1, 0.25, 0.65),
                );
                content.draw_text(
                    Point::new(16.5, row as f32 * 20. + 2.5),
                    "Layer text",
                    Color::BLACK,
                    12.,
                );
            }
            content.draw_icon(
                Rect::from_xywh(16.25, 102.5, 18., 18.),
                scarlet_ui_core::icon::Icon::Home,
                scarlet_ui_core::icon::IconStyle::outline(),
                Color::BLACK,
            );
            if glass_height > 0. {
                content.refractive_glass_rounded_rect(
                    Rect::from_xywh(12., 12., 216., glass_height),
                    20.,
                    2.52,
                    Color::WHITE.with_opacity(0.35),
                    1.,
                );
            }
            if glass_height > 0. {
                // At offset -20 the first lens crosses the scrollport and
                // its capture overlaps this otherwise interior lens. Later
                // foreground must never leak from a cached final image into
                // either lens's earlier backdrop sample.
                content.refractive_glass_rounded_rect(
                    Rect::from_xywh(32., 84., 160., 40.),
                    12.,
                    2.52,
                    Color::WHITE.with_opacity(0.35),
                    1.,
                );
                content.refractive_glass_rounded_rect(
                    Rect::from_xywh(20., 170., 100., 30.),
                    12.,
                    2.52,
                    Color::WHITE.with_opacity(0.35),
                    1.,
                );
                content.fill_rect(Rect::from_xywh(48., 90., 60., 4.), Color::rgb(0., 0., 1.));
            }
            let layer = content.into_cached_layer(Rect::from_xywh(-100., -100., 440., 1400.));
            assert!(
                layer.raster_cache_bounds().is_some(),
                "glass and its opaque backing form one retained layer"
            );
            for offset in [
                0., -0.5, -1.5, -1., -211., -425., -630., -20., -20.25, -31.25, -51.25, -91.25,
            ] {
                let mut paint = PaintContext::new();
                paint.push_rounded_clip(
                    Rect::from_xywh(0., 0., 240., 180.),
                    if glass_height == 70. { 0. } else { 9. },
                );
                paint.draw_display_list(Point::new(0., offset), layer.clone());
                paint.pop_clip();
                cached
                    .encode_frame(
                        &mut queue.executor(&mut cached_resources),
                        0,
                        None,
                        &paint,
                        Color::rgb(0.2, 0.3, 0.5),
                        scale * 1000,
                        &[(0, 0, width, height)],
                    )
                    .unwrap();
                direct
                    .encode_frame(
                        &mut queue.executor(&mut direct_resources),
                        0,
                        None,
                        &paint.flattened(),
                        Color::rgb(0.2, 0.3, 0.5),
                        scale * 1000,
                        &[(0, 0, width, height)],
                    )
                    .unwrap();
                let a = cached_resources.read_texture(cached_target).unwrap();
                let b = direct_resources.read_texture(direct_target).unwrap();
                let max = a.iter().zip(&b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                assert!(
                    max <= 2,
                    "tile seam or changed paint: analytic={analytic}, scale={scale}, glass_height={glass_height}, offset={offset}, max error={max}"
                );
            }
        }
    }
}

#[test]
fn large_refractive_dock_preserves_detail_without_grid_seams() {
    use std::io::Write;
    let Some(context) = context() else {
        return;
    };
    let (width, height) = (960, 600);
    let mut renderer = SgfxPaintEncoder::new(width, height, false).unwrap();

    let target = renderer.target_texture(0).unwrap();
    let mut resources = context.create_resources(renderer.resource_table());
    resources
        .map_image(
            target,
            context
                .create_image(width, height, TextureFormat::Bgra8Unorm)
                .unwrap(),
        )
        .unwrap();
    let queue = context.create_queue();
    for analytic in [false, true] {
        renderer.set_analytic_glass(analytic);
        for shift in [0., 0.25, 0.5] {
            let mut paint = PaintContext::new();
            // The user's failure case: thin progress tracks passing behind a wide
            // floating capsule, especially its rounded upper-left edge.
            for y in [224., 233., 242.] {
                paint.fill_rounded_rect(
                    Rect::from_xywh(16., y + shift, 300., 2.),
                    1.,
                    Color::rgb(0.75, 0.1, 0.2),
                );
            }
            // Fine stripes expose a silent 256px working-image downgrade.
            for x in (330..450).step_by(8) {
                paint.fill_rounded_rect(Rect::from_xywh(x as f32, 224., 4., 46.), 0., Color::BLACK);
            }
            paint.refractive_glass_rounded_rect(
                Rect::from_xywh(12., 220., 456., 56.),
                28.,
                2.52,
                Color::WHITE.with_opacity(0.35),
                1.,
            );
            renderer
                .encode_frame(
                    &mut queue.executor(&mut resources),
                    0,
                    None,
                    &paint,
                    Color::WHITE,
                    2000,
                    &[(0, 0, width, height)],
                )
                .unwrap();
            assert_eq!(renderer.backdrop_fallback_count(), 0);
            let pixels = resources.read_texture(target).unwrap();
            let dark = rgba(&pixels, width, 664, 500)[0];
            let bright = rgba(&pixels, width, 672, 500)[0];
            assert!(
                bright > dark + 60,
                "fine background structure must survive: {dark} vs {bright}"
            );
            if let Some(dir) = std::env::var_os("SCARLET_GLASS_PROOF_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                let mut file = std::fs::File::create(
                    std::path::Path::new(&dir).join(format!("dock-{analytic}-{shift}.ppm")),
                )
                .unwrap();
                write!(file, "P6\n{width} {height}\n255\n").unwrap();
                for p in pixels.chunks_exact(4) {
                    file.write_all(&[p[2], p[1], p[0]]).unwrap();
                }
            }
        }
    }
}

#[test]
#[ignore]
fn warm_scroll_layer_costs() {
    use scarlet_ui_core::geometry::Point;
    let context = context().expect("GPU benchmark requires an adapter");
    let mut content = PaintContext::new();
    for row in 0..1000 {
        content.fill_rounded_rect(
            Rect::from_xywh(12., row as f32 * 24., 560., 20.),
            6.,
            Color::rgb(0.75, 0.8, 0.9),
        );
        content.fill_rounded_rect(
            Rect::from_xywh(24., row as f32 * 24. + 6., 280., 5.),
            2.,
            Color::rgb(0.4, 0.45, 0.55),
        );
    }
    let content = content.into_cached_layer(Rect::from_xywh(0., 0., 600., 24000.));
    for cache in [false, true] {
        let mut renderer = SgfxPaintEncoder::new(600, 400, false).unwrap();
        let target = renderer.target_texture(0).unwrap();
        let mut resources = context.create_resources(renderer.resource_table());
        resources
            .map_image(
                target,
                context
                    .create_image(600, 400, TextureFormat::Bgra8Unorm)
                    .unwrap(),
            )
            .unwrap();
        let queue = context.create_queue();
        let mut encode = std::time::Duration::ZERO;
        let mut total = std::time::Duration::ZERO;
        for frame in 0..40 {
            let mut p = PaintContext::new();
            p.push_rounded_clip(Rect::from_xywh(0., 0., 600., 400.), 0.);
            p.draw_display_list(Point::new(0., -(frame as f32)), content.clone());
            p.pop_clip();
            let flat;
            let paint = if cache {
                &p
            } else {
                flat = p.flattened();
                &flat
            };
            let start = Instant::now();
            renderer
                .encode_frame(
                    &mut queue.executor(&mut resources),
                    0,
                    None,
                    paint,
                    Color::WHITE,
                    1000,
                    &[(0, 0, 600, 400)],
                )
                .unwrap();
            let encoded = start.elapsed();
            resources.read_texture(target).unwrap();
            if frame >= 8 {
                encode += encoded;
                total += start.elapsed();
            }
        }
        eprintln!(
            "1000 rows, GPU tiles={cache}: warm encode {:?}, frame+readback {:?}",
            encode / 32,
            total / 32
        );
    }
}

/// Run explicitly with --ignored --nocapture. Reports synchronized GPU cost;
/// timing is diagnostic, never a machine-dependent pass/fail threshold.
#[test]
#[ignore]
fn warm_glass_scene_costs() {
    use scarlet_ui_core::geometry::{Offset, Point};
    let context = context().expect("GPU benchmark requires an adapter");
    let analytic = std::env::var("SCARLET_BENCH_ANALYTIC").as_deref() != Ok("0");
    eprintln!("analytic glass: {analytic}");
    let glass_count: usize = std::env::var("SCARLET_BENCH_GLASS_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .clamp(1, 16);
    let mut items = Vec::new();
    for i in 0..24 {
        let mut p = PaintContext::new();
        p.fill_rounded_rect(
            Rect::from_xywh(0., 0., 96., 24.),
            8.,
            Color::rgb(180 + i, 170, 190),
        );
        items.push(p.into_display_list());
    }
    let mut shadow = PaintContext::new();
    shadow.draw_rounded_rect_shadow(
        Rect::from_xywh(12., 330., 576., 56.),
        28.,
        Offset::new(0., 2.),
        10.,
        0.,
        Color::BLACK.with_opacity(0.10),
    );
    let shadow = shadow.into_display_list();
    for (mode, blur, refraction) in [
        ("tint", 0., 0.),
        ("frosted", 18., 0.),
        ("refractive", 10., 1.),
    ] {
        for retained in [false, true] {
            let mut renderer = SgfxPaintEncoder::new(600, 400, false).unwrap();
            renderer.set_analytic_glass(analytic);
            let target = renderer.target_texture(0).unwrap();
            let mut resources = context.create_resources(renderer.resource_table());
            resources
                .map_image(
                    target,
                    context
                        .create_image(600, 400, TextureFormat::Bgra8Unorm)
                        .unwrap(),
                )
                .unwrap();
            let queue = context.create_queue();
            let mut elapsed = std::time::Duration::ZERO;
            let mut encode_time = std::time::Duration::ZERO;
            for frame in 0..12 {
                let mut p = PaintContext::new();
                for (i, item) in items.iter().enumerate() {
                    p.draw_display_list(
                        Point::new(
                            16. + (i % 4) as f32 * 145.,
                            20. + (i / 4) as f32 * 60. - frame as f32,
                        ),
                        item.clone(),
                    );
                }
                p.draw_display_list(Point::ZERO, shadow.clone());
                let rect = Rect::from_xywh(12., 330., 576., 56.);
                if blur == 0. {
                    p.fill_rounded_rect(rect, 28., Color::WHITE.with_opacity(0.88));
                } else {
                    p.refractive_glass_rounded_rect(
                        rect,
                        28.,
                        blur,
                        Color::WHITE.with_opacity(0.52),
                        refraction,
                    );
                }
                for i in 1..glass_count {
                    let rect = Rect::from_xywh(
                        16. + (i % 4) as f32 * 145.,
                        20. + (i / 4) as f32 * 60. - frame as f32,
                        96.,
                        24.,
                    );
                    if blur == 0. {
                        p.fill_rounded_rect(rect, 12., Color::WHITE.with_opacity(0.88));
                    } else {
                        p.refractive_glass_rounded_rect(
                            rect,
                            12.,
                            blur,
                            Color::WHITE.with_opacity(0.52),
                            refraction,
                        );
                    }
                }
                let flat;
                let paint = if retained {
                    &p
                } else {
                    flat = p.flattened();
                    &flat
                };
                let start = Instant::now();
                renderer
                    .encode_frame(
                        &mut queue.executor(&mut resources),
                        0,
                        None,
                        paint,
                        Color::WHITE,
                        1000,
                        &[(0, 0, 600, 400)],
                    )
                    .unwrap();
                let encoded = start.elapsed();
                let pixels = resources.read_texture(target).unwrap();
                assert_eq!(pixels.len(), 600 * 400 * 4);
                if frame >= 4 {
                    elapsed += start.elapsed();
                    encode_time += encoded;
                }
            }
            eprintln!(
                "{mode} retained={retained}: warm encode {:?}, frame+GPU readback {:?} (600x400, 24 moving objects, shadow, {glass_count} glass panels)",
                encode_time / 8,
                elapsed / 8
            );
        }
    }
}

#[test]
fn wide_blur_does_not_alias_fine_background_stripes() {
    let Some(context) = context() else {
        return;
    };
    let queue = context.create_queue();
    let (width, height) = (320, 160);
    for analytic in [false, true] {
        let mut renderer = SgfxPaintEncoder::new(width, height, false).unwrap();
        renderer.set_analytic_glass(analytic);
        let target = renderer.target_texture(0).unwrap();
        let mut resources = context.create_resources(renderer.resource_table());
        resources
            .map_image(
                target,
                context
                    .create_image(width, height, TextureFormat::Bgra8Unorm)
                    .unwrap(),
            )
            .unwrap();
        for period in [3, 4, 5, 9] {
            let mut paint = PaintContext::new();
            for x in (0..width).step_by(period) {
                paint.fill_rect(
                    Rect::from_xywh(x as f32, 0., (period / 2) as f32, height as f32),
                    Color::BLACK,
                );
            }
            // Use the stock navigation material: the assertion bounds visible
            // residual contrast, including portable UNORM8 accumulation.
            paint.refractive_glass_rounded_rect(
                Rect::from_xywh(16., 16., 288., 128.),
                16.,
                18.,
                scarlet_ui_core::prelude::GlassMaterial::regular().tint,
                1.,
            );
            renderer
                .encode_frame(
                    &mut queue.executor(&mut resources),
                    0,
                    None,
                    &paint,
                    Color::WHITE,
                    1000,
                    &[(0, 0, width, height)],
                )
                .unwrap();
            let pixels = resources.read_texture(target).unwrap();
            let samples: Vec<_> = (80..240).map(|x| rgba(&pixels, width, x, 80)[0]).collect();
            let contrast = samples.iter().max().unwrap() - samples.iter().min().unwrap();
            assert!(
                contrast <= 4,
                "sparse blur leaves periodic stripes: analytic={analytic}, period={period}, contrast={contrast}"
            );
        }
    }
}

#[test]
fn retained_revisions_and_partial_backdrop_reconstruction_match_full_frames() {
    use scarlet_ui_core::{geometry::Point, renderer::DisplayList};
    use std::sync::Arc;
    let Some(context) = context() else {
        return;
    };
    for (scale, analytic) in [(1, false), (2, false), (3, false), (2, true)] {
        let (width, height) = (200 * scale, 720 * scale);
        let mut cached = SgfxPaintEncoder::new(width, height, false).unwrap();
        cached.set_analytic_glass(analytic);
        let mut direct = SgfxPaintEncoder::new(width, height, false).unwrap();
        direct.set_analytic_glass(analytic);
        let a_target = cached.target_texture(0).unwrap();
        let b_target = direct.target_texture(0).unwrap();
        let mut a_resources = context.create_resources(cached.resource_table());
        let mut b_resources = context.create_resources(direct.resource_table());
        a_resources
            .map_image(
                a_target,
                context
                    .create_image(width, height, TextureFormat::Bgra8Unorm)
                    .unwrap(),
            )
            .unwrap();
        b_resources
            .map_image(
                b_target,
                context
                    .create_image(width, height, TextureFormat::Bgra8Unorm)
                    .unwrap(),
            )
            .unwrap();
        let queue = context.create_queue();
        let mut bg = PaintContext::new();
        bg.fill_rect(Rect::from_xywh(-50., -50., 300., 900.), Color::WHITE);
        for row in 0..80 {
            bg.fill_rect(
                Rect::from_xywh(12., row as f32 * 10., 176., 2.),
                Color::rgba(0.1, 0.2, 0.8, 0.6),
            );
        }
        let bg = bg.into_display_list();
        let mut glass = PaintContext::new();
        for y in [40., 100., 540.] {
            glass.refractive_glass_rounded_rect(
                Rect::from_xywh(20., y, 160., 70.),
                16.,
                6.,
                Color::WHITE.with_opacity(0.3),
                1.,
            );
        }
        let glass = glass.into_display_list();
        let mut foreground = PaintContext::new();
        foreground.fill_rect(Rect::from_xywh(50., 80., 100., 3.), Color::BLACK);
        let foreground = foreground.into_display_list();
        let mut previous: Option<Arc<DisplayList>> = None;
        for step in 0..7 {
            let mut changing = PaintContext::new();
            // The changed strip reaches the first lens through its blur halo,
            // then the overlapping second lens; the distant lens is unchanged.
            changing.fill_rect(
                Rect::from_xywh(30., 32., 50. + step as f32 * 5., 5.),
                Color::rgba(0.8, step as f32 * 0.15, 0.2, 0.7),
            );
            // Changing kernel support changes the tile grid/padding. A prior
            // revision's texture must not be interpreted using the new grid.
            let glass = if step >= 5 {
                let mut glass = PaintContext::new();
                for y in [40., 100., 540.] {
                    glass.refractive_glass_rounded_rect(
                        Rect::from_xywh(20., y, 160., 70.),
                        16.,
                        if step == 5 { 10. } else { 2. },
                        Color::WHITE.with_opacity(0.3),
                        1.,
                    );
                }
                glass.into_display_list()
            } else {
                glass.clone()
            };
            let mut content = PaintContext::new();
            content.draw_display_list(Point::ZERO, bg.clone());
            content.draw_display_list(Point::ZERO, changing.into_display_list());
            content.draw_display_list(Point::ZERO, glass.clone());
            content.draw_display_list(Point::ZERO, foreground.clone());
            let bounds = Rect::from_xywh(-50., -50., 300., 900.);
            let layer = if let Some(old) = &previous {
                content.into_cached_layer_revision(bounds, old)
            } else {
                content.into_cached_layer(bounds)
            };
            assert!(layer.raster_cache_bounds().is_some());
            let mut paint = PaintContext::new();
            paint.push_rounded_clip(Rect::from_xywh(0., 0., 200., 720.), 9.);
            paint.draw_display_list(Point::new(0., -0.25), layer.clone());
            paint.pop_clip();
            let areas = if step == 0 || step >= 5 {
                vec![(0, 0, width, height)]
            } else {
                vec![(28 * scale, 29 * scale, 90 * scale, 12 * scale)]
            };
            cached
                .encode_frame(
                    &mut queue.executor(&mut a_resources),
                    0,
                    None,
                    &paint,
                    Color::WHITE,
                    scale * 1000,
                    &areas,
                )
                .unwrap();
            direct
                .encode_frame(
                    &mut queue.executor(&mut b_resources),
                    0,
                    None,
                    &paint.flattened(),
                    Color::WHITE,
                    scale * 1000,
                    &[(0, 0, width, height)],
                )
                .unwrap();
            let a = a_resources.read_texture(a_target).unwrap();
            let b = b_resources.read_texture(b_target).unwrap();
            let max = a.iter().zip(&b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            assert!(
                max <= 2,
                "revision or partial replay changed pixels: scale={scale} analytic={analytic} step={step} max={max}"
            );
            previous = Some(layer);
        }
    }
}
