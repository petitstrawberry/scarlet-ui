//! CPU/GPU comparison of the same stock TabView and showcase backdrop.
#![cfg(not(target_arch = "wasm32"))]
use scarlet_ui_core::{
    NavigationLink, NavigationView,
    element::{Element, LayoutConstraints},
    prelude::*,
    renderer::{CpuPaintRenderer, PaintContext},
};
use scarlet_ui_renderer_sgfx::SgfxPaintEncoder;
use sgfx::ir::TextureFormat;
use sgfx_backend_wgpu::{Context, Device};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

fn context() -> Option<Context> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }))
    .or_else(|| pollster::block_on(instance.request_adapter(&Default::default())))?;
    eprintln!("Tab parity adapter: {:?}", adapter.get_info());
    if !adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::VIEW_FORMATS)
    {
        return None;
    }
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default(), None)).ok()?;
    Some(Device::new(device, queue).create_context())
}
fn paint_tree<'a>(element: &'a dyn Element, ctx: &mut PaintContext<'a>, origin: Point) {
    if let Some(render) = element.render_object() {
        render.paint(ctx, origin);
    }
    for child in element.children() {
        let p = child.position();
        paint_tree(
            child.as_ref(),
            ctx,
            Point::new(origin.x + p.x, origin.y + p.y),
        );
    }
}
fn tab_view(selected: usize, bottom: bool) -> TabView {
    TabView::with_selected_index(vec![
        TabItem::new("Mixer", || scarlet_ui_core::hstack! {
            Text::new("Ch 1").frame(72.0,42.0), Text::new("Ch 2").frame(72.0,42.0),
            Text::new("Master").frame(96.0,42.0),
        }.spacing(8.0).padding(12.0)),
        TabItem::new("Editor", || scarlet_ui_core::vstack! {
            Text::new("Region Inspector").font_size(13.0), ProgressView::new(0.52).frame(220.0,18.0),
        }.spacing(10.0).padding(12.0)),
    ], State::new(scarlet_ui_core::generate_state_id(), selected))
        .material(GlassMaterial::regular())
        .tab_bar_placement(if bottom {TabBarPlacement::Bottom} else {TabBarPlacement::Top})
}
fn tab(selected: usize, bottom: bool) -> Box<dyn Element> {
    let mut root = tab_view(selected, bottom).create_element();
    root.layout(LayoutConstraints::tight(320.0, 150.0));
    root
}
fn proof_background(ctx: &mut PaintContext<'_>) {
    let panel = Rect::from_xywh(12.0, 24.0, 344.0, 150.0);
    ctx.push_rounded_clip(panel, 20.0);
    ctx.fill_rect(panel, Color::rgb(218, 225, 237));
    let colors = [
        Color::rgb(213, 183, 181),
        Color::rgb(186, 185, 214),
        Color::rgb(168, 198, 207),
    ];
    let cell = panel.size.width / 3.0;
    for (index, color) in colors.into_iter().enumerate() {
        let x = panel.origin.x + cell * index as f32;
        ctx.fill_rect(
            Rect::from_xywh(x, panel.origin.y, cell, panel.size.height),
            color,
        );
        for line in 0..7 {
            ctx.fill_rect(
                Rect::from_xywh(
                    x + 10.0,
                    panel.origin.y + 14.0 + line as f32 * 19.0,
                    cell - 20.0,
                    1.0,
                ),
                Color::WHITE.with_opacity(0.30),
            );
        }
    }
    ctx.pop_clip();
}
fn save(path: &Path, width: u32, height: u32, bgra: &[u8]) -> io::Result<()> {
    let mut f = io::BufWriter::new(fs::File::create(path)?);
    write!(f, "P6\n{width} {height}\n255\n")?;
    for p in bgra.chunks_exact(4) {
        f.write_all(&[p[2], p[1], p[0]])?;
    }
    f.flush()
}
#[test]
fn stock_tabs_match_cpu_and_gpu_on_the_same_background() {
    let Some(context) = context() else {
        assert!(
            std::env::var_os("SCARLET_REQUIRE_GPU_TESTS").is_none(),
            "GPU comparison requested but no usable WGPU adapter"
        );
        eprintln!("SKIP: no usable WGPU adapter; Tab GPU parity unverified");
        return;
    };
    let output = std::env::var_os("SCARLET_TAB_PARITY_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &output {
        fs::create_dir_all(dir).unwrap();
    }
    for scale in [1000, 1500, 2000] {
        let width = 368 * scale / 1000;
        let height = 200 * scale / 1000;
        let mut encoder = SgfxPaintEncoder::new(width, height, false).unwrap();
        encoder.set_analytic_glass(true);
        let target = encoder.target_texture(0).unwrap();
        let mut resources = context.create_resources(encoder.resource_table());
        let image = context
            .create_image(width, height, TextureFormat::Bgra8Unorm)
            .unwrap();
        resources.map_image(target, image).unwrap();
        let queue = context.create_queue();
        for proof in [false, true] {
            for bottom in [false, true] {
                for selected in [0, 1] {
                    let root = tab(selected, bottom);
                    let mut paint = PaintContext::new();
                    if proof {
                        proof_background(&mut paint);
                    }
                    paint_tree(root.as_ref(), &mut paint, Point::new(24.0, 24.0));
                    let mut cpu =
                        CpuPaintRenderer::new(Size::new(368.0, 200.0), scale, Color::WHITE);
                    cpu.execute(&paint);
                    encoder
                        .encode_frame(
                            &mut queue.executor(&mut resources),
                            0,
                            None,
                            &paint,
                            Color::WHITE,
                            scale,
                            &[(0, 0, width, height)],
                        )
                        .unwrap();
                    assert_eq!(encoder.backdrop_fallback_count(), 0);
                    let gpu = resources.read_texture(target).unwrap();
                    let cpu: Vec<u8> = cpu
                        .buffer()
                        .as_slice()
                        .iter()
                        .flat_map(|p| p.to_le_bytes())
                        .collect();
                    let name = format!(
                        "{}-{}-selected{}-{}",
                        if proof { "proof" } else { "plain" },
                        if bottom { "bottom" } else { "top" },
                        selected,
                        scale
                    );
                    if let Some(dir) = &output {
                        save(&dir.join(format!("{name}-cpu.ppm")), width, height, &cpu).unwrap();
                        save(&dir.join(format!("{name}-gpu.ppm")), width, height, &gpu).unwrap();
                    }
                    let tab_y = if bottom { 138 } else { 24 };
                    // Follow the compact bar's geometry; the former fixed samples
                    // now intersect selected rounded corners, not material interiors.
                    let bar = paint.commands().iter().find_map(|command| match command {
                        scarlet_ui_core::renderer::PaintCommand::BackdropFilterRoundedRect { rect, .. } => Some(*rect),
                        _ => None,
                    }).unwrap();
                    let mut max_delta = 0u8;
                    for (fx, fy) in [
                        (0.25, 6.0),
                        (0.75, 6.0),
                        (0.25, bar.size.height - 6.0),
                        (0.75, bar.size.height - 6.0),
                    ] {
                        let x =
                            ((bar.origin.x + bar.size.width * fx) * scale as f32 / 1000.0) as u32;
                        let y = ((bar.origin.y + fy) * scale as f32 / 1000.0) as u32;
                        let at = ((y * width + x) * 4) as usize;
                        for c in 0..3 {
                            max_delta = max_delta.max(cpu[at + c].abs_diff(gpu[at + c]));
                        }
                    }
                    if !proof {
                        // Content tabs sit within a surface without elevation.
                        let x = 200 * scale / 1000;
                        let y = (tab_y + 37) * scale / 1000;
                        let at = ((y * width + x) * 4) as usize;
                        assert_eq!(cpu[at], 255, "{name}: no tab shadow on content");
                        assert_eq!(gpu[at], 255, "{name}: no tab shadow on content");
                    }
                    eprintln!("{name}: material interior max channel delta {max_delta}");
                    assert!(
                        max_delta <= 12,
                        "{name}: CPU/GPU material color mismatch {max_delta}"
                    );
                }
            }
        }
    }
}

