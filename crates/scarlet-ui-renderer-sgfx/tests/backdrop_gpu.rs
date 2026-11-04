//! Headless execution/readback coverage for the portable fixed-pipeline blur.
#![cfg(not(target_arch = "wasm32"))]
use scarlet_ui_core::{color::Color, geometry::Rect, renderer::PaintContext};
use scarlet_ui_renderer_sgfx::SgfxPaintEncoder;
use sgfx::ir::TextureFormat;
use sgfx_backend_wgpu::{Context, Device};
use std::time::Instant;

fn context() -> Option<Context> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
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
        eprintln!("SKIP: no headless WGPU adapter; pixel correctness has not been verified");
        return;
    };
    for scale in [1, 2] {
        let (width, height) = (128 * scale, 96 * scale);
        let mut renderer = SgfxPaintEncoder::new(width, height, false).unwrap();
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
