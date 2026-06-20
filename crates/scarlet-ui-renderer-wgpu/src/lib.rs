use scarlet_ui_core::buffer::Buffer;
use scarlet_ui_core::color::Color;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::element::{Element, ElementId};
use scarlet_ui_core::geometry::Size;
use scarlet_ui_core::renderer::{CpuRenderer, FrameSize, PaintContext, Renderer};

pub struct WgpuRenderer {
    instance: wgpu::Instance,
    cpu: CpuRenderer,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    texture: Option<wgpu::Texture>,
    pipeline: Option<wgpu::RenderPipeline>,
    bind_group: Option<wgpu::BindGroup>,
    sampler: Option<wgpu::Sampler>,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
}

impl WgpuRenderer {
    pub fn new(size: Size, scale_milli: u32, background_color: Color) -> Self {
        let cpu = CpuRenderer::new(size, scale_milli, background_color);
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let (adapter, device, queue) = pollster::block_on(async {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: None,
                })
                .await
                .expect("failed to find a suitable wgpu adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("failed to create wgpu device");
            (adapter, device, queue)
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scarlet-ui wgpu sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            instance,
            cpu,
            adapter,
            device,
            queue,
            texture: None,
            pipeline: None,
            bind_group: None,
            sampler: Some(sampler),
            surface: None,
            config: None,
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    pub fn composite_manual(&mut self, data: &[u32], width: u32, height: u32) {
        self.upload_buffer(data, width, height);
    }

    pub fn create_surface_from_raw(
        &mut self,
        wh: raw_window_handle::RawWindowHandle,
        dh: raw_window_handle::RawDisplayHandle,
        width: u32,
        height: u32,
    ) {
        let surface = unsafe {
            self.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: dh,
                    raw_window_handle: wh,
                })
                .expect("failed to create wgpu surface from raw handles")
        };
        self.configure_surface(surface, width, height);
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn configure_surface(&mut self, surface: wgpu::Surface<'static>, width: u32, height: u32) {
        let caps = surface.get_capabilities(&self.adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&self.device, &config);
        self.surface = Some(surface);
        self.config = Some(config);
        self.pipeline = Some(self.create_pipeline(format));
    }

    pub fn resize_surface(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let Some(config) = self.config.as_mut() else {
            return;
        };
        if config.width == width && config.height == height {
            return;
        }
        config.width = width;
        config.height = height;
        surface.configure(&self.device, config);
    }

    fn create_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("scarlet-ui blit shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
            });

        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scarlet-ui blit pipeline"),
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

    pub fn upload_buffer(&mut self, data: &[u32], width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.resize_surface(width, height);

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scarlet-ui frame texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let bytes_per_row = width * 4;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group_layout = self.pipeline.as_ref().map(|p| p.get_bind_group_layout(0));
        if let Some(layout) = bind_group_layout {
            let sampler = self.sampler.as_ref().unwrap();
            self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scarlet-ui bind group"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            }));
        }

        self.texture = Some(texture);
    }

    pub fn present(&mut self) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };

        let frame = match surface.get_current_texture() {
            Ok(f) => f,
            Err(_) => return,
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("scarlet-ui render encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scarlet-ui render pass"),
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
    }
}

impl Renderer for WgpuRenderer {
    fn resize(&mut self, size: FrameSize) {
        self.cpu.resize(size);
        self.resize_surface(
            (size.width * size.scale_milli as f32 / 1000.0)
                .round()
                .max(1.0) as u32,
            (size.height * size.scale_milli as f32 / 1000.0)
                .round()
                .max(1.0) as u32,
        );
    }

    fn set_background_color(&mut self, color: Color) {
        self.cpu.set_background_color(color);
    }

    fn composite(&mut self, root: &dyn Element, dirty_ids: &[ElementId]) {
        self.cpu.composite(root, dirty_ids);
        let (data, width, height) = {
            let buf = self.cpu.buffer();
            (buf.as_slice().to_vec(), buf.width(), buf.height())
        };
        self.upload_buffer(&data, width, height);
    }

    fn render_paint(&mut self, _ctx: &PaintContext) {}

    fn buffer(&self) -> &Buffer {
        self.cpu.buffer()
    }

    fn buffer_mut(&mut self) -> &mut Buffer {
        self.cpu.buffer_mut()
    }

    fn damage(&self) -> Option<&[DamageRect]> {
        self.cpu.damage()
    }
}
