//! WGPU adapter for the shared ScarletUI-to-SGFX IR lowering.

use alloc::rc::Rc;
use alloc::sync::Arc;
use alloc::vec::Vec;

use scarlet_ui_core::color::Color;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::renderer::PaintContext;
use sgfx::ir::{CommandBuffer, ResourceTable, TextureId};

use crate::error::{Error, Result, Stage};
use crate::geometry::PixelBounds;
use crate::lowering::{RenderBackend, RenderSession};

struct WgpuBackend;

impl RenderBackend for WgpuBackend {
    type Context = sgfx_backend_wgpu::Context;
    type Queue = sgfx_backend_wgpu::Queue;
    type Image = sgfx_backend_wgpu::Image;
    type ImageHandle = Arc<sgfx_backend_wgpu::Image>;
    type Resources = sgfx_backend_wgpu::Resources;

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
    context: sgfx_backend_wgpu::Context,
    queue: sgfx_backend_wgpu::Queue,
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
        let device = sgfx_backend_wgpu::Device::new(device, queue);
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
    pub fn image(&self) -> Option<&sgfx_backend_wgpu::Image> {
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
