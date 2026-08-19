//! WGPU adapter for the shared ScarletUI-to-SGFX IR lowering.

use alloc::rc::Rc;
use alloc::sync::Arc;
use alloc::vec::Vec;

use scarlet_ui_core::color::Color;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::geometry::{Rect, Size};
use scarlet_ui_core::renderer::{BackendFrame, PaintBackend, PaintContext};
use sgfx::ir::{CommandBuffer, ResourceTable, TextureId};

use crate::error::{Error, Result, Stage};
use crate::geometry::PixelBounds;
use crate::lowering::{RenderBackend, RenderSession};

struct WgpuBackend;

impl RenderBackend for WgpuBackend {
    type Context = sgfx::wgpu::Context;
    type Queue = sgfx::wgpu::Queue;
    type Image = sgfx::wgpu::Image;
    type ImageHandle = Arc<sgfx::wgpu::Image>;
    type Resources = sgfx::wgpu::Resources;

    fn create_image(context: &Self::Context, width: u32, height: u32) -> Result<Self::ImageHandle> {
        context
            .create_image(width, height, sgfx::ir::TextureFormat::Bgra8Unorm)
            .map_err(|_| Error::sgfx(Stage::CreateSharedImage))
    }

    fn create_resources(
        context: &Self::Context,
        resources: Rc<ResourceTable>,
    ) -> Result<Self::Resources> {
        Ok(context.create_resources(resources))
    }

    fn map_image(
        resources: &mut Self::Resources,
        texture: TextureId,
        image: Self::ImageHandle,
    ) -> Result<()> {
        resources
            .map_image(texture, image)
            .map_err(|_| Error::sgfx(Stage::MapSharedImage))
    }

    fn image_ref(image: &Self::ImageHandle) -> &Self::Image {
        image.as_ref()
    }

    fn submit<'r, 'data>(
        _context: &Self::Context,
        queue: &Self::Queue,
        resources: &mut Self::Resources,
        commands: &CommandBuffer<'r, 'data>,
    ) -> Result<()> {
        queue
            .submit(resources, commands)
            .map_err(|_| Error::sgfx(Stage::SubmitCommands))
    }
}

/// Persistent SGFX IR session backed by a caller-owned WGPU device and queue.
///
/// The session shares the same paint lowering as the native SWS renderer,
/// including text glyph atlases and `SgfxCanvas` draw lists. Presentation is
/// intentionally left to the caller: [`Self::image`] exposes the rendered
/// logical target so a platform adapter can copy or sample it into its WGPU
/// surface.
pub struct WgpuSgfxSession {
    context: sgfx::wgpu::Context,
    queue: sgfx::wgpu::Queue,
    session: RenderSession<WgpuBackend>,
    width: u32,
    height: u32,
    next_slot: usize,
    last_slot: Option<usize>,
}

