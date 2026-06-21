use scarlet_ui::preview::PreviewHost;
use scarlet_ui_renderer_wgpu::WgpuRenderer;

pub fn setup_gpu_present(host: &mut PreviewHost) {
    let wh = host.window().raw_window_handle();
    let dh = host.window().raw_display_handle();
    let (wh, dh) = match (wh, dh) {
        (Some(w), Some(d)) => (w, d),
        _ => {
            eprintln!("[preview] --gpu: raw window handle unavailable, falling back to CPU");
            return;
        }
    };

    let size = host.window().size();
    let scale_milli = host.window().output_scale_milli();
    let (phys_w, phys_h) = host.window().physical_size();

    let mut renderer = WgpuRenderer::new(
        scarlet_ui::geometry::Size::new(size.width, size.height),
        scale_milli,
        scarlet_ui::color::Color::rgb(255, 255, 255),
    );
    renderer.create_surface_from_raw(wh, dh, phys_w, phys_h);

    host.set_gpu_present(Box::new(move |buffer, damage| {
        if damage.is_some_and(|damage| damage.is_empty()) {
            return;
        }
        renderer.composite_manual_with_damage(
            buffer.as_slice(),
            buffer.width(),
            buffer.height(),
            damage,
        );
        renderer.present();
    }));

    println!("[preview] GPU rendering enabled (wgpu)");
}