fn navigation_scene(width: u32, height: u32) -> Box<dyn Element> {
    let links = (
        NavigationLink::new("Overview", || Text::new("Overview")).icon(Icon::Home),
        NavigationLink::new("Controls", || Text::new("Controls")).icon(Icon::Adjustments),
        NavigationLink::new("Inputs", || Text::new("Inputs")).icon(Icon::FileText),
        NavigationLink::new("Display", || {
            scarlet_ui_core::vstack! {
                Text::new("Display").font_size(24.0),
                Text::new("Layouts and content").font_size(14.0),
                tab_view(0, false).frame(320.0, 150.0),
                scarlet_ui_core::hstack! {
                    Button::new("Save changes").material(GlassMaterial::regular()),
                    Button::new("Preview").material(GlassMaterial::regular()),
                }.spacing(12.0),
            }
            .spacing(20.0)
            .padding(20.0)
        })
        .icon(Icon::Photo),
        NavigationLink::new("SGFX", || Text::new("SGFX")).icon(Icon::Package),
    );
    let view = NavigationView::new(links)
        .presentation(NavigationPresentation::Automatic)
        .sidebar_style(NavigationSidebarStyle::Floating)
        .shows_icons(true)
        .minimum_content_size(Size::new(320.0, 0.0));
    view.selected_index_state().set(3);
    let mut root = view.create_element();
    root.layout(LayoutConstraints::tight(width as f32, height as f32));
    root
}