impl WgpuSgfxSession {
    /// Create a WGPU-backed SGFX paint session.
    ///
    /// # Arguments
    ///
    /// * `device` - WGPU device used for SGFX resource materialization.
    /// * `queue` - WGPU queue paired with `device`.
    /// * `width` - Physical target width in pixels.
    /// * `height` - Physical target height in pixels.
    /// * `supports_depth` - Whether canvas depth passes may be requested.
    ///
    /// # Returns
    ///
    /// A persistent session or a lowering/resource initialization error.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        width: u32,
        height: u32,
        supports_depth: bool,
    ) -> Result<Self> {
        let device = sgfx::wgpu::Device::new(device, queue);
        let context = device.create_context();
        let queue = context.create_queue();
        let session = RenderSession::new(&context, width, height, supports_depth)?;
        Ok(Self {
            context,
            queue,
            session,
            width,
            height,
            next_slot: 0,
            last_slot: None,
        })
    }

    /// Borrow the underlying WGPU device for presentation integration.
    ///
    /// # Returns
    ///
    /// The device supplied to [`Self::new`].
    pub fn raw_device(&self) -> &wgpu::Device {
        self.context.raw_device()
    }

    /// Borrow the underlying WGPU queue for presentation integration.
    ///
    /// # Returns
    ///
    /// The queue supplied to [`Self::new`].
    pub fn raw_queue(&self) -> &wgpu::Queue {
        self.context.raw_queue()
    }

    /// Resize the persistent SGFX targets.
    ///
    /// # Arguments
    ///
    /// * `width` - New physical target width in pixels.
    /// * `height` - New physical target height in pixels.
    /// * `supports_depth` - Whether canvas depth passes may be requested.
    ///
    /// # Returns
    ///
    /// Success after allocating the new target generation, or a rendering
    /// error when the dimensions are invalid or resources cannot be created.
    pub fn resize(&mut self, width: u32, height: u32, supports_depth: bool) -> Result<()> {
        if width == self.width && height == self.height {
            return Ok(());
        }
        let session = RenderSession::new(&self.context, width, height, supports_depth)?;
        self.session = session;
        self.width = width;
        self.height = height;
        self.next_slot = 0;
        self.last_slot = None;
        Ok(())
    }

    /// Render one complete physical frame through SGFX IR.
    ///
    /// The caller remains responsible for presenting the returned target image
    /// through its platform surface. Use [`Self::render_with_damage`] when the
    /// paint context contains only a physical damage subset.
    ///
    /// # Arguments
    ///
    /// * `paint` - Backend-neutral paint command list.
    /// * `background` - Straight-alpha clear color.
    /// * `scale_milli` - Physical output scale in milli-units.
    ///
    /// # Returns
    ///
    /// Success after WGPU command submission, or a rendering error.
    pub fn render(
        &mut self,
        paint: &PaintContext<'_>,
        background: Color,
        scale_milli: u32,
    ) -> Result<()> {
        self.render_with_damage(paint, background, scale_milli, None)
    }

    /// Render a frame while preserving pixels outside physical damage.
    ///
    /// A partial [`PaintContext`] contains only commands intersecting the
    /// damage regions. Partial frames reuse the current offscreen image so
    /// pixels outside damage remain intact without copying the full target.
    /// This is safe for WGPU because the SGFX render and presentation blit are
    /// submitted to the same ordered queue.
    ///
    /// # Arguments
    ///
    /// * `paint` - Backend-neutral paint command list.
    /// * `background` - Straight-alpha clear color.
    /// * `scale_milli` - Physical output scale in milli-units.
    /// * `physical_damage` - Physical regions to redraw, or `None` for a full
    ///   frame.
    ///
    /// # Returns
    ///
    /// Success after WGPU command submission, or a rendering error.
    pub fn render_with_damage(
        &mut self,
        paint: &PaintContext<'_>,
        background: Color,
        scale_milli: u32,
        physical_damage: Option<&[DamageRect]>,
    ) -> Result<()> {
        let render_areas = self.render_areas(physical_damage)?;
        let slot = select_render_slot(
            physical_damage,
            self.next_slot,
            self.last_slot,
            &render_areas,
            self.full_bounds(),
        )?;
        self.session.render(
            &self.context,
            &self.queue,
            slot,
            None,
            paint,
            background,
            scale_milli,
            &render_areas,
        )?;
        self.next_slot = (slot + 1) % 2;
        self.last_slot = Some(slot);
        Ok(())
    }

    fn render_areas(&self, physical_damage: Option<&[DamageRect]>) -> Result<Vec<PixelBounds>> {
        let Some(damage) = physical_damage else {
            return Ok(alloc::vec![self.full_bounds()]);
        };
        let mut areas = Vec::new();
        areas
            .try_reserve(damage.len())
            .map_err(|_| Error::FrameTooComplex)?;
        for rect in damage {
            if let Some((x, y, width, height)) = clamp_damage(*rect, self.width, self.height) {
                areas.push(PixelBounds {
                    x,
                    y,
                    width,
                    height,
                });
            }
        }
        if areas.is_empty() {
            return Err(Error::InvalidFrame);
        }
        Ok(areas)
    }

    const fn full_bounds(&self) -> PixelBounds {
        PixelBounds {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }

    /// Return the last rendered target image.
    ///
    /// # Returns
    ///
    /// The target selected by the previous render call, or `None` before the
    /// first render.
    pub fn image(&self) -> Option<&sgfx::wgpu::Image> {
        self.last_slot.and_then(|slot| self.session.image(slot))
    }

    /// Return the physical target dimensions.
    ///
    /// # Returns
    ///
    /// `(width, height)` in pixels.
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// WGPU paint backend that executes SGFX IR and presents through a WGPU
/// surface.
///
/// This is the platform-facing layer above [`WgpuSgfxSession`]. SGFX renders
/// into a persistent offscreen image, then this adapter performs one fullscreen
/// WGPU blit into the acquired surface frame. The IR lowering remains shared
/// with the native SWS backend, while surface ownership stays with the caller.
pub struct WgpuPaintBackend {
    surface: wgpu::Surface<'static>,
    /// Keep the WGPU instance alive for the lifetime of the surface.
    _instance: wgpu::Instance,
    config: wgpu::SurfaceConfiguration,
    session: WgpuSgfxSession,
    sampler: wgpu::Sampler,
    blit_pipeline: wgpu::RenderPipeline,
    supports_depth: bool,
    scale_milli: u32,
}

impl WgpuPaintBackend {
    /// Create a WGPU paint backend from an already-selected device and surface.
    ///
    /// The surface is configured here and must use a format supported by the
    /// WGPU adapter that created `device`. The caller retains ownership of the
    /// native window; the backend retains the WGPU instance and surface.
    ///
    /// # Arguments
    ///
    /// * `instance` - WGPU instance that created `surface`.
    /// * `surface` - Surface with a `'static` lifetime obtained from a native
    ///   window handle.
    /// * `device` - WGPU device used for SGFX and surface presentation.
    /// * `queue` - Queue paired with `device`.
    /// * `config` - Surface configuration, including format and present mode.
    /// * `width` - Initial physical width in pixels.
    /// * `height` - Initial physical height in pixels.
    /// * `supports_depth` - Whether `SgfxCanvas` depth passes may be used.
    ///
    /// # Returns
    ///
    /// A configured WGPU paint backend or an initialization error.
    pub fn new(
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
        let session = WgpuSgfxSession::new(device, queue, width, height, supports_depth)?;
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
            scale_milli: 1000,
        })
    }

    /// Borrow the SGFX/WGPU session for platform-specific inspection.
    ///
    /// # Returns
    ///
    /// The persistent session used by this paint backend.
    pub fn session(&self) -> &WgpuSgfxSession {
        &self.session
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
        let image = self.session.image().ok_or(Error::InvalidFrame)?;
        let frame = self
            .surface
            .get_current_texture()
            .map_err(|_| Error::sgfx(Stage::AcquireSurfaceFrame))?;
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
        let width = physical_dimension(size.width, scale_milli);
        let height = physical_dimension(size.height, scale_milli);
        self.resize_physical(width, height);
    }

    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        background_color: Color,
        _logical_damage: Option<&[Rect]>,
        physical_damage: Option<&[DamageRect]>,
    ) -> scarlet_ui_core::Result<BackendFrame<'a>> {
        self.session
            .render_with_damage(context, background_color, self.scale_milli, physical_damage)
            .map_err(|error| {
                eprintln!("[ScarletUI] SGFX/WGPU render failed: {error}");
                scarlet_ui_core::error::Error::RenderError
            })?;
        self.present().map_err(|error| {
            eprintln!("[ScarletUI] SGFX/WGPU present failed: {error}");
            scarlet_ui_core::error::Error::RenderError
        })?;
        Ok(BackendFrame::External)
    }
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

