//! Winit-owned WGPU surface presentation for the SGFX renderer.

use scarlet_ui_core::color::Color;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::geometry::{Rect, Size};
use scarlet_ui_core::renderer::{BackendFrame, PaintBackend, PaintContext};
use scarlet_ui_core::{Error, Result};
use scarlet_ui_renderer_sgfx::WgpuSgfxSession;

/// Platform composition of an SGFX/WGPU session and a Winit WGPU surface.
pub(crate) struct WgpuPaintBackend {
    surface: wgpu::Surface<'static>,
    /// Keep the instance alive until after the surface is dropped.
    _instance: wgpu::Instance,
    config: wgpu::SurfaceConfiguration,
    session: WgpuSgfxSession,
    sampler: wgpu::Sampler,
    blit_pipeline: wgpu::RenderPipeline,
    supports_depth: bool,
    scale_milli: u32,
}

impl WgpuPaintBackend {
    /// Compose an SGFX/WGPU renderer with a platform-owned presentation surface.
    ///
    /// # Arguments
    ///
    /// * `instance` - WGPU instance that created `surface`.
    /// * `surface` - Winit surface used only by this platform adapter.
    /// * `device` - WGPU device used for rendering and presentation.
    /// * `queue` - Queue paired with `device`.
    /// * `config` - Initial presentation configuration.
    /// * `width` - Initial physical width in pixels.
    /// * `height` - Initial physical height in pixels.
    /// * `supports_depth` - Whether canvas depth passes may be requested.
    ///
    /// # Returns
    ///
    /// A configured platform renderer, or a rendering error.
    pub(crate) fn new(
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        mut config: wgpu::SurfaceConfiguration,
        width: u32,
        height: u32,
        supports_depth: bool,
    ) -> Result<Self> {
        let width = width.max(1);
        let height = height.max(1);
        config.width = width;
        config.height = height;
        let session = WgpuSgfxSession::new(device, queue, width, height, supports_depth)
            .map_err(|_| Error::RenderError)?;
        surface.configure(session.raw_device(), &config);
        let sampler = session
            .raw_device()
            .create_sampler(&wgpu::SamplerDescriptor {
                label: Some("scarlet-ui sgfx wgpu blit sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            });
        let blit_pipeline = create_blit_pipeline(session.raw_device(), config.format);
        Ok(Self {
            surface,
            _instance: instance,
            config,
            session,
            sampler,
            blit_pipeline,
            supports_depth,
            scale_milli: 1_000,
        })
    }

    fn resize_physical(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.config.width == width && self.config.height == height {
            return;
        }
        if self
            .session
            .resize(width, height, self.supports_depth)
            .is_err()
        {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface
            .configure(self.session.raw_device(), &self.config);
    }

    fn present(&mut self) -> Result<()> {
        let image = self.session.image().ok_or(Error::RenderError)?;
        let frame = self
            .surface
            .get_current_texture()
            .map_err(|_| Error::RenderError)?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self
            .session
            .raw_device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scarlet-ui sgfx wgpu blit bind group"),
                layout: &self.blit_pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(image.raw_view()),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
        let mut encoder =
            self.session
                .raw_device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("scarlet-ui sgfx wgpu presentation encoder"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scarlet-ui sgfx wgpu presentation pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.session
            .raw_queue()
            .submit(core::iter::once(encoder.finish()));
        frame.present();
        let _ = self.session.raw_device().poll(wgpu::Maintain::Poll);
        Ok(())
    }
}

impl PaintBackend for WgpuPaintBackend {
    fn resize(&mut self, size: Size, scale_milli: u32) {
        self.scale_milli = scale_milli.max(1);
        self.resize_physical(
            physical_dimension(size.width, scale_milli),
            physical_dimension(size.height, scale_milli),
        );
    }

    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        background_color: Color,
        _logical_damage: Option<&[Rect]>,
        physical_damage: Option<&[DamageRect]>,
    ) -> Result<BackendFrame<'a>> {
        self.session
            .render_with_damage(context, background_color, self.scale_milli, physical_damage)
            .map_err(|error| {
                eprintln!("[ScarletUI] SGFX/WGPU render failed: {error}");
                Error::RenderError
            })?;
        self.present().map_err(|error| {
            eprintln!("[ScarletUI] SGFX/WGPU present failed: {error}");
            Error::RenderError
        })?;
        Ok(BackendFrame::External)
    }
}

fn physical_dimension(value: f32, scale_milli: u32) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 1;
    }
    let logical = value as u32;
    if logical == 0 {
        return 1;
    }
    let scale = u64::from(scale_milli.max(1));
    ((u64::from(logical).saturating_mul(scale).saturating_add(999) / 1_000)
        .min(u64::from(u32::MAX))
        .max(1)) as u32
}

fn create_blit_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scarlet-ui sgfx wgpu blit shader"),
        source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("scarlet-ui sgfx wgpu blit pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

const BLIT_SHADER: &str = r#"
struct VertexOut {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOut {
    var positions = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f( 3.0, -1.0),
        vec2f(-1.0,  3.0),
    );
    var uvs = array<vec2f, 3>(
        vec2f(0.0, 1.0),
        vec2f(2.0, 1.0),
        vec2f(0.0, -1.0),
    );
    var out: VertexOut;
    out.position = vec4f(positions[vid], 0.0, 1.0);
    out.uv = uvs[vid];
    return out;
}

@group(0) @binding(0) var t_frame: texture_2d<f32>;
@group(0) @binding(1) var s_frame: sampler;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    return textureSample(t_frame, s_frame, in.uv);
}
"#;