#[test]
fn responsive_navigation_renders_rounded_glass_in_portrait_and_landscape() {
    let Some(context) = context() else {
        assert!(
            std::env::var_os("SCARLET_REQUIRE_GPU_TESTS").is_none(),
            "GPU comparison requested but no usable WGPU adapter"
        );
        eprintln!("SKIP: no usable WGPU adapter; navigation GPU parity unverified");
        return;
    };
    let output = std::env::var_os("SCARLET_TAB_PARITY_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &output {
        fs::create_dir_all(dir).unwrap();
    }
    for analytic in [false, true] {
        for proof in [false, true] {
            for (width, height) in [(390, 640), (860, 560)] {
                for scale in [1000, 2000] {
                    let pixel_width = width * scale / 1000;
                    let pixel_height = height * scale / 1000;
                    let root = navigation_scene(width, height);
                    let mut paint = PaintContext::new();
                    if proof {
                        let colors = [
                            Color::rgb(213, 183, 181),
                            Color::rgb(186, 185, 214),
                            Color::rgb(168, 198, 207),
                        ];
                        for (index, color) in colors.into_iter().enumerate() {
                            paint.fill_rect(
                                Rect::from_xywh(
                                    index as f32 * width as f32 / 3.,
                                    0.,
                                    width as f32 / 3.,
                                    height as f32,
                                ),
                                color,
                            );
                        }
                        for y in (0..height).step_by(19) {
                            paint.fill_rect(
                                Rect::from_xywh(0., y as f32, width as f32, 1.),
                                Color::WHITE.with_opacity(0.3),
                            );
                        }
                    }
                    paint_tree(root.as_ref(), &mut paint, Point::ZERO);
                    let mut cpu = CpuPaintRenderer::new(
                        Size::new(width as f32, height as f32),
                        scale,
                        Color::WHITE,
                    );
                    cpu.execute(&paint);
                    let mut encoder =
                        SgfxPaintEncoder::new(pixel_width, pixel_height, false).unwrap();
                    encoder.set_analytic_glass(analytic);
                    let target = encoder.target_texture(0).unwrap();
                    let mut resources = context.create_resources(encoder.resource_table());
                    let image = context
                        .create_image(pixel_width, pixel_height, TextureFormat::Bgra8Unorm)
                        .unwrap();
                    resources.map_image(target, image).unwrap();
                    let queue = context.create_queue();
                    encoder
                        .encode_frame(
                            &mut queue.executor(&mut resources),
                            0,
                            None,
                            &paint,
                            Color::WHITE,
                            scale,
                            &[(0, 0, pixel_width, pixel_height)],
                        )
                        .unwrap();
                    assert_eq!(encoder.backdrop_fallback_count(), 0);
                    let gpu = resources.read_texture(target).unwrap();
                    let cpu: Vec<u8> = cpu
                        .buffer()
                        .as_slice()
                        .iter()
                        .flat_map(|p| p.to_le_bytes())
                        .collect();
                    let name = format!("navigation-{width}x{height}-{scale}-{analytic}-{proof}");
                    if let Some(dir) = &output {
                        save(
                            &dir.join(format!("{name}-cpu.ppm")),
                            pixel_width,
                            pixel_height,
                            &cpu,
                        )
                        .unwrap();
                        save(
                            &dir.join(format!("{name}-gpu.ppm")),
                            pixel_width,
                            pixel_height,
                            &gpu,
                        )
                        .unwrap();
                    }
                    let (x, y) = if width < height {
                        (20, height - 12)
                    } else {
                        (100, height - 40)
                    };
                    let at = (((y * scale / 1000) * pixel_width + x * scale / 1000) * 4) as usize;
                    for c in 0..3 {
                        assert!(
                            cpu[at + c].abs_diff(gpu[at + c]) <= 12,
                            "{name}: CPU/GPU material mismatch"
                        );
                    }
                    assert_ne!(
                        &gpu[at..at + 3],
                        &[255, 255, 255],
                        "{name}: navigation surface must remain visible on white"
                    );
                }
            }
        }
    }
}

