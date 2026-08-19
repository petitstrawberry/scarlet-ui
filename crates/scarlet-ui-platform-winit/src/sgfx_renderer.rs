//! Winit-owned presentation policy for the selected SGFX backend.

use scarlet_ui_core::color::Color;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::geometry::{Rect, Size};
use scarlet_ui_core::renderer::{BackendFrame, PaintBackend, PaintContext};
use scarlet_ui_core::{Error, Result};
use scarlet_ui_renderer_sgfx::SgfxPaintEncoder;
use sgfx::{MappedTargetSession, WindowContext};

/// Platform composition of backend-owned SGFX targets and a Winit window.
pub(crate) struct SgfxWindowPaintBackend {
    // Drop the mapped session before the selected backend's window context.
    session: MappedTargetSession,
    window: WindowContext,
    encoder: SgfxPaintEncoder,
    supports_depth: bool,
    scale_milli: u32,
    next_slot: usize,
    last_slot: Option<usize>,
}

impl SgfxWindowPaintBackend {
    /// Compose SGFX mapped targets with a selected frontend window context.
    ///
    /// # Arguments
    ///
    /// * `window` - SGFX frontend context selected for the native window.
    /// * `width` - Initial physical width in pixels.
    /// * `height` - Initial physical height in pixels.
    /// # Returns
    ///
    /// A configured platform renderer, or a rendering error.
    pub(crate) fn new(window: WindowContext, width: u32, height: u32) -> Result<Self> {
        let width = width.max(1);
        let height = height.max(1);
        let supports_depth = window.supports_depth();
        let encoder =
            SgfxPaintEncoder::new(width, height, supports_depth).map_err(|_| Error::RenderError)?;
        let targets = [
            encoder.target_texture(0).ok_or(Error::RenderError)?,
            encoder.target_texture(1).ok_or(Error::RenderError)?,
        ];
        let session = window
            .create_mapped_target_session(encoder.resource_table(), &targets)
            .map_err(|_| Error::RenderError)?;
        Ok(Self {
            session,
            window,
            encoder,
            supports_depth,
            scale_milli: 1_000,
            next_slot: 0,
            last_slot: None,
        })
    }

    fn resize_physical(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.encoder.width() == width && self.encoder.height() == height {
            return;
        }
        let Ok(encoder) = SgfxPaintEncoder::new(width, height, self.supports_depth) else {
            return;
        };
        let (Some(first_target), Some(second_target)) =
            (encoder.target_texture(0), encoder.target_texture(1))
        else {
            return;
        };
        let targets = [first_target, second_target];
        let Ok(session) = self
            .window
            .create_mapped_target_session(encoder.resource_table(), &targets)
        else {
            return;
        };
        self.window.resize(width, height);
        self.encoder = encoder;
        self.session = session;
        self.next_slot = 0;
        self.last_slot = None;
    }

    fn present(&mut self, slot: usize) -> Result<()> {
        let target = self
            .encoder
            .target_texture(slot)
            .ok_or(Error::RenderError)?;
        self.window
            .present(&self.session, target)
            .map_err(|_| Error::RenderError)
    }

    fn render_areas(&self, physical_damage: Option<&[DamageRect]>) -> Result<Vec<DamageRect>> {
        let full = (0, 0, self.encoder.width(), self.encoder.height());
        let Some(damage) = physical_damage else {
            return Ok(vec![full]);
        };
        let mut areas = Vec::with_capacity(damage.len());
        for &damage in damage {
            if let Some(area) = clamp_damage(damage, self.encoder.width(), self.encoder.height()) {
                areas.push(area);
            }
        }
        if areas.is_empty() {
            Err(Error::RenderError)
        } else {
            Ok(areas)
        }
    }
}

impl PaintBackend for SgfxWindowPaintBackend {
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
        let render_areas = self.render_areas(physical_damage)?;
        let slot = select_render_slot(
            physical_damage.is_some(),
            self.next_slot,
            self.last_slot,
            &render_areas,
            (0, 0, self.encoder.width(), self.encoder.height()),
        )
        .ok_or(Error::RenderError)?;
        {
            let mut executor = self.session.executor();
            self.encoder
                .encode_frame(
                    &mut executor,
                    slot,
                    None,
                    context,
                    background_color,
                    self.scale_milli,
                    &render_areas,
                )
                .map_err(|error| {
                    eprintln!("[ScarletUI] SGFX render failed: {error}");
                    Error::RenderError
                })?;
        }
        self.present(slot).map_err(|error| {
            eprintln!("[ScarletUI] SGFX present failed: {error}");
            Error::RenderError
        })?;
        self.next_slot = (slot + 1) % 2;
        self.last_slot = Some(slot);
        Ok(BackendFrame::External)
    }
}

fn clamp_damage(damage: DamageRect, width: u32, height: u32) -> Option<DamageRect> {
    let (x, y, damage_width, damage_height) = damage;
    if damage_width == 0 || damage_height == 0 || x >= width || y >= height {
        return None;
    }
    let right = x.saturating_add(damage_width).min(width);
    let bottom = y.saturating_add(damage_height).min(height);
    (right > x && bottom > y).then_some((x, y, right - x, bottom - y))
}

fn select_render_slot(
    partial_damage: bool,
    next_slot: usize,
    last_slot: Option<usize>,
    render_areas: &[DamageRect],
    full_bounds: DamageRect,
) -> Option<usize> {
    if !partial_damage {
        return Some(next_slot);
    }
    last_slot.or_else(|| {
        (render_areas.len() == 1 && render_areas[0] == full_bounds).then_some(next_slot)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: DamageRect = (0, 0, 800, 600);

    #[test]
    fn partial_damage_reuses_the_presented_target() {
        assert_eq!(
            select_render_slot(true, 1, Some(0), &[(100, 100, 20, 20)], FULL),
            Some(0)
        );
    }

    #[test]
    fn initial_full_damage_uses_the_next_target() {
        assert_eq!(select_render_slot(true, 1, None, &[FULL], FULL), Some(1));
    }

    #[test]
    fn initial_partial_damage_requires_a_presented_target() {
        assert_eq!(
            select_render_slot(true, 0, None, &[(100, 100, 20, 20)], FULL),
            None
        );
    }

    #[test]
    fn full_repaint_rotates_between_targets() {
        assert_eq!(
            select_render_slot(false, 1, Some(0), &[FULL], FULL),
            Some(1)
        );
    }

    #[test]
    fn damage_is_clamped_to_the_render_target() {
        assert_eq!(
            clamp_damage((790, 590, 20, 20), 800, 600),
            Some((790, 590, 10, 10))
        );
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