fn physical_dimension(value: f32, scale_milli: u32) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 1;
    }
    let logical = value as u32;
    if logical == 0 {
        return 1;
    }
    let scale = u64::from(scale_milli.max(1));
    ((u64::from(logical).saturating_mul(scale).saturating_add(999) / 1000)
        .min(u64::from(u32::MAX))
        .max(1)) as u32
}

fn clamp_damage(damage: DamageRect, frame_width: u32, frame_height: u32) -> Option<DamageRect> {
    let (x, y, width, height) = damage;
    if width == 0 || height == 0 || x >= frame_width || y >= frame_height {
        return None;
    }
    let right = x.saturating_add(width).min(frame_width);
    let bottom = y.saturating_add(height).min(frame_height);
    if right <= x || bottom <= y {
        None
    } else {
        Some((x, y, right - x, bottom - y))
    }
}

fn select_render_slot(
    physical_damage: Option<&[DamageRect]>,
    next_slot: usize,
    last_slot: Option<usize>,
    render_areas: &[PixelBounds],
    full_bounds: PixelBounds,
) -> Result<usize> {
    if physical_damage.is_none() {
        return Ok(next_slot);
    }
    match last_slot {
        Some(front_slot) => Ok(front_slot),
        None if render_areas.len() == 1 && render_areas[0] == full_bounds => Ok(next_slot),
        None => Err(Error::InvalidFrame),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui_core::geometry::Rect;

    const FULL: PixelBounds = PixelBounds {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    };

    #[test]
    fn partial_damage_reuses_the_previous_front_slot() {
        let damage = [(100, 100, 20, 20)];
        let area = [PixelBounds {
            x: 100,
            y: 100,
            width: 20,
            height: 20,
        }];
        assert_eq!(
            select_render_slot(Some(&damage), 1, Some(0), &area, FULL).unwrap(),
            0
        );
    }

    #[test]
    fn first_full_damage_frame_uses_the_next_slot() {
        let damage = [(0, 0, 800, 600)];
        assert_eq!(
            select_render_slot(Some(&damage), 0, None, &[FULL], FULL).unwrap(),
            0
        );
    }

    #[test]
    fn partial_damage_without_a_front_frame_is_rejected() {
        let damage = [(100, 100, 20, 20)];
        let area = [PixelBounds {
            x: 100,
            y: 100,
            width: 20,
            height: 20,
        }];
        assert_eq!(
            select_render_slot(Some(&damage), 0, None, &area, FULL),
            Err(Error::InvalidFrame)
        );
    }

    #[test]
    fn full_repaint_uses_the_next_slot() {
        let area = [PixelBounds {
            x: 100,
            y: 100,
            width: 20,
            height: 20,
        }];
        assert_eq!(
            select_render_slot(None, 1, Some(0), &area, FULL).unwrap(),
            1
        );
    }

    #[test]
    fn partial_damage_preserves_pixels_outside_the_damage() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let Some(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            }))
        else {
            eprintln!("skipping partial-damage GPU test: no WGPU adapter");
            return;
        };
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
        else {
            eprintln!("skipping partial-damage GPU test: no WGPU device");
            return;
        };
        let mut session =
            WgpuSgfxSession::new(device, queue, 8, 8, false).expect("headless SGFX/WGPU session");
        let mut full = PaintContext::new();
        full.fill_rect(Rect::from_xywh(0.0, 0.0, 8.0, 8.0), Color::RED);
        session
            .render(&full, Color::WHITE, 1_000)
            .expect("initial full frame");

        let mut partial = PaintContext::new();
        partial.fill_rect(Rect::from_xywh(2.0, 2.0, 2.0, 2.0), Color::BLUE);
        session
            .render_with_damage(&partial, Color::WHITE, 1_000, Some(&[(2, 2, 2, 2)]))
            .expect("partial frame");

        let bytes_per_row = 256u32;
        let readback = session.raw_device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("scarlet-ui partial-damage readback"),
            size: u64::from(bytes_per_row) * 8,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            session
                .raw_device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("scarlet-ui partial-damage readback encoder"),
                });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: session.image().expect("rendered image").raw_texture(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(8),
                },
            },
            wgpu::Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
        );
        session
            .raw_queue()
            .submit(core::iter::once(encoder.finish()));
        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).expect("send map result");
        });
        let _ = session.raw_device().poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .expect("receive map result")
            .expect("map readback");
        let bytes = slice.get_mapped_range();
        let pixel = |x: usize, y: usize| {
            let offset = y * bytes_per_row as usize + x * 4;
            [
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]
        };
        assert_eq!(pixel(1, 1), [0, 0, 255, 255]);
        assert_eq!(pixel(2, 2), [255, 0, 0, 255]);
        assert_eq!(pixel(4, 4), [0, 0, 255, 255]);
    }
}