/// A popup must keep labels readable even when the page changes from white to
/// black underneath it. Exercise the portable Scarlet/Web path as well as WGSL.
#[test]
fn menu_background_contrast_is_bounded_in_both_render_paths() {
    use scarlet_ui_core::{
        element::ElementRenderObject,
        views::menu::{MenuItemContent, MenuRenderObject},
    };
    let Some(context) = context() else {
        assert!(std::env::var_os("SCARLET_REQUIRE_GPU_TESTS").is_none());
        return;
    };
    scarlet_ui_core::graphics::set_default_font(include_bytes!(
        "../../../examples/web-counter/assets/DejaVuSans.ttf"
    ))
    .unwrap();
    let output = std::env::var_os("SCARLET_TAB_PARITY_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &output {
        fs::create_dir_all(dir).unwrap();
    }
    let mut menu = MenuRenderObject::new_adaptive(
        vec![
            MenuItemContent::new("Slider preset"),
            MenuItemContent::new("Set sample text").shortcut("Ctrl+L"),
        ],
        220.,
    );
    let size = menu.layout(LayoutConstraints::unconstrained());
    for analytic in [false, true] {
        for scale in [1000, 1500, 2000] {
            let (width, height) = (480 * scale / 1000, 180 * scale / 1000);
            let mut encoder = SgfxPaintEncoder::new(width, height, false).unwrap();
            encoder.set_analytic_glass(analytic);
            let target = encoder.target_texture(0).unwrap();
            let mut resources = context.create_resources(encoder.resource_table());
            resources
                .map_image(
                    target,
                    context
                        .create_image(width, height, TextureFormat::Bgra8Unorm)
                        .unwrap(),
                )
                .unwrap();
            let queue = context.create_queue();
            let mut frames = Vec::new();
            for (name, background) in [("light", Color::WHITE), ("dark", Color::BLACK)] {
                let mut paint = PaintContext::new();
                paint.draw_text(
                    Point::new(80., 48.),
                    "Widget Factory",
                    Color::rgb(70, 70, 75),
                    32.,
                );
                paint.draw_text(
                    Point::new(80., 90.),
                    "PaintCommand default rendering",
                    Color::rgb(70, 70, 75),
                    16.,
                );
                menu.paint(&mut paint, Point::new(40., 40.));
                encoder
                    .encode_frame(
                        &mut queue.executor(&mut resources),
                        0,
                        None,
                        &paint,
                        background,
                        scale,
                        &[(0, 0, width, height)],
                    )
                    .unwrap();
                assert_eq!(encoder.backdrop_fallback_count(), 0);
                let pixels = resources.read_texture(target).unwrap();
                if let Some(dir) = &output {
                    save(
                        &dir.join(format!("menu-{analytic}-{scale}-{name}.ppm")),
                        width,
                        height,
                        &pixels,
                    )
                    .unwrap();
                }
                frames.push(pixels);
            }
            let mut max_delta = 0;
            for y in (48 * scale / 1000)..((40. + size.height - 8.) * scale as f32 / 1000.) as u32 {
                for x in (48 * scale / 1000)..(252 * scale / 1000) {
                    let at = ((y * width + x) * 4) as usize;
                    for c in 0..3 {
                        max_delta = max_delta.max(frames[0][at + c].abs_diff(frames[1][at + c]));
                    }
                }
            }
            assert!(
                max_delta <= 35,
                "background competes with menu labels: analytic={analytic}, scale={scale}, contrast={max_delta}"
            );
        }
    }
}

/// Strong contour strokes are especially visible on dark glass. Compare the
/// entire shoulder and the finer horizontal rims, including corners, in both
/// SGFX paths. Fractional origins cover the search bar and dock reference shapes.
#[test]
fn shoulder_reflection_stays_soft_and_matches_cpu() {
    use scarlet_ui_core::{
        color::ColorPalette,
        glass::{glass_bezel, glass_edge},
    };
    let Some(context) = context() else {
        assert!(std::env::var_os("SCARLET_REQUIRE_GPU_TESTS").is_none());
        return;
    };
    let output = std::env::var_os("SCARLET_TAB_PARITY_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &output {
        fs::create_dir_all(dir).unwrap();
    }
    for (name, background, tint) in [
        (
            "dark",
            Color::rgb(24, 24, 24),
            ColorPalette::dark().surface(),
        ),
        (
            "light",
            Color::rgb(180, 180, 180),
            ColorPalette::light().surface(),
        ),
    ] {
        for (shape, rect, radius) in [
            ("folder", Rect::from_xywh(104., 24., 112., 112.), 24.),
            ("search", Rect::from_xywh(24.25, 52.25, 272., 55.5), 27.75),
            ("dock", Rect::from_xywh(24.5, 44.5, 272., 71.), 32.),
        ] {
            for scale in [1000, 1500, 2000] {
                let (width, height) = (320 * scale / 1000, 160 * scale / 1000);
                let mut paint = PaintContext::new();
                paint.refractive_glass_rounded_rect(
                    rect,
                    radius,
                    2.52,
                    tint.with_opacity(0.64),
                    1.,
                );
                let mut cpu = CpuPaintRenderer::new(Size::new(320., 160.), scale, background);
                cpu.execute(&paint);
                let cpu: Vec<u8> = cpu
                    .buffer()
                    .as_slice()
                    .iter()
                    .flat_map(|p| p.to_le_bytes())
                    .collect();
                for analytic in [false, true] {
                    let mut encoder = SgfxPaintEncoder::new(width, height, false).unwrap();
                    encoder.set_analytic_glass(analytic);
                    let target = encoder.target_texture(0).unwrap();
                    let mut resources = context.create_resources(encoder.resource_table());
                    resources
                        .map_image(
                            target,
                            context
                                .create_image(width, height, TextureFormat::Bgra8Unorm)
                                .unwrap(),
                        )
                        .unwrap();
                    let queue = context.create_queue();
                    encoder
                        .encode_frame(
                            &mut queue.executor(&mut resources),
                            0,
                            None,
                            &paint,
                            background,
                            scale,
                            &[(0, 0, width, height)],
                        )
                        .unwrap();
                    assert_eq!(encoder.backdrop_fallback_count(), 0);
                    let gpu = resources.read_texture(target).unwrap();
                    let center = ((height / 2 * width + width / 2) * 4) as usize;
                    let mut max_delta = 0;
                    let mut max_lift = 0;
                    for y in 0..height {
                        for x in 0..width {
                            let point = Point::new(
                                (x as f32 + 0.5) * 1000. / scale as f32,
                                (y as f32 + 0.5) * 1000. / scale as f32,
                            );
                            let (distance, _, _) = glass_edge(rect, radius, point);
                            if distance * scale as f32 / 1000. < 0.5
                                || distance >= glass_bezel(rect, radius)
                            {
                                continue;
                            }
                            let at = ((y * width + x) * 4) as usize;
                            for c in 0..3 {
                                max_delta = max_delta.max(cpu[at + c].abs_diff(gpu[at + c]));
                                if distance >= 3. {
                                    max_lift =
                                        max_lift.max(gpu[at + c].saturating_sub(gpu[center + c]));
                                }
                            }
                        }
                    }
                    if name == "light" {
                        let factor = scale as f32 / 1000.;
                        let side = ((rect.left() * factor).ceil() as u32
                            ..((rect.left() + 3.) * factor) as u32)
                            .map(|x| gpu[((height / 2 * width + x) * 4) as usize])
                            .min()
                            .unwrap();
                        let top = ((rect.top() * factor).ceil() as u32
                            ..((rect.top() + 3.) * factor) as u32)
                            .map(|y| gpu[((y * width + width / 2) * 4) as usize])
                            .min()
                            .unwrap();
                        assert!(side + 3 < gpu[center], "sides need a compact dark contour");
                        assert!(
                            side + 3 < top,
                            "side contours must be firmer than horizontal edges"
                        );
                    }
                    let label = format!("shoulder-{shape}-{name}-{scale}-{analytic}");
                    if name == "light" {
                        let factor = scale as f32 / 1000.;
                        for (edge, inward) in [(rect.top(), 1.), (rect.bottom(), -1.)] {
                            let boundary = edge * factor;
                            let shadow = (boundary.floor() as u32..=boundary.ceil() as u32 + 1)
                                .map(|y| gpu[((y * width + width / 2) * 4) as usize])
                                .min()
                                .unwrap();
                            assert!(
                                shadow + 2 < gpu[center],
                                "{label}: faint shadow at {edge}, inward={inward}"
                            );
                        }
                    }
                    eprintln!("{label}: CPU/GPU delta {max_delta}, reflection lift {max_lift}");
                    if let Some(dir) = &output {
                        save(&dir.join(format!("{label}-gpu.ppm")), width, height, &gpu).unwrap();
                    }
                    assert!(
                        max_lift <= 40,
                        "{label}: the broad shoulder must stay soft outside the fine rim"
                    );
                    assert!(
                        max_delta <= 14,
                        "{label}: shoulder differs between renderers"
                    );
                }
            }
        }
    }
}

/// Manual proof for comparing contour treatments on identical colored content.
#[test]
#[ignore = "manual GPU readback for contour review"]
fn contour_review_scene() {
    let context = context().expect("contour review requires a GPU adapter");
    let output = std::path::PathBuf::from(
        std::env::var_os("SCARLET_TAB_PARITY_DIR").expect("set SCARLET_TAB_PARITY_DIR"),
    );
    fs::create_dir_all(&output).unwrap();
    scarlet_ui_core::graphics::set_default_font(include_bytes!(
        "../../../examples/web-counter/assets/DejaVuSans.ttf"
    ))
    .unwrap();
    let mut paint = PaintContext::new();
    paint.fill_vertical_gradient_rounded_rect(
        Rect::from_xywh(0., 0., 353., 300.),
        0.,
        Color::rgb(124, 211, 244),
        Color::rgb(149, 160, 204),
    );
    paint.fill_rounded_rect(
        Rect::from_xywh(-25., 100., 110., 220.),
        48.,
        Color::rgb(226, 236, 246),
    );
    paint.fill_rounded_rect(
        Rect::from_xywh(150., 90., 145., 240.),
        64.,
        Color::rgb(57, 64, 91),
    );
    paint.fill_rounded_rect(
        Rect::from_xywh(235., 187., 160., 138.),
        48.,
        Color::rgb(206, 161, 181),
    );
    for y in [51., 159., 240., 270.] {
        paint.fill_rect(
            Rect::from_xywh(0., y, 353., 1.),
            Color::WHITE.with_opacity(0.4),
        );
    }
    for (rect, radius, tint) in [
        (
            Rect::from_xywh(23., 16., 307., 42.),
            21.,
            Color::WHITE.with_opacity(0.18),
        ),
        (
            Rect::from_xywh(23., 78., 142., 142.),
            24.,
            Color::WHITE.with_opacity(0.18),
        ),
        (
            Rect::from_xywh(188., 78., 142., 142.),
            24.,
            Color::WHITE.with_opacity(0.18),
        ),
        (
            Rect::from_xywh(15., 236., 323., 52.),
            26.,
            GlassMaterial::regular().tint,
        ),
    ] {
        paint.refractive_glass_rounded_rect(rect, radius, 2.52, tint, 1.);
    }
    paint.draw_text(Point::new(147., 28.), "Search", Color::WHITE, 16.);
    for origin in [Point::new(36., 91.), Point::new(201., 91.)] {
        for (i, color) in [
            Color::rgb(233, 238, 245),
            Color::rgb(52, 126, 232),
            Color::rgb(45, 181, 109),
            Color::rgb(36, 41, 51),
        ]
        .into_iter()
        .enumerate()
        {
            let rect = Rect::from_xywh(
                origin.x + (i % 2) as f32 * 63.,
                origin.y + (i / 2) as f32 * 63.,
                53.,
                53.,
            );
            paint.fill_rounded_rect(rect, 12., color);
        }
    }
    for (i, color) in [
        Color::rgb(230, 139, 190),
        Color::rgb(222, 226, 234),
        Color::rgb(22, 25, 31),
        Color::rgb(57, 160, 232),
    ]
    .into_iter()
    .enumerate()
    {
        paint.fill_rounded_rect(
            Rect::from_xywh(39. + i as f32 * 80., 244., 36., 36.),
            10.,
            color,
        );
    }
    for scale in [1000, 1500, 2000] {
        let (width, height) = (353 * scale / 1000, 300 * scale / 1000);
        for analytic in [false, true] {
            let mut encoder = SgfxPaintEncoder::new(width, height, false).unwrap();
            encoder.set_analytic_glass(analytic);
            let target = encoder.target_texture(0).unwrap();
            let mut resources = context.create_resources(encoder.resource_table());
            resources
                .map_image(
                    target,
                    context
                        .create_image(width, height, TextureFormat::Bgra8Unorm)
                        .unwrap(),
                )
                .unwrap();
            let queue = context.create_queue();
            encoder
                .encode_frame(
                    &mut queue.executor(&mut resources),
                    0,
                    None,
                    &paint,
                    Color::WHITE,
                    scale,
                    &[(0, 0, width, height)],
                )
                .unwrap();
            assert_eq!(encoder.backdrop_fallback_count(), 0);
            let gpu = resources.read_texture(target).unwrap();
            save(
                &output.join(format!("review-{scale}-{analytic}.ppm")),
                width,
                height,
                &gpu,
            )
            .unwrap();
        }
    }
}
